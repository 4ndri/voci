use crate::{domain::*, provider::DictionaryProvider};
use std::time::Duration;

pub struct LookupService<P> {
    provider: P,
    preferred_target: Language,
    deadline: Duration,
}

impl<P: DictionaryProvider> LookupService<P> {
    pub fn new(provider: P, preferred_target: Language) -> Self {
        Self {
            provider,
            preferred_target,
            deadline: Duration::from_secs(10),
        }
    }

    pub async fn lookup(&self, request: LookupRequest) -> Result<LookupResult, LookupError> {
        let query = validate_query(&request.query)?;
        tokio::time::timeout(
            self.deadline,
            self.resolve(&query, request.from, request.to),
        )
        .await
        .map_err(|_| LookupError::Timeout)?
    }

    fn pair_for(&self, from: Language, to: Option<Language>) -> Result<LanguagePair, LookupError> {
        let pairs = self.provider.capabilities().dictionary_pairs;
        let preferred = LanguagePair {
            from,
            to: to.unwrap_or(self.preferred_target),
        };
        if pairs.contains(&preferred) {
            return Ok(preferred);
        }
        if to.is_none()
            && preferred.from == preferred.to
            && let Some(pair) = pairs
                .iter()
                .find(|pair| pair.from == from && pair.to != from)
        {
            return Ok(*pair);
        }
        Err(LookupError::UnsupportedPair(preferred))
    }

    async fn resolve(
        &self,
        query: &str,
        from: Option<Language>,
        to: Option<Language>,
    ) -> Result<LookupResult, LookupError> {
        let result = if let Some(from) = from {
            self.provider
                .lookup(query, self.pair_for(from, to)?)
                .await?
        } else {
            // This inference strategy is deliberately limited to the initial pair.
            let (de, en) = tokio::try_join!(
                self.provider.lookup(query, INITIAL_PAIRS[0]),
                self.provider.lookup(query, INITIAL_PAIRS[1]),
            )?;
            let result = match (de.candidates.is_empty(), en.candidates.is_empty()) {
                (false, true) => de,
                (true, false) => en,
                (false, false) => return Err(LookupError::Ambiguous(query.into())),
                (true, true) => return Err(LookupError::Undetermined(query.into())),
            };
            self.pair_for(result.pair.from, to)?;
            result
        };
        if result.candidates.is_empty() {
            Err(LookupError::NotFound {
                query: query.into(),
                pair: result.pair,
            })
        } else {
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderCapabilities;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Barrier;

    struct Probe {
        barrier: Barrier,
        active: Arc<AtomicUsize>,
        stall: bool,
    }

    struct Active(Arc<AtomicUsize>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl DictionaryProvider for Probe {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                dictionary_pairs: INITIAL_PAIRS.to_vec(),
                translation_pairs: vec![],
            }
        }

        async fn lookup(
            &self,
            query: &str,
            pair: LanguagePair,
        ) -> Result<LookupResult, LookupError> {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(Arc::clone(&self.active));
            if self.stall {
                std::future::pending::<()>().await;
            }
            self.barrier.wait().await;
            Ok(LookupResult {
                query: query.into(),
                headword: query.into(),
                normalized_headword: query.into(),
                pair,
                candidates: vec![],
                provider: "test".into(),
                attribution: None,
                kind: ResultKind::Dictionary,
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn both_directions_start_concurrently() {
        let active = Arc::new(AtomicUsize::new(0));
        let service = LookupService::new(
            Probe {
                barrier: Barrier::new(2),
                active: Arc::clone(&active),
                stall: false,
            },
            Language::English,
        );
        let result = service
            .lookup(LookupRequest {
                query: "word".into(),
                from: None,
                to: None,
            })
            .await;
        assert!(matches!(result, Err(LookupError::Undetermined(_))));
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn total_deadline_cancels_both_requests() {
        let active = Arc::new(AtomicUsize::new(0));
        let service = LookupService::new(
            Probe {
                barrier: Barrier::new(2),
                active: Arc::clone(&active),
                stall: true,
            },
            Language::English,
        );
        let start = tokio::time::Instant::now();
        let result = service
            .lookup(LookupRequest {
                query: "word".into(),
                from: None,
                to: None,
            })
            .await;
        assert!(matches!(result, Err(LookupError::Timeout)));
        assert_eq!(start.elapsed(), Duration::from_secs(10));
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
