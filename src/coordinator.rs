use crate::{
    app::LookupService,
    config::{Config, ProviderName},
    domain::*,
    history::{Finished, HistoryStore},
    provider::Provider,
    wikdict::WikDictProvider,
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, watch};

pub struct Completion {
    pub result: Result<LookupResult, LookupError>,
    pub warnings: Vec<String>,
}
#[derive(Clone)]
pub struct Coordinator {
    pub history: Result<HistoryStore, String>,
    pub config_path: Option<PathBuf>,
    pub provider_override: Option<ProviderName>,
    pub config: Option<Arc<Config>>,
}
impl Coordinator {
    pub fn new(config_path: Option<PathBuf>, provider_override: Option<ProviderName>) -> Self {
        Self {
            history: crate::history::default_path().map(HistoryStore::new),
            config_path,
            provider_override,
            config: None,
        }
    }
    pub async fn run(
        &self,
        request: LookupRequest,
        mut cancellation: watch::Receiver<bool>,
        progress: Option<mpsc::UnboundedSender<String>>,
    ) -> Completion {
        if let Err(error) = validate_query(&request.query).and_then(|_| {
            if let (Some(from), Some(to)) = (request.from, request.to)
                && from == to
            {
                return Err(LookupError::UnsupportedPair(LanguagePair { from, to }));
            }
            Ok(())
        }) {
            return Completion {
                result: Err(error),
                warnings: vec![],
            };
        }
        let mut warnings = vec![];
        let initial_provider = self
            .provider_override
            .or_else(|| self.config.as_ref().map(|c| c.provider))
            .map(provider_name);
        let attempt = match &self.history {
            Ok(store) => match store.start(request.clone(), initial_provider).await {
                Ok(id) => Some(id),
                Err(e) => {
                    warnings.push(format!("History was not saved: {e}"));
                    None
                }
            },
            Err(e) => {
                warnings.push(format!("History was not saved: {e}"));
                None
            }
        };
        let config = self
            .config
            .clone()
            .map(Ok)
            .unwrap_or_else(|| Config::load(self.config_path.as_deref()).map(Arc::new));
        let selected = config
            .as_ref()
            .ok()
            .map(|c| self.provider_override.unwrap_or(c.provider));
        let operation = async {
            let config = config?;
            let provider = match selected.unwrap() {
                ProviderName::Wikdict => {
                    let provider = WikDictProvider::new(config.wikdict_dir()?);
                    provider
                        .prepare(|message| {
                            if let Some(tx) = &progress {
                                let _ = tx.send(message.to_owned());
                            }
                        })
                        .await?;
                    Provider::WikDict(provider)
                }
                ProviderName::Microsoft => Provider::Microsoft(config.microsoft()?),
            };
            LookupService::new(provider, config.target_language)
                .lookup(request)
                .await
        };
        let result = tokio::select! {
            biased;
            _=cancelled(&mut cancellation)=>Err(LookupError::Cancelled),
            result=operation=>result,
        };
        if let (Ok(store), Some(id)) = (&self.history, attempt)
            && let Err(e) = store
                .finish(
                    id,
                    Finished::from_result(&result),
                    selected.map(provider_name),
                )
                .await
        {
            warnings.push(format!(
                "History outcome was not saved; the attempt remains unfinished: {e}"
            ));
        }
        Completion { result, warnings }
    }
}
fn provider_name(provider: ProviderName) -> String {
    match provider {
        ProviderName::Wikdict => "WikDict",
        ProviderName::Microsoft => "Microsoft",
    }
    .into()
}
async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}
