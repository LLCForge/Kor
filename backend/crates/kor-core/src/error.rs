use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Normalized provider failure classes. The router decides retry/fallback from these alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    RateLimited,
    ContextTooLong,
    ContentFiltered,
    AuthFailed,
    InvalidRequest,
    Unavailable,
    Timeout,
    Cancelled,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{provider}: {kind:?}: {message}")]
pub struct ProviderError {
    pub provider: String,
    pub kind: ProviderErrorKind,
    /// Already redacted, safe to show to the user.
    pub message: String,
    pub retry_after: Option<Duration>,
    pub http_status: Option<u16>,
}

impl ProviderError {
    pub fn new(provider: &str, kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            provider: provider.to_owned(),
            kind,
            message: message.into(),
            retry_after: None,
            http_status: None,
        }
    }

    /// Worth retrying against the same provider.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::RateLimited
                | ProviderErrorKind::Unavailable
                | ProviderErrorKind::Timeout
        )
    }

    /// Worth trying the next provider in a fallback chain. Auth and bad requests are
    /// not: they indicate user configuration problems that another provider would mask.
    pub fn allows_fallback(&self) -> bool {
        self.is_retryable()
            || matches!(
                self.kind,
                ProviderErrorKind::ContextTooLong | ProviderErrorKind::ContentFiltered
            )
    }

    /// Map an HTTP status to a kind. Adapters refine this with body inspection.
    pub fn kind_from_status(status: u16) -> ProviderErrorKind {
        match status {
            401 | 403 => ProviderErrorKind::AuthFailed,
            408 | 504 => ProviderErrorKind::Timeout,
            413 => ProviderErrorKind::ContextTooLong,
            429 => ProviderErrorKind::RateLimited,
            400..=499 => ProviderErrorKind::InvalidRequest,
            _ => ProviderErrorKind::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_policy() {
        let e = |k| ProviderError::new("x", k, "m");
        assert!(e(ProviderErrorKind::RateLimited).allows_fallback());
        assert!(e(ProviderErrorKind::ContentFiltered).allows_fallback());
        assert!(!e(ProviderErrorKind::ContentFiltered).is_retryable());
        assert!(!e(ProviderErrorKind::AuthFailed).allows_fallback());
        assert_eq!(
            ProviderError::kind_from_status(429),
            ProviderErrorKind::RateLimited
        );
        assert_eq!(
            ProviderError::kind_from_status(503),
            ProviderErrorKind::Unavailable
        );
        assert_eq!(
            ProviderError::kind_from_status(404),
            ProviderErrorKind::InvalidRequest
        );
    }
}
