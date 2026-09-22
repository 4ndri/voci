//! Lazy provider construction shared by a session and its clones.

use crate::{
    config::{Config, ProviderName},
    lookup::{
        LookupError, LookupService,
        providers::{MicrosoftProvider, Provider, WikDictProvider},
    },
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{OnceCell, mpsc};

pub(super) struct Prepared {
    pub(super) config: Arc<Config>,
    pub(super) service: LookupService<Provider>,
}

#[derive(Clone)]
pub(super) struct LookupSetup {
    config_path: Option<PathBuf>,
    provider_override: Option<ProviderName>,
    config: Option<Arc<Config>>,
    prepared: Arc<OnceCell<Prepared>>,
}

impl LookupSetup {
    pub(super) fn new(
        config_path: Option<PathBuf>,
        provider_override: Option<ProviderName>,
    ) -> Self {
        Self {
            config_path,
            provider_override,
            config: None,
            prepared: Arc::new(OnceCell::new()),
        }
    }

    pub(super) fn with_config(mut self, config: Config) -> Self {
        self.config = Some(Arc::new(config));
        self.prepared = Arc::new(OnceCell::new());
        self
    }

    pub(super) fn known_provider(&self) -> Option<ProviderName> {
        self.provider_override
            .or_else(|| self.config.as_ref().map(|config| config.provider))
            .or_else(|| self.prepared.get().map(|prepared| prepared.config.provider))
    }

    pub(super) fn config(&self) -> Result<Arc<Config>, LookupError> {
        self.prepared
            .get()
            .map(|prepared| Arc::clone(&prepared.config))
            .or_else(|| self.config.clone())
            .map(Ok)
            .unwrap_or_else(|| {
                Config::load(self.config_path.as_deref())
                    .map(Arc::new)
                    .map_err(|error| LookupError::Configuration(error.0))
            })
    }

    pub(super) fn selected_provider(&self, config: &Config) -> ProviderName {
        self.provider_override.unwrap_or(config.provider)
    }

    pub(super) async fn prepare(
        &self,
        config: Arc<Config>,
        progress: Option<&mpsc::UnboundedSender<String>>,
    ) -> Result<&Prepared, LookupError> {
        // Failed or cancelled preparation leaves the shared cell empty for retry.
        self.prepared
            .get_or_try_init(|| async {
                let provider = match self.selected_provider(&config) {
                    ProviderName::Wikdict => {
                        let provider = WikDictProvider::new(
                            config
                                .wikdict_dir()
                                .map_err(|error| LookupError::Configuration(error.0))?,
                        );
                        provider
                            .prepare(|message| {
                                if let Some(tx) = progress {
                                    let _ = tx.send(message.to_owned());
                                }
                            })
                            .await?;
                        Provider::WikDict(provider)
                    }
                    ProviderName::Microsoft => Provider::Microsoft(microsoft(&config)?),
                };
                Ok(Prepared {
                    service: LookupService::new(provider, config.target_language),
                    config,
                })
            })
            .await
    }
}

fn microsoft(config: &Config) -> Result<MicrosoftProvider, LookupError> {
    let key = config.key.as_deref().ok_or_else(|| LookupError::Configuration("Set VOCI_MICROSOFT_KEY to use Microsoft, or select --provider wikdict for lookup without a key.".into()))?;
    if config.region.as_ref().is_some_and(|value| {
        value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        return Err(LookupError::Configuration(
            "Microsoft region must be a nonempty Azure region name such as 'westeurope'.".into(),
        ));
    }
    MicrosoftProvider::new(key, config.region.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_requires_nonempty_credentials_without_echoing_them() {
        for key in [None, Some(String::new())] {
            let config = Config::from_sources(None, key, None).unwrap();
            assert!(microsoft(&config).is_err());
        }
        let config =
            Config::from_sources(None, Some("secret".into()), Some("bad region".into())).unwrap();
        let error = microsoft(&config).err().unwrap();
        assert!(!error.to_string().contains("secret"));
    }
}
