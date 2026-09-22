//! Pure formatting shared by terminal frontends and recorded diagnostics.

use crate::{domain::LookupResult, text::safe_text};
use chrono::{DateTime, Local};

pub fn display_time(timestamp: i64) -> String {
    DateTime::from_timestamp_micros(timestamp)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| "invalid timestamp".into())
}

pub fn values(result: &LookupResult, index: Option<usize>) -> Option<String> {
    let value = |c: &crate::domain::TranslationCandidate| {
        let prefix = safe_text(&c.prefix);
        if prefix.is_empty() {
            safe_text(&c.text)
        } else {
            format!("{} {}", prefix, safe_text(&c.text))
        }
    };
    match index {
        Some(i) => result.candidates.get(i).map(value),
        None => Some(
            result
                .candidates
                .iter()
                .map(value)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

/// Keep frontend advice and persisted diagnostic wording stable across adapters.
pub(crate) fn lookup_error_text(error: &crate::lookup::LookupError) -> String {
    use crate::lookup::LookupError;
    match error {
        LookupError::Ambiguous(_) => {
            format!("{error} Choose --from de or --from en (Source in the TUI).")
        }
        LookupError::Undetermined(_) => {
            format!("{error} Check spelling or specify --from de / --from en (Source in the TUI).")
        }
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    #[test]
    fn copies_plain_values_including_prefix_without_control_sequences() {
        let result = LookupResult {
            query: "word".into(),
            headword: "word".into(),
            normalized_headword: "word".into(),
            pair: INITIAL_PAIRS[0],
            candidates: vec![
                TranslationCandidate {
                    text: "hello\u{202e}".into(),
                    normalized: "hello".into(),
                    part_of_speech: None,
                    sense: Some("not copied".into()),
                    prefix: "the".into(),
                    back_translations: vec![],
                },
                TranslationCandidate {
                    text: "world".into(),
                    normalized: "world".into(),
                    part_of_speech: None,
                    sense: None,
                    prefix: String::new(),
                    back_translations: vec![],
                },
            ],
            provider: "fixture".into(),
            attribution: None,
            kind: ResultKind::Dictionary,
        };
        assert_eq!(values(&result, Some(0)).as_deref(), Some("the hello"));
        assert_eq!(values(&result, None).as_deref(), Some("the hello\nworld"));
        assert!(values(&result, Some(9)).is_none());
    }
}
