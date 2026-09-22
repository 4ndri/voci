use voci::lookup::{LookupError, LookupRequest};
#[path = "support/commands.rs"]
mod support;
use predicates::prelude::*;
use rusqlite::{Connection, functions::FunctionFlags};
use std::{path::Path, time::Duration};
use unicode_normalization::UnicodeNormalization;
use voci::{
    domain::*,
    lookup::DictionaryProvider,
    lookup::LookupService,
    lookup::providers::{RELEASE, WikDictProvider},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn fixture(path: &Path, german: bool) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE translation(lexentry, sense_num, sense, written_rep TEXT, trans_list, score, is_good, importance);").unwrap();
    let rows = if german {
        vec![
            (
                "Verbindlichkeit",
                "binding duty",
                "liability | obligation",
                200,
            ),
            ("Verbindlichkeit", "promise", "commitment", 100),
            ("Verbindlichkeit", "binding duty", "liability", 50),
            ("Gift", "harmful substance", "poison", 200),
            ("Grüße", "salutation", "greetings", 100),
            ("Straße", "road", "street", 100),
            ("Bank", "seat", "bench | bank", 200),
            ("Bank", "financial institution", "bank", 100),
        ]
    } else {
        vec![
            (
                "liability",
                "obligation",
                "Verbindlichkeit | Verpflichtung",
                200,
            ),
            ("gift", "present", "Geschenk", 100),
            ("shell", "cover", "Schale", 100),
            ("street", "road", "Straße", 100),
        ]
    };
    for (word, sense, translations, score) in rows {
        db.execute(
            "INSERT INTO translation VALUES(NULL, NULL, ?1, ?2, ?3, ?4, 1, 1)",
            rusqlite::params![sense, word, translations, score],
        )
        .unwrap();
    }
}

fn installed(root: &Path) {
    std::fs::create_dir_all(root.join(RELEASE)).unwrap();
    for (file, german) in [("de-en.sqlite3", true), ("en-de.sqlite3", false)] {
        fixture(&root.join(RELEASE).join(file), german);
    }
}

#[tokio::test]
async fn local_lookup_preserves_senses_order_unicode_and_attribution() {
    let root = tempfile::tempdir().unwrap();
    installed(root.path());
    let provider =
        WikDictProvider::with_download_base(root.path().into(), "http://127.0.0.1:1".into());
    provider
        .prepare(|_| panic!("installed dictionaries must not be downloaded"))
        .await
        .unwrap();
    let result = provider
        .lookup("verbindlichkeit", INITIAL_PAIRS[0])
        .await
        .unwrap();
    assert_eq!(result.headword, "Verbindlichkeit");
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        ["liability", "obligation", "commitment"]
    );
    assert_eq!(result.candidates[0].sense.as_deref(), Some("binding duty"));
    assert!(result.attribution.unwrap().contains("CC BY-SA 4.0"));
    for query in ["GRÜßE", "GRÜSSE", "GRÜẞE", "Gru\u{308}sse", "Gru\u{308}ße"] {
        let result = provider.lookup(query, INITIAL_PAIRS[0]).await.unwrap();
        assert_eq!(result.candidates[0].text, "greetings");
        assert_eq!(result.headword, "Grüße");
        assert_eq!(result.normalized_headword, "grüsse");
    }
    for query in ["Straße", "STRASSE", "STRAẞE"] {
        let result = provider.lookup(query, INITIAL_PAIRS[0]).await.unwrap();
        assert_eq!(result.candidates[0].text, "street");
        assert_eq!(result.normalized_headword, "strasse");
    }
    let bank = provider.lookup("Bank", INITIAL_PAIRS[0]).await.unwrap();
    assert_eq!(bank.candidates.len(), 3); // Same translation, genuinely different senses.
    let street = provider.lookup("ſtreet", INITIAL_PAIRS[1]).await.unwrap();
    assert_eq!(street.candidates[0].text, "Straße");
    assert_eq!(street.candidates[0].normalized, "strasse");
    let service = LookupService::new(provider, Language::English);
    assert!(matches!(
        service
            .lookup(LookupRequest {
                query: "Gift".into(),
                from: None,
                to: None
            })
            .await,
        Err(LookupError::Ambiguous(_))
    ));
    let result = service
        .lookup(LookupRequest {
            query: "liability".into(),
            from: None,
            to: None,
        })
        .await
        .unwrap();
    assert_eq!(result.pair, INITIAL_PAIRS[1]);
    assert!(matches!(
        service
            .lookup(LookupRequest {
                query: "missing".into(),
                from: Some(Language::German),
                to: None
            })
            .await,
        Err(LookupError::NotFound { .. })
    ));
}

#[tokio::test]
async fn downloads_validate_index_and_reuse_both_files_without_sending_a_query() {
    let server = MockServer::start().await;
    let fixtures = tempfile::tempdir().unwrap();
    for (file, german) in [("de-en.sqlite3", true), ("en-de.sqlite3", false)] {
        let source = fixtures.path().join(file);
        fixture(&source, german);
        Mock::given(method("GET"))
            .and(path(format!("/{RELEASE}/{file}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(std::fs::read(source).unwrap()))
            .expect(1)
            .mount(&server)
            .await;
    }
    let root = tempfile::tempdir().unwrap();
    let provider = WikDictProvider::with_download_base(root.path().into(), server.uri());
    provider.prepare(|_| {}).await.unwrap();
    provider
        .prepare(|_| panic!("must reuse completed files"))
        .await
        .unwrap();
    let result = provider.lookup("GRÜSSE", INITIAL_PAIRS[0]).await.unwrap();
    assert_eq!(result.candidates[0].text, "greetings");
    let db = Connection::open(root.path().join(RELEASE).join("de-en.sqlite3")).unwrap();
    let index: String = db
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='index' AND name='voci_lookup_v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index, "voci_lookup_v2");
    for request in server.received_requests().await.unwrap() {
        assert!(request.body.is_empty());
        assert!(request.url.query().is_none());
        assert!(!request.headers.contains_key("Ocp-Apim-Subscription-Key"));
    }
    assert_eq!(
        std::fs::read_dir(root.path().join(RELEASE))
            .unwrap()
            .count(),
        2
    );
}

#[tokio::test]
async fn existing_lowercase_indexes_are_upgraded_once_without_downloading() {
    let root = tempfile::tempdir().unwrap();
    installed(root.path());
    for file in ["de-en.sqlite3", "en-de.sqlite3"] {
        let db = Connection::open(root.path().join(RELEASE).join(file)).unwrap();
        // Reproduce the original application's persisted normalization exactly.
        db.create_scalar_function(
            "voci_fold",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(context
                    .get::<String>(0)?
                    .nfc()
                    .collect::<String>()
                    .to_lowercase()
                    .nfc()
                    .collect::<String>())
            },
        )
        .unwrap();
        db.execute(
            "CREATE INDEX voci_lookup ON translation(voci_fold(written_rep))",
            [],
        )
        .unwrap();
    }
    let provider =
        WikDictProvider::with_download_base(root.path().into(), "http://127.0.0.1:1".into());
    let other =
        WikDictProvider::with_download_base(root.path().into(), "http://127.0.0.1:1".into());
    let (first, second) = tokio::join!(
        provider.prepare(|_| panic!("migration must work offline")),
        other.prepare(|_| panic!("migration must work offline")),
    );
    first.unwrap();
    second.unwrap();
    for query in ["Straße", "STRASSE", "STRAẞE", "GRÜSSE", "Gru\u{308}sse"] {
        let result = provider.lookup(query, INITIAL_PAIRS[0]).await.unwrap();
        assert_eq!(result.candidates.len(), 1, "{query}");
    }
    let path = root.path().join(RELEASE).join("de-en.sqlite3");
    let before = std::fs::read(&path).unwrap();
    provider
        .prepare(|_| panic!("must reuse upgraded dictionaries"))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let db = Connection::open(path).unwrap();
    let indexes: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='index'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(indexes, ["voci_lookup_v2"]);
    let original: String = db
        .query_row(
            "SELECT written_rep FROM translation WHERE trans_list='street'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, "Straße");
}

#[tokio::test]
async fn failed_or_corrupt_download_never_installs_a_dictionary() {
    for response in [
        ResponseTemplate::new(503),
        ResponseTemplate::new(200).set_body_string("not a SQLite file"),
        ResponseTemplate::new(200).insert_header("Content-Length", "70000000"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let provider = WikDictProvider::with_download_base(root.path().into(), server.uri());
        assert!(provider.prepare(|_| {}).await.is_err());
        assert_eq!(
            std::fs::read_dir(root.path().join(RELEASE))
                .unwrap()
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn cancellation_leaves_no_partial_install() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let provider = WikDictProvider::with_download_base(root.path().into(), server.uri());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), provider.prepare(|_| {}))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(root.path().join(RELEASE))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn existing_corrupt_dictionary_is_not_treated_as_a_miss_or_overwritten() {
    let root = tempfile::tempdir().unwrap();
    installed(root.path());
    let path = root.path().join(RELEASE).join("de-en.sqlite3");
    std::fs::write(&path, "corrupt").unwrap();
    let provider = WikDictProvider::new(root.path().into());
    let error = provider
        .prepare(|_| panic!("do not silently overwrite existing files"))
        .await
        .unwrap_err();
    assert!(matches!(error, LookupError::Dictionary(_)));
    assert!(error.to_string().contains("remove it and retry"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "corrupt");
}

#[test]
fn cli_defaults_to_keyless_wikdict_and_supports_provider_overrides() {
    let root = tempfile::tempdir().unwrap();
    installed(root.path());
    let config = root.path().join("config.toml");
    let value = toml::toml! { [wikdict] data_dir = "placeholder" };
    let mut value = toml::Value::Table(value);
    value["wikdict"]["data_dir"] = toml::Value::String(root.path().to_str().unwrap().into());
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    let mut command = support::command();
    command
        .env_remove("VOCI_MICROSOFT_KEY")
        .env("VOCI_MICROSOFT_REGION", "irrelevant invalid region")
        .arg("--config")
        .arg(&config)
        .arg("Verbindlichkeit")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1. liability").and(predicate::str::contains("CC BY-SA 4.0")),
        )
        .stderr("");
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args(["--json", "Verbindlichkeit"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(output.stderr.is_empty());
    let result: voci::domain::LookupResult = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result.query, "Verbindlichkeit");
    assert_eq!(result.candidates[0].text, "liability");
    assert_eq!(
        result.attribution.as_deref(),
        Some(voci::lookup::providers::ATTRIBUTION)
    );
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args(["--json", "--from", "de", "absent"])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No entry found")
    );
    for (query, expected) in [("STRASSE", "street"), ("GRÜSSE", "greetings")] {
        support::command()
            .arg("--config")
            .arg(&config)
            .arg(query)
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("1. {expected}")))
            .stderr("");
    }
    let mut command = support::command();
    command
        .env_remove("VOCI_MICROSOFT_KEY")
        .arg("--config")
        .arg(&config)
        .args(["--provider", "microsoft", "word"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("VOCI_MICROSOFT_KEY"));
    value
        .as_table_mut()
        .unwrap()
        .insert("provider".into(), toml::Value::String("microsoft".into()));
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    let mut command = support::command();
    command
        .env_remove("VOCI_MICROSOFT_KEY")
        .arg("--config")
        .arg(&config)
        .args(["--provider", "wikdict", "--", "shell"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Schale"));
}

#[tokio::test]
async fn retry_keeps_completed_dictionary_and_fetches_only_missing_direction() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(RELEASE)).unwrap();
    let completed = root.path().join(RELEASE).join("de-en.sqlite3");
    fixture(&completed, true);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{RELEASE}/en-de.sqlite3")))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let provider = WikDictProvider::with_download_base(root.path().into(), server.uri());
    assert!(provider.prepare(|_| {}).await.is_err());
    // Preparing an existing dictionary can upgrade its derived index once.
    assert_eq!(
        provider
            .lookup("STRASSE", INITIAL_PAIRS[0])
            .await
            .unwrap()
            .candidates[0]
            .text,
        "street",
    );
    let before = std::fs::read(&completed).unwrap();
    server.verify().await;
    server.reset().await;
    let fixture_root = tempfile::tempdir().unwrap();
    let source = fixture_root.path().join("en-de.sqlite3");
    fixture(&source, false);
    Mock::given(method("GET"))
        .and(path(format!("/{RELEASE}/en-de.sqlite3")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(std::fs::read(source).unwrap()))
        .expect(1)
        .mount(&server)
        .await;
    provider.prepare(|_| {}).await.unwrap();
    assert_eq!(std::fs::read(completed).unwrap(), before);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_first_runs_install_complete_dictionaries() {
    let server = MockServer::start().await;
    let fixtures = tempfile::tempdir().unwrap();
    for (file, german) in [("de-en.sqlite3", true), ("en-de.sqlite3", false)] {
        let source = fixtures.path().join(file);
        fixture(&source, german);
        Mock::given(method("GET"))
            .and(path(format!("/{RELEASE}/{file}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(std::fs::read(source).unwrap())
                    .set_delay(Duration::from_millis(20)),
            )
            .mount(&server)
            .await;
    }
    let root = tempfile::tempdir().unwrap();
    let first = WikDictProvider::with_download_base(root.path().into(), server.uri());
    let second = WikDictProvider::with_download_base(root.path().into(), server.uri());
    let (a, b) = tokio::join!(first.prepare(|_| {}), second.prepare(|_| {}));
    a.unwrap();
    b.unwrap();
    assert_eq!(
        std::fs::read_dir(root.path().join(RELEASE))
            .unwrap()
            .count(),
        2
    );
    assert!(
        !first
            .lookup("liability", INITIAL_PAIRS[1])
            .await
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[tokio::test]
async fn coordinator_retries_failed_preparation_and_reuses_success_across_submissions() {
    use voci::{app::Coordinator, config::Config};
    let root = tempfile::tempdir().unwrap();
    installed(root.path());
    let reverse = root.path().join(RELEASE).join("en-de.sqlite3");
    std::fs::write(&reverse, b"broken dictionary").unwrap();
    let mut config = Config::from_sources(None, None, None).unwrap();
    config.wikdict_data_dir = Some(root.path().into());
    let mut coordinator = Coordinator::new(None, None, Some(root.path().join("history.db")));
    coordinator = coordinator.with_config(config);
    let (_cancel, receiver) = tokio::sync::watch::channel(false);
    let request = LookupRequest {
        query: "Verbindlichkeit".into(),
        from: Some(Language::German),
        to: None,
    };
    // Construction remains lazy; failed initialization must be retried.
    assert!(!root.path().join("history.db").exists());
    assert!(matches!(
        coordinator
            .record_lookup(request.clone(), receiver.clone(), None)
            .await
            .result,
        Err(LookupError::Dictionary(_))
    ));
    std::fs::remove_file(&reverse).unwrap();
    fixture(&reverse, false);
    assert!(
        coordinator
            .record_lookup(request.clone(), receiver.clone(), None)
            .await
            .result
            .is_ok()
    );
    // If each submission prepared both dictionaries again, this would fail.
    std::fs::write(&reverse, b"broken dictionary").unwrap();
    assert!(
        coordinator
            .clone()
            .record_lookup(request, receiver, None)
            .await
            .result
            .is_ok()
    );
    let entries = coordinator
        .history()
        .unwrap()
        .page(Default::default(), None, 20, false)
        .await
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|entry| entry.finished.is_some()));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.result().is_some())
            .count(),
        2
    );
}
