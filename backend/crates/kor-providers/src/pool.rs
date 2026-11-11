//! Key pool and failover (Master Build Prompt §4.5).
//!
//! A [`Failover`] is a `Provider` made of members tried in order: several keys of one provider,
//! then fallback providers (each with its own model). On 429, 5xx or a timeout, before or as the
//! first stream event, the next member takes the same request, so the turn goes on without the
//! user seeing the error. A member that answered 429 rests for its `retry-after` (or
//! [`DEFAULT_COOLDOWN`]) and is tried last until then. Auth and request errors are returned at
//! once: another key would only hide a configuration problem.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use kor_core::{ProviderError, ProviderErrorKind};
use tokio_util::sync::CancellationToken;

use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, health};

pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

pub struct Member {
    pub provider: Arc<dyn Provider>,
    /// Model to request from this member instead of the caller's (fallback providers).
    pub model: Option<String>,
    /// Shown in health stats: `openai`, `openai#2`, `openrouter`.
    pub label: String,
}

impl std::fmt::Debug for Member {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Member")
            .field("label", &self.label)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Failover {
    members: Vec<Member>,
    resting_until: Mutex<Vec<Option<Instant>>>,
}

/// 429 / 5xx / timeout: the cases §4.5 switches on.
fn switches(e: &ProviderError) -> bool {
    e.is_retryable()
}

impl Failover {
    /// `members` must not be empty; the first one's manifest names the whole pool.
    pub fn new(members: Vec<Member>) -> Option<Self> {
        if members.is_empty() {
            return None;
        }
        let n = members.len();
        Some(Self {
            members,
            resting_until: Mutex::new(vec![None; n]),
        })
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Member indexes: rested members in their order, then resting ones.
    fn order(&self) -> Vec<usize> {
        let now = Instant::now();
        let rest = self
            .resting_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (ready, resting): (Vec<usize>, Vec<usize>) =
            (0..self.members.len()).partition(|i| rest[*i].is_none_or(|t| t <= now));
        ready.into_iter().chain(resting).collect()
    }

    fn rest(&self, i: usize, e: &ProviderError) {
        if e.kind == ProviderErrorKind::RateLimited {
            let d = e
                .retry_after
                .unwrap_or(DEFAULT_COOLDOWN)
                .min(Duration::from_secs(600));
            self.resting_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner)[i] = Some(Instant::now() + d);
        }
    }
}

#[async_trait]
impl Provider for Failover {
    fn manifest(&self) -> &ProviderManifest {
        self.members[0].provider.manifest()
    }

    /// Keys share one host: warming the first member warms the pool's connection.
    fn preconnect(&self) {
        self.members[0].provider.preconnect();
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut last = None;
        for i in self.order() {
            match self.members[i].provider.list_models().await {
                Ok(m) => return Ok(m),
                Err(e) if switches(&e) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| exhausted(self.manifest())))
    }

    async fn count_tokens(&self, req: &ChatRequest) -> Result<Option<u64>, ProviderError> {
        self.members[0].provider.count_tokens(req).await
    }

    async fn complete(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let mut last = None;
        for i in self.order() {
            let m = &self.members[i];
            let mut r = req.clone();
            if let Some(model) = &m.model {
                r.model.clone_from(model);
            }
            let started = Instant::now();
            let err = match m.provider.complete(r, cancel.clone()).await {
                Ok(mut stream) => match stream.next().await {
                    Some(Err(e)) if switches(&e) => e,
                    first => {
                        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        match &first {
                            Some(Err(e)) => note(&m.label, Err(&e.message)),
                            _ => note(&m.label, Ok(ms)),
                        }
                        return Ok(match first {
                            Some(item) => {
                                Box::pin(futures::stream::once(async { item }).chain(stream))
                            }
                            None => stream,
                        });
                    }
                },
                Err(e) => e,
            };
            note(&m.label, Err(&err.message));
            if !switches(&err) || cancel.is_cancelled() {
                return Err(err);
            }
            self.rest(i, &err);
            last = Some(err);
        }
        Err(last.unwrap_or_else(|| exhausted(self.manifest())))
    }
}

/// Every key of `m` in failover order: its env var, then keychain slots
/// `provider:<id>`, `provider:<id>#2`… Empty for keyless providers.
pub fn keys_for(
    m: &ProviderManifest,
    store: &dyn kor_secrets::SecretStore,
) -> Result<Vec<kor_secrets::SecretString>, kor_secrets::SecretError> {
    if !m.needs_key() {
        return Ok(Vec::new());
    }
    Ok(
        kor_secrets::resolve_keys(&m.id, &m.key_env(), |v| std::env::var(v).ok(), store)?
            .into_iter()
            .map(|k| k.secret)
            .collect(),
    )
}

/// An empty label (a nested pool) records its own health.
fn note(label: &str, outcome: Result<u64, &str>) {
    if !label.is_empty() {
        health::record(label, outcome);
    }
}

fn exhausted(m: &ProviderManifest) -> ProviderError {
    ProviderError::new(
        &m.id,
        ProviderErrorKind::Unavailable,
        "every key and fallback failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_core::{FinishReason, StreamEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        manifest: ProviderManifest,
        fail: Option<ProviderErrorKind>,
        calls: AtomicUsize,
        seen_model: Mutex<String>,
    }

    fn fake(fail: Option<ProviderErrorKind>) -> Arc<Fake> {
        Arc::new(Fake {
            manifest: crate::Registry::builtin().get("local").unwrap().clone(),
            fail,
            calls: AtomicUsize::new(0),
            seen_model: Mutex::new(String::new()),
        })
    }

    #[async_trait]
    impl Provider for Fake {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            req: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.seen_model.lock().unwrap() = req.model;
            match self.fail {
                Some(k) => Err(ProviderError::new("fake", k, "nope")),
                None => Ok(Box::pin(futures::stream::iter([Ok(StreamEvent::Finish {
                    reason: FinishReason::Stop,
                })]))),
            }
        }
    }

    fn member(p: &Arc<Fake>, model: Option<&str>) -> Member {
        Member {
            provider: p.clone(),
            model: model.map(str::to_owned),
            label: "pool-test".into(),
        }
    }

    #[tokio::test]
    async fn rate_limit_moves_to_next_member_and_rests() {
        let a = fake(Some(ProviderErrorKind::RateLimited));
        let b = fake(None);
        let pool = Failover::new(vec![member(&a, None), member(&b, Some("fb-model"))]).unwrap();
        let req = ChatRequest {
            model: "m".into(),
            ..ChatRequest::default()
        };
        for _ in 0..2 {
            let mut s = pool
                .complete(req.clone(), CancellationToken::new())
                .await
                .unwrap();
            assert!(matches!(
                s.next().await,
                Some(Ok(StreamEvent::Finish { .. }))
            ));
        }
        assert_eq!(*b.seen_model.lock().unwrap(), "fb-model");
        // Second turn: `a` was resting, so `b` went first and `a` was not called again.
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn auth_errors_do_not_fail_over() {
        let a = fake(Some(ProviderErrorKind::AuthFailed));
        let b = fake(None);
        let pool = Failover::new(vec![member(&a, None), member(&b, None)]).unwrap();
        let e = pool
            .complete(ChatRequest::default(), CancellationToken::new())
            .await
            .err()
            .unwrap();
        assert_eq!(e.kind, ProviderErrorKind::AuthFailed);
        assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn all_members_failing_returns_the_last_error() {
        let a = fake(Some(ProviderErrorKind::Unavailable));
        let b = fake(Some(ProviderErrorKind::Timeout));
        let pool = Failover::new(vec![member(&a, None), member(&b, None)]).unwrap();
        let e = pool
            .complete(ChatRequest::default(), CancellationToken::new())
            .await
            .err()
            .unwrap();
        assert_eq!(e.kind, ProviderErrorKind::Timeout);
        assert!(Failover::new(Vec::new()).is_none());
    }
}
