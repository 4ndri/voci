//! Shared control-character and bidi sanitization for output and stored diagnostics.

/// Prevent escape sequences, line injection, and bidi overrides in terminal output.
pub fn safe_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect()
}
