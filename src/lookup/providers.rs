//! Concrete dictionary sources and their statically dispatched selection.

mod microsoft;
mod wikdict;

pub use microsoft::MicrosoftProvider;
pub use wikdict::{ATTRIBUTION, DOWNLOAD_BASE, RELEASE, WikDictProvider};

use super::{DictionaryProvider, LookupError, ProviderCapabilities};
use crate::domain::{LanguagePair, LookupResult};

/// Static dispatch keeps provider selection small without introducing a plugin registry.
pub enum Provider {
    WikDict(WikDictProvider),
    Microsoft(MicrosoftProvider),
}

impl DictionaryProvider for Provider {
    fn capabilities(&self) -> ProviderCapabilities {
        match self {
            Self::WikDict(provider) => provider.capabilities(),
            Self::Microsoft(provider) => provider.capabilities(),
        }
    }

    async fn lookup(&self, query: &str, pair: LanguagePair) -> Result<LookupResult, LookupError> {
        match self {
            Self::WikDict(provider) => provider.lookup(query, pair).await,
            Self::Microsoft(provider) => provider.lookup(query, pair).await,
        }
    }
}
