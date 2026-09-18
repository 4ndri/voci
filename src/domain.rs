use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    German,
    English,
}

impl Language {
    pub const fn code(self) -> &'static str {
        match self {
            Self::German => "de",
            Self::English => "en",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Language {
    type Err = LookupError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "de" => Ok(Self::German),
            "en" => Ok(Self::English),
            _ => Err(LookupError::UnsupportedLanguage(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguagePair {
    pub from: Language,
    pub to: Language,
}

impl fmt::Display for LanguagePair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} → {}", self.from, self.to)
    }
}

pub const INITIAL_PAIRS: [LanguagePair; 2] = [
    LanguagePair {
        from: Language::German,
        to: Language::English,
    },
    LanguagePair {
        from: Language::English,
        to: Language::German,
    },
];

#[derive(Clone, Debug)]
pub struct LookupRequest {
    pub query: String,
    pub from: Option<Language>,
    pub to: Option<Language>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Dictionary,
    MachineTranslation,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TranslationCandidate {
    pub text: String,
    pub normalized: String,
    pub part_of_speech: Option<String>,
    pub sense: Option<String>,
    pub prefix: String,
    pub back_translations: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct LookupResult {
    pub query: String,
    pub headword: String,
    pub normalized_headword: String,
    pub pair: LanguagePair,
    pub candidates: Vec<TranslationCandidate>,
    pub provider: String,
    pub attribution: Option<String>,
    pub kind: ResultKind,
}

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("Unsupported language '{0}'. Supported directions: de → en, en → de.")]
    UnsupportedLanguage(String),
    #[error("Unsupported language pair {0}. Supported directions: de → en, en → de.")]
    UnsupportedPair(LanguagePair),
    #[error(
        "'{0}' has entries in both German and English. Choose --from de or --from en (Source in the TUI)."
    )]
    Ambiguous(String),
    #[error(
        "Could not determine the source of '{0}': neither dictionary returned an entry. Check spelling or specify --from de / --from en (Source in the TUI)."
    )]
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

impl LookupError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidInput(_) | Self::UnsupportedLanguage(_) | Self::UnsupportedPair(_) => 2,
            _ => 1,
        }
    }
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
