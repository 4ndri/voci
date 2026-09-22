//! Contract consumed by the lookup service and implemented by dictionary sources.

use super::LookupError;
use crate::domain::{LanguagePair, LookupResult};
use std::future::Future;

pub struct ProviderCapabilities {
    pub dictionary_pairs: Vec<LanguagePair>,
    pub translation_pairs: Vec<LanguagePair>,
}

pub trait DictionaryProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    fn lookup(
        &self,
        query: &str,
        pair: LanguagePair,
    ) -> impl Future<Output = Result<LookupResult, LookupError>> + Send;
}
