//! Saved-result policy and recording of fresh lookup attempts.

use super::{history_outcome, setup::LookupSetup};
use crate::{
    config::{Config, ProviderName},
    domain::{LanguagePair, LookupResult},
    history::HistoryStore,
    lookup::{LookupError, LookupRequest, validate_query},
};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy)]
pub enum LookupPolicy {
    PreferSaved,
    Fresh,
}

pub struct Completion {
    pub result: Result<LookupResult, LookupError>,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
pub struct Coordinator {
    history: Result<HistoryStore, String>,
    setup: LookupSetup,
}

impl Coordinator {
    pub fn new(
        config_path: Option<PathBuf>,
        provider_override: Option<ProviderName>,
        database: Option<PathBuf>,
    ) -> Self {
        let history = crate::config::history_path(config_path.as_deref(), database.as_deref())
            .map(HistoryStore::new)
            .map_err(|e| e.to_string());
        Self {
            history,
            setup: LookupSetup::new(config_path, provider_override),
        }
    }

    /// Supply explicit settings before executing a session (also useful for fixtures).
    pub fn with_config(mut self, config: Config) -> Self {
        self.setup = self.setup.with_config(config);
        self
    }

    pub fn history(&self) -> Result<&HistoryStore, &str> {
        self.history.as_ref().map_err(String::as_str)
    }

    /// Reuse an exact saved success when requested; otherwise record a fresh attempt.
    pub async fn lookup(
        &self,
        request: LookupRequest,
        policy: LookupPolicy,
        cancellation: watch::Receiver<bool>,
        progress: Option<mpsc::UnboundedSender<String>>,
    ) -> Completion {
        if let Err(error) = validate_request(&request) {
            return Completion {
                result: Err(error),
                warnings: vec![],
            };
        }
        let mut warnings = Vec::new();
        if matches!(policy, LookupPolicy::PreferSaved)
            && let Ok(store) = &self.history
        {
            match store.saved_result(request.clone()).await {
                Ok(Some(entry)) => {
                    return Completion {
                        result: Ok(entry
                            .result()
                            .expect("saved_result returns successful results")
                            .clone()),
                        warnings,
                    };
                }
                Ok(None) => {}
                Err(error) => warnings.push(error.to_string()),
            }
        }
        let mut completion = self.record_lookup(request, cancellation, progress).await;
        warnings.append(&mut completion.warnings);
        completion.warnings = warnings;
        completion
    }

    pub async fn record_lookup(
        &self,
        request: LookupRequest,
        mut cancellation: watch::Receiver<bool>,
        progress: Option<mpsc::UnboundedSender<String>>,
    ) -> Completion {
        if let Err(error) = validate_request(&request) {
            return Completion {
                result: Err(error),
                warnings: vec![],
            };
        }
        let mut warnings = vec![];
        let initial_provider = self.setup.known_provider().map(provider_name);
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
        let config = self.setup.config();
        let selected = config
            .as_ref()
            .ok()
            .map(|config| self.setup.selected_provider(config));
        let operation = async {
            self.setup
                .prepare(config?, progress.as_ref())
                .await?
                .service
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
                .finish(id, history_outcome(&result), selected.map(provider_name))
                .await
        {
            warnings.push(format!(
                "History outcome was not saved; the attempt remains unfinished: {e}"
            ));
        }
        Completion { result, warnings }
    }
}

fn validate_request(request: &LookupRequest) -> Result<(), LookupError> {
    validate_query(&request.query)?;
    if let (Some(from), Some(to)) = (request.from, request.to)
        && from == to
    {
        return Err(LookupError::UnsupportedPair(LanguagePair { from, to }));
    }
    Ok(())
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
