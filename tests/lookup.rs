mod support;

use serde_json::json;
use std::time::Duration;
use voci::{
    app::LookupService,
    domain::*,
    presentation::render_result,
    provider::{DictionaryProvider, MicrosoftProvider},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

fn request(from: Option<Language>, to: Option<Language>) -> LookupRequest {
    LookupRequest {
        query: "Verbindlichkeit".into(),
        from,
        to,
    }
}

async fn dictionary(server: &MockServer, from: &str, targets: &[&str]) {
    Mock::given(method("POST"))
        .and(query_param("from", from))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::entry("Verbindlichkeit", targets)),
        )
        .mount(server)
        .await;
}

fn provider(server: &MockServer) -> MicrosoftProvider {
    MicrosoftProvider::with_endpoint(
        "test-secret",
        Some("westeurope"),
        &format!("{}/dictionary/lookup", server.uri()),
    )
    .unwrap()
}

#[tokio::test]
async fn adapter_sends_correct_request_and_preserves_meanings() {
    let server = MockServer::start().await;
    let mut response = support::entry("Verbindlichkeit", &["liability", "liability", "obligation"]);
    // Exact duplicate is collapsed; a different dictionary distinction is retained.
    let mut different = response[0]["translations"][0].clone();
    different["backTranslations"] = json!([{"displayText": "Schuld"}]);
    response[0]["translations"]
        .as_array_mut()
        .unwrap()
        .push(different);
    Mock::given(method("POST"))
        .and(path("/dictionary/lookup"))
        .and(query_param("api-version", "3.0"))
        .and(query_param("from", "de"))
        .and(query_param("to", "en"))
        .and(header("Ocp-Apim-Subscription-Key", "test-secret"))
        .and(header("Ocp-Apim-Subscription-Region", "westeurope"))
        .and(body_json(json!([{"text": "Verbindlichkeit"}])))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(&server)
        .await;
    let result = provider(&server)
        .lookup("  Verbindlichkeit  ", INITIAL_PAIRS[0])
        .await
        .unwrap();
    assert_eq!(result.candidates.len(), 3);
    assert_eq!(result.candidates[1].text, "obligation");
    assert_eq!(result.candidates[2].back_translations, ["Schuld"]);
    assert_eq!(result.kind, ResultKind::Dictionary);
    assert_eq!(result.provider, "Microsoft Translator");
    let text = render_result(&result);
    assert!(text.starts_with("Verbindlichkeit · de → en\n\n1. liability"));
    assert!(text.contains("Schuld"));
}

#[tokio::test]
async fn automatic_lookup_reuses_the_only_matching_direction() {
    for (german, english, expected) in [
        (vec!["liability"], vec![], INITIAL_PAIRS[0]),
        (vec![], vec!["Verbindlichkeit"], INITIAL_PAIRS[1]),
    ] {
        let server = MockServer::start().await;
        dictionary(&server, "de", &german).await;
        dictionary(&server, "en", &english).await;
        let service = LookupService::new(provider(&server), Language::English);
        let result = service.lookup(request(None, None)).await.unwrap();
        assert_eq!(result.pair, expected);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn ambiguous_and_undetermined_are_distinct_and_target_does_not_guess_source() {
    for targets in [vec!["meaning"], vec![]] {
        let server = MockServer::start().await;
        dictionary(&server, "de", &targets).await;
        dictionary(&server, "en", &targets).await;
        let service = LookupService::new(provider(&server), Language::English);
        let error = service
            .lookup(request(None, Some(Language::English)))
            .await
            .unwrap_err();
        if targets.is_empty() {
            assert!(matches!(error, LookupError::Undetermined(_)));
        } else {
            assert!(matches!(error, LookupError::Ambiguous(_)));
        }
    }
}

#[tokio::test]
async fn explicit_lookup_and_preference_use_one_request() {
    for preferred in [Language::German, Language::English] {
        let server = MockServer::start().await;
        dictionary(&server, "de", &["liability"]).await;
        let service = LookupService::new(provider(&server), preferred);
        let result = service
            .lookup(request(Some(Language::German), None))
            .await
            .unwrap();
        assert_eq!(result.pair, INITIAL_PAIRS[0]);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(matches!(
            service
                .lookup(request(Some(Language::German), Some(Language::German)))
                .await,
            Err(LookupError::UnsupportedPair(_))
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn explicit_miss_is_not_found_and_automatic_target_is_never_substituted() {
    let server = MockServer::start().await;
    dictionary(&server, "de", &[]).await;
    dictionary(&server, "en", &["Verbindlichkeit"]).await;
    let service = LookupService::new(provider(&server), Language::English);
    assert!(matches!(
        service.lookup(request(Some(Language::German), None)).await,
        Err(LookupError::NotFound { .. })
    ));
    assert!(matches!(
        service.lookup(request(None, Some(Language::English))).await,
        Err(LookupError::UnsupportedPair(_))
    ));
}

#[tokio::test]
async fn partial_failure_never_becomes_language_evidence() {
    let server = MockServer::start().await;
    dictionary(&server, "de", &["liability"]).await;
    Mock::given(query_param("from", "en"))
        .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(20)))
        .mount(&server)
        .await;
    let service = LookupService::new(provider(&server), Language::English);
    assert!(matches!(
        service.lookup(request(None, None)).await,
        Err(LookupError::ProviderUnavailable)
    ));
}

#[tokio::test]
async fn maps_statuses_without_leaking_response_or_retrying() {
    for (status, expected) in [
        (401, "authentication"),
        (403, "authentication"),
        (429, "quota"),
        (503, "unavailable"),
        (400, "rejected"),
        (302, "unexpected"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string("test-secret private-response")
                    .insert_header("Location", format!("{}/redirect", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = provider(&server)
            .lookup("word", INITIAL_PAIRS[0])
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains(expected), "{message}");
        assert!(!message.contains("test-secret"));
        assert!(!message.contains("private-response"));
    }
}

#[tokio::test]
async fn empty_entries_are_valid_but_malformed_responses_are_not() {
    let server = MockServer::start().await;
    dictionary(&server, "de", &[]).await;
    assert!(
        provider(&server)
            .lookup("word", INITIAL_PAIRS[0])
            .await
            .unwrap()
            .candidates
            .is_empty()
    );
    for body in ["not-json", "[]", "[{}]", "{}"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        assert!(matches!(
            provider(&server).lookup("word", INITIAL_PAIRS[0]).await,
            Err(LookupError::InvalidResponse)
        ));
    }
}

#[tokio::test]
async fn connectivity_failure_is_not_a_dictionary_miss() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let provider = MicrosoftProvider::with_endpoint(
        "test",
        None,
        &format!("http://{address}/dictionary/lookup"),
    )
    .unwrap();
    assert!(matches!(
        provider.lookup("word", INITIAL_PAIRS[0]).await,
        Err(LookupError::Network)
    ));
}

#[tokio::test]
async fn rejects_invalid_input_without_network_requests() {
    let server = MockServer::start().await;
    let service = LookupService::new(provider(&server), Language::English);
    for query in [
        "".into(),
        "  ".into(),
        "a\nb".into(),
        "\x1b[31m".into(),
        "ä".repeat(101),
    ] {
        assert!(matches!(
            service
                .lookup(LookupRequest {
                    query,
                    from: None,
                    to: None
                })
                .await,
            Err(LookupError::InvalidInput(_))
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn safe_concise_rendering_keeps_complete_domain_result() {
    let server = MockServer::start().await;
    let targets = [
        "\x1b[31mone\n",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
    ];
    dictionary(&server, "de", &targets).await;
    let result = provider(&server)
        .lookup("word", INITIAL_PAIRS[0])
        .await
        .unwrap();
    let output = render_result(&result);
    assert!(!output.contains('\x1b'));
    assert!(output.contains("1 more candidates omitted"));
    assert!(!output.contains("9. nine"));
    assert_eq!(result.candidates.len(), 9);
}
