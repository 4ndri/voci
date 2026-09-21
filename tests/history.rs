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
async fn nushell_completion_decodes_open_quotes_and_quoted_database_paths() {
    match std::process::Command::new("nu").arg("--version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Skipping Nushell adapter check; nu is not installed");
            return;
        }
        result => assert!(result.unwrap().status.success()),
    }
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("saved history.db"));
    for query in [
        "Vergangenheit",
        "ice cream",
        "say \"hello\"",
        "Grüße",
        "cash$env.HOME",
        "star*",
    ] {
        let id = store.start(request(query), None).await.unwrap();
        store
            .finish(id, Finished::from_result(&Ok(result(query))), None)
            .await
            .unwrap();
    }
    let cases = [
        ("Ver", "Vergangenheit"),
        ("\"Ver", "Vergangenheit"),
        ("'Ver", "Vergangenheit"),
        ("`Ver", "Vergangenheit"),
        ("\"Ver\"", "Vergangenheit"),
        ("\"ice cr", "\"ice cream\""),
        ("\"say \\\"h", r#""say \"hello\"""#),
        ("Grü", "Grüße"),
        ("cash", "\"cash$env.HOME\""),
        ("star", "\"star*\""),
    ];
    let spans: Vec<_> = cases
        .iter()
        .map(|(prefix, _)| {
            vec![
                assert_cmd::cargo::cargo_bin!("voci")
                    .to_string_lossy()
                    .into_owned(),
                "--database".into(),
                serde_json::to_string(store.path().to_str().unwrap()).unwrap(),
                (*prefix).into(),
            ]
        })
        .collect();
    let output = std::process::Command::new("nu")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--no-config-file", "-c", "source assets/completions/voci.nu; $env.VOCI_TEST_SPANS | from json | each {|spans| do $env.config.completions.external.completer $spans } | to json"])
        .env("VOCI_TEST_SPANS", serde_json::to_string(&spans).unwrap())
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected: Vec<_> = cases
        .iter()
        .map(|(_, completed)| {
            serde_json::json!([
                {"value": completed, "description": "voci"}
            ])
        })
        .collect();
    assert_eq!(actual, serde_json::json!(expected));
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        6
    );
}

#[tokio::test]
async fn tab_completion_and_exact_cli_reuse_are_read_only_and_skip_provider_setup() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("saved.db");
    let config = home.path().join("config.toml");
    std::fs::write(
        &config,
        "provider='invalid'\n[history]\ndatabase='saved.db'\n",
    )
    .unwrap();
    let store = HistoryStore::new(database.clone());
    for query in ["Straße", "ice cream", "Straße"] {
        let id = store.start(request(query), None).await.unwrap();
        store
            .finish(id, Finished::from_result(&Ok(result(query))), None)
            .await
            .unwrap();
    }
    let failed = store.start(request("Straße"), None).await.unwrap();
    store
        .finish(
            failed,
            Finished::from_result(&Err(LookupError::Network)),
            None,
        )
        .await
        .unwrap();
    for (words, expected) in [
        (vec!["STRASS"], vec!["Straße"]),
        (vec!["ice"], vec!["ice cream"]),
        (vec!["value"], vec![]),
        (vec!["--from", "de", "str"], vec!["Straße"]),
        (vec!["--from", "en", "str"], vec![]),
        (vec!["--to", "de", "str"], vec![]),
        (vec!["--from", "d"], vec!["de"]),
    ] {
        let mut command = support::command();
        support::isolate(&mut command, home.path());
        let output = command
            .args(["--json", "__complete", "--", "--config"])
            .arg(&config)
            .args(words)
            .assert()
            .success()
            .get_output()
            .clone();
        assert!(output.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&output.stdout).unwrap(),
            expected
        );
    }
    // Completion also honors an explicit database even when config cannot be read.
    let output = support::command()
        .args([
            "--json",
            "__complete",
            "--",
            "--config",
            "missing.toml",
            "--database",
        ])
        .arg(&database)
        .arg("str")
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&output.stdout).unwrap(),
        ["Straße"]
    );
    for args in [vec!["STRASSE"], vec!["--json", "STRASSE"]] {
        let output = support::command()
            .arg("--config")
            .arg(&config)
            .args(&args)
            .assert()
            .success()
            .get_output()
            .clone();
        assert!(output.stderr.is_empty());
        if args.contains(&"--json") {
            let saved: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(saved["query"], "Straße");
            assert_eq!(saved["candidates"].as_array().unwrap().len(), 12);
        } else {
            assert!(String::from_utf8(output.stdout).unwrap().contains("Straße"));
        }
    }
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        4
    );
    for args in [
        vec!["--fresh", "Straße"],
        vec!["Stra"],
        vec!["--from", "en", "Straße"],
    ] {
        support::command()
            .arg("--config")
            .arg(&config)
            .args(args)
            .assert()
            .code(1);
    }
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        7
    );
}
#[tokio::test]
async fn lookup_suggestions_search_saved_successes_before_limiting_and_respect_languages() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    assert!(
        store
            .suggestions(request("word"), 5)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(!store.path().exists());
    for query in ["Verbindlichkeit", "Verbindung"] {
        let id = store.start(request(query), None).await.unwrap();
        store
            .finish(id, Finished::from_result(&Ok(result(query))), None)
            .await
            .unwrap();
    }
    for _ in 0..6 {
        let id = store.start(request("Verbindlichkeit"), None).await.unwrap();
        store
            .finish(id, Finished::from_result(&Err(LookupError::Network)), None)
            .await
            .unwrap();
    }
    store.start(request("Verbindlichkeit"), None).await.unwrap();
    let page = store.suggestions(request(" VERBIND "), 1).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].query, "Verbindung");
    assert!(page.has_more);
    for query in ["STRASSE", "100%_", "é"] {
        assert_eq!(
            store
                .suggestions(request(query), 5)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
    }
    for (from, to, count) in [
        (None, None, 2),
        (Some(Language::German), None, 2),
        (None, Some(Language::English), 2),
        (Some(Language::English), None, 0),
        (None, Some(Language::German), 0),
    ] {
        let page = store
            .suggestions(
                LookupRequest {
                    query: "verbind".into(),
                    from,
                    to,
                },
                5,
            )
            .await
            .unwrap();
        assert_eq!(page.entries.len(), count);
    }
    assert!(
        store
            .suggestions(request("  "), 5)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        9
    );
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
    let mut coordinator = Coordinator::new(
        None,
        Some(ProviderName::Microsoft),
        Some(store.path().into()),
    );
    coordinator.config = Some(Arc::new(config));
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
            predicate::str::contains("│ 12 │ value 11")
                .and(predicate::str::contains("Source: Attribution")),
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
    let mut coordinator = Coordinator::new(None, None, Some(blocked.join("voci.db")));
    coordinator.config = Some(Arc::new(config));
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let completion = coordinator.run(request("word"), rx, None).await;
    assert!(matches!(
        completion.result,
        Err(LookupError::Configuration(_))
    ));
    assert_eq!(completion.warnings.len(), 1);
    assert!(completion.warnings[0].contains("voci.db"));
}

#[tokio::test]
async fn misses_preserve_resolved_pairs_and_legacy_outcomes_remain_readable() {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(root.path().join("voci.db"));
    let request = LookupRequest {
        query: "missing".into(),
        from: Some(Language::German),
        to: None,
    };
    let id = store.start(request, None).await.unwrap();
    let outcome = Finished::from_result(&Err(LookupError::NotFound {
        query: "missing".into(),
        pair: INITIAL_PAIRS[0],
    }));
    let json = serde_json::to_string(&outcome).unwrap();
    let restored: Finished = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.resolved_pair, Some(INITIAL_PAIRS[0]));
    store.finish(id, restored, None).await.unwrap();
    let page = store
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap();
    assert_eq!(page.entries[0].from, Some(Language::German));
    assert_eq!(page.entries[0].to, Some(Language::English));
    assert!(page.entries[0].result().is_none());

    let mut legacy = serde_json::to_value(Finished::from_result(&Ok(result("legacy")))).unwrap();
    legacy.as_object_mut().unwrap().remove("resolved_pair");
    let restored: Finished = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.resolved_pair, None);
    let id = store
        .start(
            LookupRequest {
                query: "legacy".into(),
                from: None,
                to: None,
            },
            None,
        )
        .await
        .unwrap();
    store.finish(id, restored, None).await.unwrap();
    let page = store
        .page(HistoryFilter::default(), None, 20, false)
        .await
        .unwrap();
    assert_eq!(page.entries[0].from, Some(Language::German));
    assert_eq!(page.entries[0].to, Some(Language::English));
    let legacy_failure: Finished = serde_json::from_str(
        r#"{"outcome":"not_found","result":null,"error_code":"not_found","message":"missing"}"#,
    )
    .unwrap();
    assert_eq!(legacy_failure.resolved_pair, None);
}

#[tokio::test]
async fn shell_settings_and_saved_history_do_not_require_valid_provider_settings() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    for invalid in ["provider='unavailable'", "target_language='fr'"] {
        std::fs::write(
            &path,
            format!(
                "{invalid}\n[history]\ndatabase='history.db'\n[tui]\nkeybindings='custom.toml'\n"
            ),
        )
        .unwrap();
        let settings = voci::config::TuiConfig::load(Some(&path)).unwrap();
        assert_eq!(settings.keybindings, Some(PathBuf::from("custom.toml")));
        let coordinator = Coordinator::new(Some(path.clone()), None, None);
        let store = coordinator.history.as_ref().unwrap();
        let id = store.start(request("saved encounter"), None).await.unwrap();
        let (_cancel, receiver) = tokio::sync::watch::channel(false);
        let completion = coordinator.run(request("new lookup"), receiver, None).await;
        assert!(matches!(
            completion.result,
            Err(LookupError::Configuration(_))
        ));
        let page = store
            .page(HistoryFilter::default(), None, 20, false)
            .await
            .unwrap();
        assert!(page.entries.iter().any(|entry| entry.id == id));
        assert_eq!(
            page.entries[0]
                .finished
                .as_ref()
                .unwrap()
                .error_code
                .as_deref(),
            Some("configuration")
        );
    }
}

#[tokio::test]
async fn cli_json_history_preserves_results_statuses_and_pagination() {
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
    let failed = store.start(request("failed"), None).await.unwrap();
    store
        .finish(
            failed,
            Finished::from_result(&Err(LookupError::Network)),
            None,
        )
        .await
        .unwrap();
    let unfinished = store.start(request("unfinished"), None).await.unwrap();
    for args in [vec!["--json", "history"], vec!["history", "--json"]] {
        let mut command = support::command();
        support::isolate(&mut command, home.path());
        let output = command.args(args).assert().success().get_output().clone();
        assert!(output.stderr.is_empty());
        let page: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(page["has_more"], false);
        assert_eq!(page["entries"].as_array().unwrap().len(), 3);
        assert_eq!(page["entries"][0]["id"], unfinished);
        assert!(page["entries"][0]["finished"].is_null());
        assert_eq!(page["entries"][1]["finished"]["error_code"], "network");
        let saved = &page["entries"][2]["finished"]["result"];
        assert_eq!(saved["candidates"].as_array().unwrap().len(), 12);
        assert_eq!(saved["candidates"][0]["text"], "Straße 100%_ e\u{301}");
        assert_eq!(saved["attribution"], "Attribution");
    }
    for (args, count, more) in [
        (vec!["history", "--json", "--limit", "1"], 1, true),
        (vec!["search", "STRASSE", "--json"], 1, false),
        (vec!["search", "absent", "--json"], 0, false),
    ] {
        let mut command = support::command();
        support::isolate(&mut command, home.path());
        let output = command.args(args).assert().success().get_output().clone();
        let page: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(page["entries"].as_array().unwrap().len(), count);
        assert_eq!(page["has_more"], more);
    }
    let output = support::command()
        .args(["history", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();
    let page: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(page, serde_json::json!({"entries": [], "has_more": false}));
}

#[test]
fn history_tables_wrap_unicode_and_keep_outcomes_visible() {
    use unicode_width::UnicodeWidthStr;
    let mut lookup = result("request");
    lookup.candidates[0].text = "界 👩‍💻 e\u{301} ".repeat(30);
    lookup.candidates[0].sense = Some(format!("{}\x1b[31m", "longword".repeat(40)));
    let mut entry = HistoryEntry {
        id: "test".into(),
        sequence: 1,
        query: "request\x1b[31m".into(),
        from: Some(Language::German),
        to: Some(Language::English),
        provider: Some("fixture".into()),
        started_at: 0,
        finished_at: Some(1),
        finished: Some(Finished::from_result(&Ok(lookup))),
    };
    for width in [16, 32, 59, 60, 80, 100, 120] {
        let rendered = voci::presentation::render_history_at_width(&entry, width);
        assert!(
            rendered.lines().all(|line| line.width() == width),
            "width {width}"
        );
        assert!(!rendered.contains('\x1b'));
        assert!(rendered.contains("👩‍💻"));
        assert!(rendered.contains("e\u{301}"));
        assert!(rendered.contains("Source:"));
    }
    entry.finished = Some(Finished::from_result(&Err(LookupError::Network)));
    let rendered = voci::presentation::render_history(&entry);
    assert!(rendered.contains("failed"));
    assert!(rendered.contains("Cannot connect"));
    entry.finished = None;
    entry.finished_at = None;
    assert!(
        voci::presentation::render_history(&entry).contains("unfinished · outcome not recorded")
    );
}

#[test]
fn json_errors_keep_stdout_empty_and_exit_codes_intact() {
    for args in [
        vec!["--json", "shell"],
        vec!["--json", "--from", "de", "--to", "de", "word"],
    ] {
        let output = support::command()
            .args(args)
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], 2);
        assert!(error["error"]["message"].as_str().is_some());
    }
    let output = support::command()
        .args(["history", "--json", "--config", "missing.toml"])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], 1);
}

#[tokio::test]
async fn cli_history_defaults_to_twenty_and_all_keeps_filters() {
    let home = tempfile::tempdir().unwrap();
    let store = HistoryStore::new(db_path(home.path()));
    for index in 0..25 {
        store
            .start(request(&format!("saved {index:02}")), None)
            .await
            .unwrap();
    }
    store.start(request("other"), None).await.unwrap();

    for (args, count, more, first, last) in [
        (vec!["history"], 20, true, "other", "saved 06"),
        (vec!["history", "--all"], 26, false, "other", "saved 00"),
        (
            vec!["history", "--limit", "3"],
            3,
            true,
            "other",
            "saved 23",
        ),
        (vec!["search", "saved"], 20, true, "saved 24", "saved 05"),
        (
            vec!["search", "saved", "--all", "--today"],
            25,
            false,
            "saved 24",
            "saved 00",
        ),
    ] {
        let mut command = support::command();
        support::isolate(&mut command, home.path());
        let output = command
            .args(&args)
            .arg("--json")
            .assert()
            .success()
            .get_output()
            .clone();
        let page: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let entries = page["entries"].as_array().unwrap();
        assert_eq!(entries.len(), count);
        assert_eq!(page["has_more"], more);
        assert_eq!(entries.first().unwrap()["query"], first);
        assert_eq!(entries.last().unwrap()["query"], last);

        let mut command = support::command();
        support::isolate(&mut command, home.path());
        let output = command.args(&args).assert().success().get_output().clone();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.matches("Request:").count(), count);
    }

    for args in [
        vec!["history", "--all", "--limit", "10"],
        vec!["search", "saved", "--limit", "20", "--all"],
    ] {
        support::command().args(args).assert().code(2);
    }
    let output = support::command()
        .args(["history", "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"entries": [], "has_more": false})
    );
}
