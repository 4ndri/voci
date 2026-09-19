#[path = "support/commands.rs"]
mod support;
use predicates::prelude::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use voci::{
    config::{Config, ProviderName},
    coordinator::Coordinator,
    domain::*,
    history::*,
};

#[test]
fn bundled_sqlite_includes_wal_reset_fix() {
    // Concurrent WAL writers/checkpoints need the upstream fix introduced in 3.51.3.
    // https://www.sqlite.org/wal.html#walresetbug
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "SQLite {} predates the required WAL-reset fix; upgrade the bundled dependency",
        rusqlite::version()
    );
}

fn request(query: &str) -> LookupRequest {
    LookupRequest {
        query: query.into(),
        from: Some(Language::German),
        to: Some(Language::English),
    }
}
fn result(query: &str) -> LookupResult {
    LookupResult {
        query: query.into(),
        headword: query.into(),
        normalized_headword: fold(query),
        pair: INITIAL_PAIRS[0],
        candidates: (0..12)
            .map(|i| TranslationCandidate {
                text: if i == 0 {
                    "Straße 100%_ e\u{301}".into()
                } else {
                    format!("value {i}")
                },
                normalized: format!("value {i}"),
                part_of_speech: Some("noun".into()),
                sense: Some(format!("sense {i}")),
                prefix: String::new(),
                back_translations: vec!["back".into()],
            })
            .collect(),
        provider: "fixture".into(),
        attribution: Some("Attribution".into()),
        kind: ResultKind::Dictionary,
    }
}
fn db_path(home: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        home.join("voci/data/voci.db")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/voci/voci.db")
    } else {
        home.join("voci/voci.db")
    }
}
#[tokio::test]
async fn immutable_events_preserve_repeats_complete_results_and_unfinished_attempts() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    assert!(
        store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(!store.path().exists());
    let first = store.start(request("Verbindlichkeit"), None).await.unwrap();
    store
        .finish(
            first.clone(),
            Finished::from_result(&Ok(result("Verbindlichkeit"))),
            None,
        )
        .await
        .unwrap();
    let second = store.start(request("Verbindlichkeit"), None).await.unwrap();
    let page = store
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].id, second);
    assert!(page.entries[0].finished.is_none());
    let saved = page.entries[1].result().unwrap();
    assert_eq!(saved.candidates.len(), 12);
    assert_eq!(saved.candidates[10].sense.as_deref(), Some("sense 10"));
    assert_eq!(saved.attribution.as_deref(), Some("Attribution"));
    assert!(
        store
            .finish(
                first,
                Finished::from_result(&Err(LookupError::Network)),
                None
            )
            .await
            .is_err()
    );
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    assert!(
        connection
            .execute("UPDATE events SET query='changed'", [])
            .is_err()
    );
    assert!(connection.execute("DELETE FROM events", []).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}
#[tokio::test]
async fn literal_unicode_filters_apply_before_limit_and_keyset_pages_go_both_ways() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    for query in ["older", "Verbindlichkeit", "newer"] {
        let id = store.start(request(query), None).await.unwrap();
        store
            .finish(id, Finished::from_result(&Ok(result(query))), None)
            .await
            .unwrap();
    }
    for text in ["STRASSE", "É", "%_", "VERBINDLICH"] {
        let page = store
            .page(
                HistoryFilter {
                    text: text.into(),
                    today: true,
                },
                None,
                1,
                false,
            )
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 1, "{text}");
    }
    assert!(
        store
            .page(
                HistoryFilter {
                    text: "absent%".into(),
                    today: false
                },
                None,
                20,
                false
            )
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    let a = store
        .page(HistoryFilter::default(), None, 1, false)
        .await
        .unwrap();
    assert!(a.has_more);
    assert_eq!(a.entries[0].query, "newer");
    let b = store
        .page(
            HistoryFilter::default(),
            Some(a.entries[0].cursor()),
            1,
            false,
        )
        .await
        .unwrap();
    assert_eq!(b.entries[0].query, "Verbindlichkeit");
    let back = store
        .page(
            HistoryFilter::default(),
            Some(b.entries[0].cursor()),
            1,
            true,
        )
        .await
        .unwrap();
    assert_eq!(back.entries[0].query, "newer");
    let oldest = store
        .page(HistoryFilter::default(), None, 1, true)
        .await
        .unwrap();
    assert_eq!(oldest.entries[0].query, "older");
}
#[tokio::test]
async fn concurrent_writers_and_storage_failures_preserve_data() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let store = store.clone();
        tasks.spawn(async move {
            let id = store.start(request("repeat"), None).await?;
            store
                .finish(id, Finished::from_result(&Err(LookupError::Network)), None)
                .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap().unwrap();
    }
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap()
            .entries
            .len(),
        8
    );
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(store.start(request("locked"), None).await.is_err());
    connection.execute_batch("ROLLBACK").unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    assert!(
        store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .is_err()
    );
    let corrupt = root.path().join("corrupt.sqlite3");
    std::fs::write(&corrupt, b"original corrupt contents").unwrap();
    assert!(
        HistoryStore::new(corrupt.clone())
            .start(request("query"), None)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(corrupt).unwrap(),
        b"original corrupt contents"
    );
}
#[tokio::test]
async fn coordinator_records_setup_failures_and_cancellation_but_not_invalid_input() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    let config = Config::from_sources(Some("provider='microsoft'"), None, None).unwrap();
    let coordinator = Coordinator {
        history: Ok(store.clone()),
        config_path: None,
        provider_override: Some(ProviderName::Microsoft),
        config: Some(Arc::new(config)),
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let bad = coordinator.run(request(""), rx.clone(), None).await;
    assert!(matches!(bad.result, Err(LookupError::InvalidInput(_))));
    assert!(!store.path().exists());
    let failed = coordinator.run(request("word"), rx, None).await;
    assert!(matches!(failed.result, Err(LookupError::Configuration(_))));
    let (_tx, rx) = tokio::sync::watch::channel(true);
    let cancelled = coordinator.run(request("word"), rx, None).await;
    assert!(matches!(cancelled.result, Err(LookupError::Cancelled)));
    let entries = store
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].finished.as_ref().unwrap().outcome,
        AttemptOutcome::Cancelled
    );
    assert_eq!(
        entries[1].finished.as_ref().unwrap().error_code.as_deref(),
        Some("configuration")
    );
}
#[test]
fn cli_history_is_independent_of_config_and_provider_and_has_literal_reserved_words() {
    for args in [
        vec!["history"],
        vec!["--provider", "microsoft", "history"],
        vec!["search", "verbindlich", "--today", "--limit", "20"],
    ] {
        support::command()
            .args(args)
            .assert()
            .success()
            .stdout(predicate::str::contains("No saved lookups"));
    }
    support::command()
        .args(["--config", "absent.toml", "history"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot read"));
    for args in [
        vec!["history", "--limit", "0"],
        vec!["search", ""],
        vec!["history", "--limit", "-1"],
    ] {
        support::command().args(args).assert().code(2);
    }
    support::command()
        .args(["--provider", "microsoft", "shell"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("interactive terminal"));
    for word in ["history", "search", "shell"] {
        support::command()
            .args(["--provider", "microsoft", "--", word])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("VOCI_MICROSOFT_KEY"));
    }
}
#[tokio::test]
async fn configured_database_is_shared_by_lookup_and_history_without_touching_defaults() {
    let home = tempfile::tempdir().unwrap();
    let configs = home.path().join("settings");
    std::fs::create_dir(&configs).unwrap();
    let config = configs.join("config.toml");
    std::fs::write(
        &config,
        "provider='microsoft'\n[history]\ndatabase='data/custom.db'\n",
    )
    .unwrap();
    let custom = configs.join("data/custom.db");
    let mut lookup = support::command();
    support::isolate(&mut lookup, home.path());
    lookup
        .current_dir(home.path())
        .arg("--config")
        .arg(&config)
        .arg("Verbindlichkeit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("VOCI_MICROSOFT_KEY"));
    assert!(custom.exists());
    assert!(!db_path(home.path()).exists());
    // History only needs its own config section, not valid provider settings.
    std::fs::write(
        &config,
        "provider='unavailable'\ntarget_language='fr'\n[history]\ndatabase='data/custom.db'\n",
    )
    .unwrap();
    let mut history = support::command();
    support::isolate(&mut history, home.path());
    history
        .current_dir(home.path())
        .arg("--config")
        .arg(&config)
        .args(["search", "Verbindlichkeit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Verbindlichkeit"));
    let override_path = home.path().join("override.db");
    let mut lookup = support::command();
    support::isolate(&mut lookup, home.path());
    lookup
        .arg("--database")
        .arg(&override_path)
        .arg("--config")
        .arg(&config)
        .arg("other")
        .assert()
        .failure();
    assert!(override_path.exists());
    assert_eq!(
        HistoryStore::new(custom.clone())
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
    let mut history = support::command();
    support::isolate(&mut history, home.path());
    history
        .args(["history", "--config", "missing.toml", "--database"])
        .arg(&override_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("other"));
    assert!(!db_path(home.path()).exists());
    // Invalid database configuration must never silently fall back to the default.
    std::fs::write(&config, "[history]\ndatabase=''\n").unwrap();
    let mut history = support::command();
    support::isolate(&mut history, home.path());
    history
        .arg("--config")
        .arg(&config)
        .arg("history")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be empty"));
    assert!(!db_path(home.path()).exists());
}

#[tokio::test]
async fn legacy_database_is_preserved_and_can_be_selected_explicitly() {
    let home = tempfile::tempdir().unwrap();
    let legacy = db_path(home.path()).with_file_name("history.sqlite3");
    let store = HistoryStore::new(legacy.clone());
    store.start(request("legacy word"), None).await.unwrap();
    let mut default = support::command();
    support::isolate(&mut default, home.path());
    default
        .arg("history")
        .assert()
        .success()
        .stdout(predicate::str::contains("No saved lookups"));
    assert!(!db_path(home.path()).exists());
    let mut explicit = support::command();
    support::isolate(&mut explicit, home.path());
    explicit
        .arg("--database")
        .arg(&legacy)
        .arg("history")
        .assert()
        .success()
        .stdout(predicate::str::contains("legacy word"));
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn cli_prints_all_candidates_and_searches_existing_events() {
    let home = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(db_path(home.path()));
    let id = store.start(request("Verbindlichkeit"), None).await.unwrap();
    store
        .finish(
            id,
            Finished::from_result(&Ok(result("Verbindlichkeit"))),
            None,
        )
        .await
        .unwrap();
    let mut command = support::command();
    support::isolate(&mut command, home.path());
    command
        .args(["search", "VERBINDLICH"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("12. value 11").and(predicate::str::contains("Attribution")),
        );
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}
fn stalled_child(home: &Path) -> (std::process::Child, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("voci"));
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        command.env(key, home);
    }
    let child = command
        .args(["--provider", "microsoft", "--from", "de", "word"])
        .env("VOCI_MICROSOFT_KEY", "test-key")
        .env("HTTPS_PROXY", &proxy)
        .env("https_proxy", &proxy)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    (child, listener)
}
fn wait_for_start(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(c) =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            && c.query_row("SELECT count(*) FROM events WHERE phase=0", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
                == 1
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("start event not recorded");
}
#[tokio::test]
async fn hard_termination_leaves_unfinished_attempt() {
    let home = tempfile::tempdir().unwrap();
    let (mut child, _listener) = stalled_child(home.path());
    let path = db_path(home.path());
    wait_for_start(&path);
    child.kill().unwrap();
    child.wait().unwrap();
    let entries = HistoryStore::new(path)
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 1);
    assert!(entries[0].finished.is_none());
}
#[cfg(unix)]
#[tokio::test]
async fn sigint_records_cancellation_and_preserves_exit_status() {
    let home = tempfile::tempdir().unwrap();
    let (child, _listener) = stalled_child(home.path());
    let path = db_path(home.path());
    wait_for_start(&path);
    std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    let entries = HistoryStore::new(path)
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap()
        .entries;
    assert_eq!(
        entries[0].finished.as_ref().unwrap().outcome,
        AttemptOutcome::Cancelled
    );
}

#[tokio::test]
async fn equal_timestamps_use_event_order_and_terminal_outcomes_do_not_reorder_attempts() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    let id = store.start(request("real"), None).await.unwrap();
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let time = chrono::Utc::now().timestamp_micros();
    for id in ["a", "b", "c"] {
        connection
            .execute(
                "INSERT INTO events(attempt_id,phase,timestamp,query) VALUES (?1,0,?2,?1)",
                rusqlite::params![id, time],
            )
            .unwrap();
    }
    store
        .finish(id, Finished::from_result(&Err(LookupError::Network)), None)
        .await
        .unwrap();
    let first = store
        .page(HistoryFilter::default(), None, 2, false)
        .await
        .unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b"]
    );
    let next = store
        .page(
            HistoryFilter::default(),
            first.entries.last().map(HistoryEntry::cursor),
            2,
            false,
        )
        .await
        .unwrap();
    assert_eq!(next.entries[0].id, "a");
    assert_eq!(next.entries[1].query, "real");
}
#[tokio::test]
async fn lookup_failure_and_cancellation_remain_visible_when_storage_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let blocked = root.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let config = Config::from_sources(Some("provider='microsoft'"), None, None).unwrap();
    let coordinator = Coordinator {
        history: Ok(HistoryStore::new(blocked.join("voci.db"))),
        config_path: None,
        provider_override: None,
        config: Some(Arc::new(config)),
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let completion = coordinator.run(request("word"), rx, None).await;
    assert!(matches!(
        completion.result,
        Err(LookupError::Configuration(_))
    ));
    assert_eq!(completion.warnings.len(), 1);
    assert!(completion.warnings[0].contains("voci.db"));
}
