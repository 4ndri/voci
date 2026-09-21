//! Shared language and dictionary-result contracts.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    type Err = ParseLanguageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "de" => Ok(Self::German),
            "en" => Ok(Self::English),
            _ => Err(ParseLanguageError(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResultKind {
    Dictionary,
    MachineTranslation,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TranslationCandidate {
    pub text: String,
    pub normalized: String,
    pub part_of_speech: Option<String>,
    pub sense: Option<String>,
    pub prefix: String,
    pub back_translations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[error("Unsupported language '{0}'. Supported directions: de → en, en → de.")]
pub struct ParseLanguageError(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupRequest {
    pub query: String,
    pub from: Option<Language>,
    pub to: Option<Language>,
}
