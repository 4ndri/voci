//! Append-only encounter history and saved-result queries.

mod model;
mod store;

pub use model::{
    AttemptId, AttemptOutcome, Cursor, Finished, HistoryEntry, HistoryFilter, HistoryPage,
};
pub(crate) use store::default_path;
pub use store::{HistoryError, HistoryStore};
