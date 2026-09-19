//! Keep native clipboard ownership alive for the entire TUI session.
use crate::{domain::LookupResult, presentation::safe_text};

pub trait Clipboard {
    fn copy(&mut self, text: String) -> Result<(), String>;
}
#[derive(Default)]
pub struct DesktopClipboard {
    inner: Option<arboard::Clipboard>,
}
impl Clipboard for DesktopClipboard {
    fn copy(&mut self, text: String) -> Result<(), String> {
        if self.inner.is_none() {
            self.inner = Some(arboard::Clipboard::new().map_err(|_| {
                "Desktop clipboard unavailable in this terminal environment.".to_string()
            })?);
        }
        self.inner
            .as_mut()
            .unwrap()
            .set_text(text)
            .map_err(|_| "Cannot write to the desktop clipboard. Try again.".to_string())
    }
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
