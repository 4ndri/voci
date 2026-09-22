//! Translate completed lookup attempts into the persisted history schema.

use crate::{
    domain::LookupResult,
    history::{AttemptOutcome, Finished},
    lookup::LookupError,
};

pub fn history_outcome(result: &Result<LookupResult, LookupError>) -> Finished {
    match result {
        Ok(result) => Finished {
            outcome: AttemptOutcome::Success,
            result: Some(result.clone()),
            resolved_pair: Some(result.pair),
            error_code: None,
            message: None,
        },
        Err(error) => {
            let (outcome, code) = match error {
                LookupError::NotFound { .. } => (AttemptOutcome::NotFound, "not_found"),
                LookupError::Ambiguous(_) => (AttemptOutcome::Ambiguous, "ambiguous"),
                LookupError::Undetermined(_) => (AttemptOutcome::Undetermined, "undetermined"),
                LookupError::Cancelled => (AttemptOutcome::Cancelled, "cancelled"),
                LookupError::Configuration(_) => (AttemptOutcome::Failure, "configuration"),
                LookupError::Dictionary(_) => (AttemptOutcome::Failure, "dictionary"),
                LookupError::DictionaryDownload(_) => {
                    (AttemptOutcome::Failure, "dictionary_download")
                }
                LookupError::Authentication => (AttemptOutcome::Failure, "authentication"),
                LookupError::RateLimited => (AttemptOutcome::Failure, "rate_limited"),
                LookupError::Network => (AttemptOutcome::Failure, "network"),
                LookupError::Timeout => (AttemptOutcome::Failure, "timeout"),
                LookupError::InvalidResponse => (AttemptOutcome::Failure, "invalid_response"),
                LookupError::ProviderRejected => (AttemptOutcome::Failure, "provider_rejected"),
                LookupError::ProviderUnavailable => {
                    (AttemptOutcome::Failure, "provider_unavailable")
                }
                _ => (AttemptOutcome::Failure, "invalid_request"),
            };
            Finished {
                outcome,
                result: None,
                resolved_pair: match error {
                    LookupError::NotFound { pair, .. } => Some(*pair),
                    _ => None,
                },
                error_code: Some(code.into()),
                message: Some(crate::text::safe_text(
                    &crate::presentation::lookup_error_text(error),
                )),
            }
        }
    }
}
