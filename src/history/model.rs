//! Saved encounters and history query contracts.

use crate::domain::{Language, LanguagePair, LookupResult};
use serde::{Deserialize, Serialize};

pub type AttemptId = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Success,
    NotFound,
    Ambiguous,
    Undetermined,
    Failure,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finished {
    pub outcome: AttemptOutcome,
    pub result: Option<LookupResult>,
    // Optional for compatibility with previously stored outcome events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_pair: Option<LanguagePair>,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

impl Finished {
    pub fn label(&self) -> &'static str {
        match self.outcome {
            AttemptOutcome::Success => "success",
            AttemptOutcome::NotFound => "not found",
            AttemptOutcome::Ambiguous => "ambiguous",
            AttemptOutcome::Undetermined => "undetermined",
            AttemptOutcome::Failure => "failed",
            AttemptOutcome::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryEntry {
    pub id: AttemptId,
    pub sequence: i64,
    pub query: String,
    pub from: Option<Language>,
    pub to: Option<Language>,
    pub provider: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub finished: Option<Finished>,
}

impl HistoryEntry {
    pub fn result(&self) -> Option<&LookupResult> {
        self.finished.as_ref().and_then(|f| f.result.as_ref())
    }

    pub fn status(&self) -> &'static str {
        self.finished
            .as_ref()
            .map_or("unfinished · outcome not recorded", Finished::label)
    }

    pub fn cursor(&self) -> Cursor {
        Cursor {
            timestamp: self.started_at,
            sequence: self.sequence,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub timestamp: i64,
    pub sequence: i64,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryFilter {
    pub text: String,
    pub today: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}
