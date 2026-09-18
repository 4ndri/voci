use serde_json::{Value, json};

pub fn entry(source: &str, targets: &[&str]) -> Value {
    json!([{
        "normalizedSource": source.to_lowercase(),
        "displaySource": source,
        "translations": targets.iter().map(|target| json!({
            "normalizedTarget": target.to_lowercase(),
            "displayTarget": target,
            "posTag": "NOUN",
            "confidence": 0.5,
            "prefixWord": "",
            "backTranslations": [{"displayText": source}]
        })).collect::<Vec<_>>()
    }])
}
