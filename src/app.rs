//! Application workflows composing lookup, history, and session setup.

mod lookup;
mod outcome;
mod setup;
#[cfg(test)]
mod tests;

pub use lookup::{Completion, Coordinator, LookupPolicy};
pub use outcome::history_outcome;
