use super::*;
use crate::{
    domain::{INITIAL_PAIRS, LookupResult, ResultKind, TranslationCandidate},
    history::HistoryFilter,
    lookup::{LookupError, LookupRequest},
};
use tokio::sync::watch;

#[tokio::test]
async fn reuse_skips_provider_setup_and_recording_while_fresh_records_an_attempt() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    std::fs::write(&config, "provider='invalid'").unwrap();
    let coordinator = Coordinator::new(Some(config), None, Some(root.path().join("history.db")));
    let store = coordinator.history().unwrap();
    let request = LookupRequest {
        query: "word".into(),
        from: None,
        to: None,
    };
    let saved = LookupResult {
        query: request.query.clone(),
        headword: "word".into(),
        normalized_headword: "word".into(),
        pair: INITIAL_PAIRS[0],
        candidates: vec![TranslationCandidate {
            text: "saved value".into(),
            normalized: "saved value".into(),
            part_of_speech: None,
            sense: None,
            prefix: String::new(),
            back_translations: vec![],
        }],
        provider: "fixture".into(),
        attribution: None,
        kind: ResultKind::Dictionary,
    };
    let id = store.start(request.clone(), None).await.unwrap();
    store
        .finish(id, history_outcome(&Ok(saved)), None)
        .await
        .unwrap();

    let (_cancel, receiver) = watch::channel(false);
    let reused = coordinator
        .lookup(
            request.clone(),
            LookupPolicy::PreferSaved,
            receiver.clone(),
            None,
        )
        .await;
    assert_eq!(reused.result.unwrap().candidates[0].text, "saved value");
    assert!(reused.warnings.is_empty());
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );

    // Validation applies to the application entry point, before reading or recording.
    let invalid = coordinator
        .lookup(
            LookupRequest {
                query: String::new(),
                ..request.clone()
            },
            LookupPolicy::PreferSaved,
            receiver.clone(),
            None,
        )
        .await;
    assert!(matches!(invalid.result, Err(LookupError::InvalidInput(_))));
    assert_eq!(
        store
            .page(HistoryFilter::default(), None, 50, false)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );

    let fresh = coordinator
        .lookup(request, LookupPolicy::Fresh, receiver, None)
        .await;
    assert!(matches!(fresh.result, Err(LookupError::Configuration(_))));
    let entries = store
        .page(HistoryFilter::default(), None, 50, false)
        .await
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].finished.as_ref().unwrap().error_code.as_deref(),
        Some("configuration")
    );
}
