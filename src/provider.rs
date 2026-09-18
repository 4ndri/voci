use std::{collections::HashSet, future::Future, time::Duration};

use reqwest::{
    Client, StatusCode, Url,
    header::{HeaderMap, HeaderValue},
};
use serde::Deserialize;

use crate::domain::*;
use crate::wikdict::WikDictProvider;

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

pub struct MicrosoftProvider {
    client: Client,
    endpoint: Url,
}

impl MicrosoftProvider {
    pub fn new(key: &str, region: Option<&str>) -> Result<Self, LookupError> {
        Self::with_endpoint(
            key,
            region,
            "https://api.cognitive.microsofttranslator.com/dictionary/lookup",
        )
    }

    // Dependency injection for local HTTP tests, not a user-configurable credential destination.
    #[doc(hidden)]
    pub fn with_endpoint(
        key: &str,
        region: Option<&str>,
        endpoint: &str,
    ) -> Result<Self, LookupError> {
        let mut headers = HeaderMap::new();
        if key.trim().is_empty() {
            return Err(LookupError::Configuration(
                "Set VOCI_MICROSOFT_KEY to your Azure Translator subscription key.".into(),
            ));
        }
        let mut key = HeaderValue::from_str(key).map_err(|_| {
            LookupError::Configuration("VOCI_MICROSOFT_KEY contains invalid characters.".into())
        })?;
        key.set_sensitive(true);
        headers.insert("Ocp-Apim-Subscription-Key", key);
        if let Some(region) = region {
            headers.insert(
                "Ocp-Apim-Subscription-Region",
                HeaderValue::from_str(region).map_err(|_| {
                    LookupError::Configuration(
                        "Microsoft region contains invalid characters.".into(),
                    )
                })?,
            );
        }
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| {
                LookupError::Configuration("Unable to initialize the HTTPS client.".into())
            })?;
        let endpoint = Url::parse(endpoint)
            .map_err(|_| LookupError::Configuration("Invalid provider endpoint.".into()))?;
        Ok(Self { client, endpoint })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    normalized_source: String,
    display_source: String,
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Translation {
    normalized_target: String,
    display_target: String,
    pos_tag: String,
    #[serde(default)]
    prefix_word: String,
    #[serde(default)]
    back_translations: Vec<BackTranslation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackTranslation {
    display_text: String,
}

impl DictionaryProvider for MicrosoftProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            dictionary_pairs: INITIAL_PAIRS.to_vec(),
            translation_pairs: vec![],
        }
    }

    async fn lookup(&self, query: &str, pair: LanguagePair) -> Result<LookupResult, LookupError> {
        let query = validate_query(query)?;
        if !INITIAL_PAIRS.contains(&pair) {
            return Err(LookupError::UnsupportedPair(pair));
        }
        let response = self
            .client
            .post(self.endpoint.clone())
            .query(&[
                ("api-version", "3.0"),
                ("from", pair.from.code()),
                ("to", pair.to.code()),
            ])
            .json(&serde_json::json!([{ "text": query }]))
            .send()
            .await
            .map_err(transport_error)?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(LookupError::Authentication);
            }
            StatusCode::TOO_MANY_REQUESTS => return Err(LookupError::RateLimited),
            status if status.is_server_error() => return Err(LookupError::ProviderUnavailable),
            status if status.is_redirection() => return Err(LookupError::InvalidResponse),
            _ => return Err(LookupError::ProviderRejected),
        }
        let mut entries: Vec<Entry> = response.json().await.map_err(|error| {
            if error.is_timeout() {
                LookupError::Timeout
            } else if error.is_decode() {
                LookupError::InvalidResponse
            } else {
                LookupError::Network
            }
        })?;
        if entries.len() != 1 {
            return Err(LookupError::InvalidResponse);
        }
        let entry = entries.remove(0);
        if entry.display_source.trim().is_empty() || entry.normalized_source.trim().is_empty() {
            return Err(LookupError::InvalidResponse);
        }
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for value in entry.translations {
            if value.display_target.trim().is_empty() || value.normalized_target.trim().is_empty() {
                return Err(LookupError::InvalidResponse);
            }
            let mut meanings: Vec<_> = value
                .back_translations
                .into_iter()
                .map(|v| v.display_text)
                .collect();
            meanings.sort();
            meanings.dedup();
            let candidate = TranslationCandidate {
                text: value.display_target,
                normalized: value.normalized_target,
                part_of_speech: (!value.pos_tag.is_empty()).then_some(value.pos_tag),
                sense: None,
                prefix: value.prefix_word,
                back_translations: meanings,
            };
            let identity = (
                candidate.normalized.clone(),
                candidate.part_of_speech.clone(),
                candidate.prefix.clone(),
                candidate.back_translations.clone(),
            );
            if seen.insert(identity) {
                candidates.push(candidate);
            }
        }
        Ok(LookupResult {
            query,
            headword: entry.display_source,
            normalized_headword: entry.normalized_source,
            pair,
            candidates,
            provider: "Microsoft Translator".into(),
            attribution: None,
            kind: ResultKind::Dictionary,
        })
    }
}

fn transport_error(error: reqwest::Error) -> LookupError {
    if error.is_timeout() {
        LookupError::Timeout
    } else {
        LookupError::Network
    }
}
