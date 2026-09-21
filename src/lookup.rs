//! Dictionary lookup policy and provider contracts.

mod model;
mod provider;
pub mod providers;
mod service;

pub use crate::domain::LookupRequest;
pub use model::{LookupError, validate_query};
pub use provider::{DictionaryProvider, ProviderCapabilities};
pub use service::LookupService;
