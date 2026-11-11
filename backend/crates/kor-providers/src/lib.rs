//! Provider abstraction for Kor: one `Provider` trait, a few protocol adapters, and
//! declarative manifests describing each concrete provider.

mod adapters;
pub mod catalog;
pub mod custom;
pub mod fal;
pub mod health;
pub mod huggingface;
mod http;
mod manifest;
pub mod media;
pub mod media_catalog;
pub mod model_catalog;
pub mod openrouter;
pub mod pool;
pub mod privacy_card;
pub mod probe;
pub mod quota;
mod stream;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
pub use kor_core::{Reasoning, ReasoningEffort};
use kor_core::{Message, ProviderError, ProviderErrorKind, StreamEvent, ToolSpec};
use kor_secrets::SecretString;
use serde::{Deserialize, Serialize};
pub use tokio_util::sync::CancellationToken;

pub use manifest::{
    AdapterKind, Auth, AuthKind, Category, ManifestError, Privacy, ProviderManifest, Quirks,
    Registry,
};

/// Events of one completion. Ends after `Finish` or the first `Err`.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Model id as the provider knows it (the part after `provider/`).
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Reasoning effort / thinking budget; `None` leaves the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    pub models: usize,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str {
        &self.manifest().id
    }

    fn manifest(&self) -> &ProviderManifest;

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;

    /// Always a stream; a provider without streaming emits one burst.
    async fn complete(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError>;

    /// Input token count, when the provider offers an endpoint for it.
    async fn count_tokens(&self, _req: &ChatRequest) -> Result<Option<u64>, ProviderError> {
        Ok(None)
    }

    async fn health(&self) -> Result<Health, ProviderError> {
        let models = self.list_models().await?.len();
        Ok(Health { ok: true, models })
    }

    /// Open the connection (DNS, TCP, TLS, HTTP/2) to the provider in the background so the
    /// first token does not wait for it (§32.2, ADR 0062). No-op outside a tokio runtime; sends no
    /// key; the network guard applies as for any request.
    fn preconnect(&self) {}
}

/// Instantiate the adapter a manifest names. `key` is required unless `auth.kind = "none"`.
pub fn build(
    manifest: &ProviderManifest,
    key: Option<SecretString>,
) -> Result<Arc<dyn Provider>, ProviderError> {
    let invalid = |msg: String| ProviderError::new(&manifest.id, ProviderErrorKind::InvalidRequest, msg);
    if manifest.tags.iter().any(|t| t == "retired") {
        return Err(invalid(format!(
            "{} has been retired by its operator; see {}",
            manifest.name,
            manifest.docs.as_deref().unwrap_or("the provider docs")
        )));
    }
    if manifest.adapter == AdapterKind::Sagemaker {
        return Err(invalid(
            "SageMaker needs SigV4 signing, not implemented yet; serve the model with an \
             OpenAI-compatible container and use `kor provider add <id> --url <base>`"
                .into(),
        ));
    }
    let manifest = &*manifest
        .resolved(|v| std::env::var(v).ok())
        .map_err(|e| invalid(e.message))?;
    if manifest.needs_key() && key.is_none() {
        return Err(ProviderError::new(
            &manifest.id,
            ProviderErrorKind::AuthFailed,
            format!(
                "no API key (set {} or run `kor auth login {}`)",
                manifest.key_env(),
                manifest.id
            ),
        ));
    }
    let http = http::Http::new(manifest, key)?;
    let m = manifest.clone();
    Ok(match manifest.adapter {
        AdapterKind::OpenaiCompat => Arc::new(adapters::openai_compat::OpenAiCompat::new(m, http)),
        AdapterKind::Openai => Arc::new(adapters::openai::OpenAi::new(m, http)),
        AdapterKind::Anthropic => Arc::new(adapters::anthropic::Anthropic::new(m, http)),
        AdapterKind::Gemini => Arc::new(adapters::gemini::Gemini::new(m, http)),
        AdapterKind::Ollama => Arc::new(adapters::ollama::Ollama::new(m, http)),
        AdapterKind::Cohere => Arc::new(adapters::cohere::Cohere::new(m, http)),
        other => {
            return Err(ProviderError::new(
                &manifest.id,
                ProviderErrorKind::InvalidRequest,
                format!("adapter `{}` is not available yet", other.as_str()),
            ));
        }
    })
}

/// One provider over several keys (§4.5 key pool): on 429/5xx/timeout the next key takes the
/// same request. Always a [`pool::Failover`], so every call lands in [`health`]; no keys means
/// a keyless provider (local).
pub fn build_pool(
    manifest: &ProviderManifest,
    keys: Vec<SecretString>,
) -> Result<Arc<dyn Provider>, ProviderError> {
    let keys: Vec<Option<SecretString>> = if keys.is_empty() {
        vec![None]
    } else {
        keys.into_iter().map(Some).collect()
    };
    let members = keys
        .into_iter()
        .enumerate()
        .map(|(i, k)| {
            Ok(pool::Member {
                provider: build(manifest, k)?,
                model: None,
                label: if i == 0 {
                    manifest.id.clone()
                } else {
                    format!("{}#{}", manifest.id, i + 1)
                },
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    pool::Failover::new(members)
        .map(|f| Arc::new(f) as Arc<dyn Provider>)
        .ok_or_else(|| ProviderError::new(&manifest.id, ProviderErrorKind::AuthFailed, "no key"))
}

/// `primary`, then each `(provider, model)` fallback when the one before is exhausted.
pub fn with_fallbacks(
    primary: Arc<dyn Provider>,
    fallbacks: Vec<(Arc<dyn Provider>, String)>,
) -> Arc<dyn Provider> {
    if fallbacks.is_empty() {
        return primary;
    }
    let mut members = vec![pool::Member {
        // Empty label: the primary records its own health (per key when pooled).
        label: String::new(),
        provider: primary.clone(),
        model: None,
    }];
    members.extend(fallbacks.into_iter().map(|(p, model)| pool::Member {
        label: p.id().to_owned(),
        provider: p,
        model: Some(model),
    }));
    pool::Failover::new(members).map_or(primary, |f| Arc::new(f) as Arc<dyn Provider>)
}
