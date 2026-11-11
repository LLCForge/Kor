//! `kor-proxy`: a local OpenAI-, Anthropic- and Gemini-compatible gateway, the Rust port of
//! [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) (Master Build Prompt §4.3.3).
//!
//! NOTICE: derived from CLIProxyAPI, Copyright (c) 2025 Luis Pater and Router-For.ME, used under
//! the MIT License; the full text is in `LICENSE-CLIProxyAPI` next to this crate's manifest.
//!
//! What is ported (see `docs/decisions/0010-kor-proxy.md` for what is not, and why):
//! * [`config`]: CLIProxyAPI `config.yaml` (v8 and legacy layouts), from `<config>/kor/proxy.yaml`
//!   or any existing CLIProxyAPI file (`--config`).
//! * [`translate`]: the translator matrix, OpenAI Chat / OpenAI Responses / Claude Messages /
//!   Gemini on the client side, through Kor's neutral request and event stream.
//! * [`routing`]: API-key credentials with priority, round-robin / weighted / fill-first, 429 and
//!   transient cooldowns, retry rounds.
//! * [`usage`]: per credential and model request and token counters.
//! * [`oauth`]: subscription logins for Claude Code, Codex (callback and device code),
//!   Antigravity, Kimi (`.com` and `.ai`), Devin, and GitHub Copilot's device code. Tokens are
//!   written as CLIProxyAPI-compatible JSON auth files. Claude logins use the Claude Code TLS
//!   fingerprint, and saved Claude accounts answer chat requests for `claude*` models.
//! * `server`: the HTTP endpoints (`/v1/chat/completions`, `/v1/responses`, `/v1/messages`,
//!   `/v1beta/models/{m}:generateContent`, `/v1/models`, `/v0/management/…`, `/management.html`).
//!
//! Requests that no configured credential serves fall back to Kor's own providers (`provider/model`
//! with the keychain key pool), so local models such as Ollama are servable too.

pub mod config;
pub mod oauth;
pub mod routing;
mod server;
pub mod translate;
pub mod usage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use kor_core::{ModelRef, ProviderError, ProviderErrorKind, StreamEvent};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, Provider, ProviderManifest, Registry,
};
use kor_secrets::{ExposeSecret, SecretStore, SecretString};
use serde_json::{Value, json};

pub use server::{Running, start};

/// Keychain account of the proxy access key (`kor proxy key`).
pub const KEY_ACCOUNT: &str = "proxy:access";
const MODELS_TTL: Duration = Duration::from_secs(600);
const LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// A request failure with the HTTP status the client gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<u64>,
}

impl Fail {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after: None,
        }
    }
}

impl From<&ProviderError> for Fail {
    fn from(e: &ProviderError) -> Self {
        Self {
            status: translate::status_of(e),
            message: format!("{}: {}", e.provider, e.message),
            retry_after: e.retry_after.map(|d| d.as_secs().max(1)),
        }
    }
}

type KeysFn = dyn Fn(&ProviderManifest) -> Vec<SecretString> + Send + Sync;
type LocalFn = dyn Fn(&ProviderManifest) -> bool + Send + Sync;

/// Where Kor's own providers come from. [`Sources::user`] is the real thing; tests build one.
pub struct Sources {
    pub registry: Registry,
    pub keys: Arc<KeysFn>,
    pub is_local: Arc<LocalFn>,
    /// Privacy mode: only local providers and credentials with a loopback base URL.
    pub privacy: bool,
    /// Also record tokens in Kor's shared usage ledger (`kor cost`).
    pub ledger: bool,
    /// List Kor registry models on `/v1/models` (network calls to configured providers).
    pub list_registry: bool,
}

impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sources")
            .field("privacy", &self.privacy)
            .finish_non_exhaustive()
    }
}

impl Sources {
    /// Kor's registry (built-ins + user manifests) and keychain key pool.
    pub fn user(
        l: &kor_i18n::Localizer,
        privacy: bool,
        notes: &mut Vec<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            registry: kor_runtime::registry(l, notes)?,
            keys: Arc::new(|m: &ProviderManifest| {
                kor_providers::pool::keys_for(m, &kor_secrets::KeychainStore).unwrap_or_default()
            }),
            is_local: Arc::new(kor_runtime::is_local_provider),
            privacy,
            ledger: true,
            list_registry: true,
        })
    }
}

/// Default config file: `<config>/kor/proxy.yaml`.
pub fn default_config_path() -> Option<PathBuf> {
    kor_config::user_dir().map(|d| d.join("proxy.yaml"))
}

/// How the server authenticates clients.
#[derive(Clone, Default)]
pub struct Access {
    /// Kor's generated key from the keychain.
    pub key: Option<SecretString>,
    /// `--no-auth`: only honoured on a loopback bind.
    pub no_auth: bool,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access")
            .field("no_auth", &self.no_auth)
            .finish_non_exhaustive()
    }
}

struct ModelCache {
    at: Instant,
    models: Vec<(String, String)>,
}

/// The gateway: config generation, credentials, Kor providers and counters.
pub struct Gateway {
    pool: RwLock<Arc<routing::Pool>>,
    config_path: Option<PathBuf>,
    sources: Sources,
    kor: Mutex<HashMap<String, Arc<dyn Provider>>>,
    models: tokio::sync::Mutex<Option<ModelCache>>,
    /// Shared with in-flight streams, which may outlive their request handler.
    pub stats: Arc<usage::Stats>,
    pub access: Access,
    /// Problems found while loading (bad groups, ignored keys); shown by `kor proxy serve`.
    pub warnings: Vec<String>,
    page: String,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("config", &self.config_path)
            .finish_non_exhaustive()
    }
}

fn pool_of(mut settings: config::Settings, sources: &Sources) -> (routing::Pool, Vec<String>) {
    let local = sources.is_local.clone();
    let (mut creds, mut errors) =
        routing::credentials(&settings, &sources.registry, &move |m| local(m));
    let dir = settings
        .auth_dir
        .clone()
        .unwrap_or_else(oauth::default_auth_dir);
    errors.extend(oauth::load_subscriptions(&dir, &mut settings, &mut creds));
    (routing::Pool::new(settings, creds), errors)
}

impl Gateway {
    pub fn new(
        settings: config::Settings,
        config_path: Option<PathBuf>,
        sources: Sources,
        access: Access,
        l: &kor_i18n::Localizer,
    ) -> Self {
        let mut warnings: Vec<String> = settings
            .ignored
            .iter()
            .map(|k| format!("config `{k}` is not supported by kor-proxy; ignored"))
            .collect();
        let (pool, errors) = pool_of(settings, &sources);
        warnings.extend(errors);
        Self {
            pool: RwLock::new(Arc::new(pool)),
            config_path,
            sources,
            kor: Mutex::new(HashMap::new()),
            models: tokio::sync::Mutex::new(None),
            stats: Arc::default(),
            access,
            warnings,
            page: server::page(l),
        }
    }

    pub fn settings(&self) -> config::Settings {
        self.pool().settings.clone()
    }

    fn pool(&self) -> Arc<routing::Pool> {
        self.pool
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// Swap in a new config generation (`PUT /v0/management/config.yaml`). Cooldowns restart.
    pub fn reload(&self, settings: config::Settings) -> Vec<String> {
        let (pool, errors) = pool_of(settings, &self.sources);
        *self.pool.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(pool);
        errors
    }

    pub fn credentials(&self) -> Vec<routing::CredentialView> {
        self.pool()
            .creds
            .iter()
            .map(routing::Credential::view)
            .collect()
    }

    /// Run one request; the stream is already counted in [`Gateway::stats`].
    pub async fn complete(
        &self,
        req: ChatRequest,
        client_model: &str,
    ) -> Result<EventStream, Fail> {
        let pool = self.pool();
        if let Some(target) = pool.settings.aliases.get(client_model) {
            return self.via_kor(target, req, client_model).await;
        }
        if !pool.order(client_model, self.sources.privacy).is_empty() {
            return self.via_pool(&pool, req, client_model).await;
        }
        if client_model.contains('/') {
            return self.via_kor(client_model, req, client_model).await;
        }
        Err(Fail::new(
            404,
            format!("model `{client_model}` is not served by this proxy"),
        ))
    }

    async fn via_pool(
        &self,
        pool: &routing::Pool,
        req: ChatRequest,
        client_model: &str,
    ) -> Result<EventStream, Fail> {
        let mut last: Option<ProviderError> = None;
        for round in 0..=pool.settings.request_retry {
            let attempts = pool.order(client_model, self.sources.privacy);
            if round > 0 {
                let wait = pool.next_ready(&attempts);
                if wait > pool.settings.max_retry_interval {
                    break;
                }
                tokio::time::sleep(wait).await;
            }
            let now_ready = pool.next_ready(&attempts).is_zero();
            for a in attempts {
                let c = &pool.creds[a.cred];
                // Cooling keys are skipped while another key is ready; a later round waits for them.
                if now_ready && !pool.next_ready(std::slice::from_ref(&a)).is_zero() {
                    continue;
                }
                let mut r = req.clone();
                r.model.clone_from(&a.model);
                match first_event(c.provider.as_ref(), r).await {
                    Ok(stream) => {
                        pool.succeeded(a.cred);
                        self.stats.request(&c.label, &a.model, true, false);
                        return Ok(self.counted(stream, c.label.clone(), a.model));
                    }
                    Err(e) => {
                        pool.failed(a.cred, &e);
                        self.stats.request(
                            &c.label,
                            &a.model,
                            false,
                            e.kind == ProviderErrorKind::RateLimited,
                        );
                        let next_key_may_work =
                            e.is_retryable() || e.kind == ProviderErrorKind::AuthFailed;
                        if !next_key_may_work {
                            return Err(Fail::from(&e));
                        }
                        last = Some(e);
                    }
                }
            }
            // Another round only helps when keys are cooling down, not when they were all refused.
            if last.as_ref().is_some_and(|e| !e.is_retryable()) {
                break;
            }
        }
        Err(last.as_ref().map_or_else(
            || Fail::new(429, "every credential for this model is cooling down"),
            Fail::from,
        ))
    }

    /// `provider/model` through Kor's registry and key pool (failover across its keys).
    async fn via_kor(
        &self,
        target: &str,
        req: ChatRequest,
        client_model: &str,
    ) -> Result<EventStream, Fail> {
        let mref: ModelRef = target.parse().map_err(|_| {
            Fail::new(
                404,
                format!("model `{client_model}` is not served by this proxy"),
            )
        })?;
        let provider = self.kor_provider(mref.provider())?;
        let mut r = req;
        r.model = mref.model().to_owned();
        match first_event(provider.as_ref(), r).await {
            Ok(s) => {
                self.stats
                    .request(mref.provider(), mref.model(), true, false);
                Ok(self.counted(s, mref.provider().to_owned(), mref.model().to_owned()))
            }
            Err(e) => {
                self.stats.request(
                    mref.provider(),
                    mref.model(),
                    false,
                    e.kind == ProviderErrorKind::RateLimited,
                );
                Err(Fail::from(&e))
            }
        }
    }

    fn kor_provider(&self, id: &str) -> Result<Arc<dyn Provider>, Fail> {
        if let Some(p) = self
            .kor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
        {
            return Ok(p.clone());
        }
        let m = self
            .sources
            .registry
            .get(id)
            .ok_or_else(|| Fail::new(404, format!("unknown provider `{id}`")))?;
        if self.sources.privacy && !(self.sources.is_local)(m) {
            return Err(Fail::new(
                403,
                format!("privacy mode: `{id}` is not a local provider"),
            ));
        }
        // A pool keeps its cooldown state, so it is built once per provider and reused.
        let p = kor_providers::build_pool(m, (self.sources.keys)(m)).map_err(|e| Fail::from(&e))?;
        self.kor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), p.clone());
        Ok(p)
    }

    fn counted(&self, s: EventStream, label: String, model: String) -> EventStream {
        let stats = self.stats.clone();
        let ledger = self.sources.ledger;
        Box::pin(s.inspect(move |ev| {
            if let Ok(StreamEvent::Usage { usage }) = ev {
                stats.tokens(&label, &model, usage);
                if ledger {
                    kor_runtime::usage::record_default(&model, usage.input, usage.output);
                }
            }
        }))
    }

    /// Client-visible models: configured names and aliases, Kor aliases, and (cached for ten
    /// minutes) Kor providers that are local or have a key, as `provider/model`.
    pub async fn models(&self) -> Vec<(String, String)> {
        let pool = self.pool();
        let mut out: Vec<(String, String)> = pool
            .listed_models(self.sources.privacy)
            .into_iter()
            .map(|(n, o)| (n, o.to_owned()))
            .collect();
        out.extend(
            pool.settings
                .aliases
                .keys()
                .map(|k| (k.clone(), "kor".to_owned())),
        );
        if !self.sources.list_registry {
            return out;
        }
        let mut cache = self.models.lock().await;
        if cache.as_ref().is_none_or(|c| c.at.elapsed() > MODELS_TTL) {
            *cache = Some(ModelCache {
                at: Instant::now(),
                models: self.registry_models().await,
            });
        }
        if let Some(c) = cache.as_ref() {
            out.extend(c.models.iter().cloned());
        }
        out
    }

    async fn registry_models(&self) -> Vec<(String, String)> {
        let mut providers = Vec::new();
        for m in self.sources.registry.iter() {
            let local = (self.sources.is_local)(m);
            if self.sources.privacy && !local {
                continue;
            }
            if (local || !(self.sources.keys)(m).is_empty())
                && let Ok(p) = self.kor_provider(&m.id)
            {
                providers.push((m.id.clone(), p));
            }
        }
        let lists = futures::future::join_all(providers.iter().map(|(id, p)| async move {
            match tokio::time::timeout(LIST_TIMEOUT, p.list_models()).await {
                Ok(Ok(models)) => models
                    .into_iter()
                    .map(|x| (format!("{id}/{}", x.id), id.clone()))
                    .collect(),
                _ => Vec::new(),
            }
        }))
        .await;
        lists.into_iter().flatten().collect()
    }

    pub fn usage_json(&self) -> Value {
        let mut v = self.stats.to_json();
        v["credentials"] = serde_json::to_value(self.credentials()).unwrap_or(Value::Null);
        let quota: serde_json::Map<String, Value> = kor_providers::health::snapshot()
            .into_iter()
            .filter(|(_, s)| !s.quota.is_empty())
            .map(|(id, s)| (id, serde_json::to_value(s.quota).unwrap_or(Value::Null)))
            .collect();
        v["quota"] = Value::Object(quota);
        v
    }

    /// `GET /v0/management/config`: the running settings, upstream keys masked.
    pub fn config_json(&self) -> Value {
        let st = self.settings();
        json!({
            "host": st.host, "port": st.port, "max-request-body-bytes": st.max_body,
            "api-keys": st.client_keys.iter().map(|k| kor_secrets::mask(k)).collect::<Vec<_>>(),
            "routing": {"strategy": format!("{:?}", st.strategy), "force-model-prefix": st.force_model_prefix,
                        "request-retry": st.request_retry, "max-retry-interval": st.max_retry_interval.as_secs()},
            "usage-statistics-enabled": st.usage_statistics,
            "credentials": self.credentials(),
            "kor-aliases": st.aliases,
            "privacy-mode": self.sources.privacy,
        })
    }
}

/// First event decides between success and failover; it is put back in front of the stream.
async fn first_event(p: &dyn Provider, r: ChatRequest) -> Result<EventStream, ProviderError> {
    let mut s = p.complete(r, CancellationToken::new()).await?;
    match s.next().await {
        Some(Err(e)) => Err(e),
        Some(Ok(ev)) => Ok(Box::pin(
            futures::stream::once(async move { Ok(ev) }).chain(s),
        )),
        None => Ok(s),
    }
}

// ---------------------------------------------------------------------------------------------
// Access key
// ---------------------------------------------------------------------------------------------

fn generate_key() -> Result<SecretString, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no secure random source: {e}"))?;
    let hex = b.iter().fold(String::with_capacity(64), |mut s, x| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{x:02x}"));
        s
    });
    Ok(SecretString::from(format!("kor-{hex}")))
}

/// The access key, created on first use when `create` is set.
pub fn access_key(store: &dyn SecretStore, create: bool) -> Result<Option<SecretString>, String> {
    if let Some(k) = store.get(KEY_ACCOUNT).map_err(|e| e.to_string())? {
        return Ok(Some(k));
    }
    if !create {
        return Ok(None);
    }
    rotate_key(store).map(Some)
}

/// Replace the access key with a fresh random one.
pub fn rotate_key(store: &dyn SecretStore) -> Result<SecretString, String> {
    let k = generate_key()?;
    store.set(KEY_ACCOUNT, &k).map_err(|e| e.to_string())?;
    Ok(k)
}

/// `sk-…` style masking of the access key (last four visible).
pub fn masked(k: &SecretString) -> String {
    kor_secrets::mask(k.expose_secret())
}

/// Ask a running proxy for its usage (`kor proxy status`). `None` when nothing listens there.
pub async fn remote_usage(base: &str, key: Option<&SecretString>) -> Result<Option<Value>, String> {
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let mut rq = client.get(format!("{base}/v0/management/usage"));
    if let Some(k) = key {
        rq = rq.bearer_auth(k.expose_secret());
    }
    let resp = match rq.send().await {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_key_is_created_once_and_rotates() {
        let store = kor_secrets::MemoryStore::default();
        assert!(access_key(&store, false).unwrap().is_none());
        let a = access_key(&store, true).unwrap().unwrap();
        let b = access_key(&store, true).unwrap().unwrap();
        assert_eq!(a.expose_secret(), b.expose_secret());
        assert!(a.expose_secret().starts_with("kor-") && a.expose_secret().len() == 68);
        let c = rotate_key(&store).unwrap();
        assert_ne!(a.expose_secret(), c.expose_secret());
        assert!(!masked(&c).contains(&c.expose_secret()[8..60]));
    }
}
