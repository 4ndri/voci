//! Opt-in network test. Never executed by the normal test suite.
use std::time::Instant;
use voci::{
    domain::INITIAL_PAIRS,
    provider::{DictionaryProvider, MicrosoftProvider},
};

#[tokio::test]
#[ignore = "requires Azure credentials, network access, and consumes provider quota"]
async fn dictionary_feasibility() {
    let key = std::env::var("VOCI_MICROSOFT_KEY")
        .expect("Set VOCI_MICROSOFT_KEY before explicitly running this test");
    let region = std::env::var("VOCI_MICROSOFT_REGION").ok();
    let provider = MicrosoftProvider::new(&key, region.as_deref()).unwrap();
    for word in [
        "Verbindlichkeit",
        "liability",
        "Gift",
        "Verbindlichkeiten",
        "running",
        "zzzxqvnonword",
    ] {
        for pair in INITIAL_PAIRS {
            let start = Instant::now();
            let result = provider.lookup(word, pair).await.unwrap();
            eprintln!(
                "{word} ({pair}), {:?}: {:?}",
                start.elapsed(),
                result
                    .candidates
                    .iter()
                    .map(|candidate| &candidate.text)
                    .collect::<Vec<_>>()
            );
            if (word == "Verbindlichkeit" && pair == INITIAL_PAIRS[0])
                || (word == "liability" && pair == INITIAL_PAIRS[1])
            {
                assert!(
                    result.candidates.len() >= 2,
                    "Expected several useful candidates; review provider feasibility"
                );
            }
        }
    }
}
