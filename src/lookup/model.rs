//! Lookup validation and feature-specific failures.

use crate::domain::{LanguagePair, ParseLanguageError};

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("Lookup cancelled.")]
    Cancelled,
    #[error("{0}")]
    InvalidInput(String),
    #[error("Unsupported language '{0}'. Supported directions: de → en, en → de.")]
    UnsupportedLanguage(String),
    #[error("Unsupported language pair {0}. Supported directions: de → en, en → de.")]
    UnsupportedPair(LanguagePair),
    #[error("'{0}' has entries in both German and English.")]
    Ambiguous(String),
    #[error("Could not determine the source of '{0}': neither dictionary returned an entry.")]
    Undetermined(String),
    #[error("No entry found for '{query}' ({pair}). Check spelling or change the source language.")]
    NotFound { query: String, pair: LanguagePair },
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("WikDict dictionary error: {0}")]
    Dictionary(String),
    #[error(
        "WikDict download failed: {0}. Check your network and retry; completed dictionaries remain available."
    )]
    DictionaryDownload(String),
    #[error("Microsoft authentication failed. Check VOCI_MICROSOFT_KEY and the configured region.")]
    Authentication,
    #[error(
        "Microsoft's request quota or rate limit was reached. Retry later and check your Azure quota."
    )]
    RateLimited,
    #[error("Microsoft Dictionary Lookup is unavailable. Retry later.")]
    ProviderUnavailable,
    #[error("Cannot connect to Microsoft Dictionary Lookup. Check your network and retry.")]
    Network,
    #[error("Lookup timed out. Check your network and retry.")]
    Timeout,
    #[error("Microsoft returned an unexpected dictionary response. Retry later.")]
    InvalidResponse,
    #[error(
        "Microsoft rejected the lookup request. Check the query, languages, and provider setup."
    )]
    ProviderRejected,
}

pub fn validate_query(query: &str) -> Result<String, LookupError> {
    // Validate before trimming so embedded/pasted terminal controls cannot be hidden.
    if query.chars().any(char::is_control) {
        return Err(LookupError::InvalidInput(
            "The query must not contain control characters.".into(),
        ));
    }
    let query = query.trim();
    if query.is_empty() {
        return Err(LookupError::InvalidInput("Enter a word to look up.".into()));
    }
    if query.chars().count() > 100 {
        return Err(LookupError::InvalidInput(
            "The query must be at most 100 characters.".into(),
        ));
    }
    Ok(query.to_owned())
}

impl From<ParseLanguageError> for LookupError {
    fn from(error: ParseLanguageError) -> Self {
        Self::UnsupportedLanguage(error.0)
    }
}
