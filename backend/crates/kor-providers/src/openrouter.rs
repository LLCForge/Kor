//! `OpenRouter` extras beyond the `OpenAI`-compatible chat: per-call generation stats
//! (`GET /generation?id=`, <https://openrouter.ai/docs/api-reference/get-a-generation>).
//! Model prices and context come from the public `/models` list via [`crate::model_catalog`].

use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use serde_json::Value;

use crate::ProviderManifest;
use crate::http::Http;

/// Cost and token counts `OpenRouter` recorded for one generation.
#[derive(Debug, Clone, PartialEq)]
pub struct Generation {
    pub id: String,
    pub model: Option<String>,
    pub provider_name: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub tokens_prompt: Option<u64>,
    pub tokens_completion: Option<u64>,
    pub native_tokens_cached: Option<u64>,
    pub latency_ms: Option<u64>,
}

/// Stats of the generation `id` (the `id` field of the chat stream's chunks).
pub async fn generation(
    manifest: &ProviderManifest,
    key: SecretString,
    id: &str,
) -> Result<Generation, ProviderError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(ProviderError::new(
            &manifest.id,
            ProviderErrorKind::InvalidRequest,
            "invalid generation id",
        ));
    }
    let http = Http::new(manifest, Some(key))?;
    let v = http.get_json(&format!("/generation?id={id}")).await?;
    let d = &v["data"];
    Ok(Generation {
        id: d["id"].as_str().unwrap_or(id).to_owned(),
        model: d["model"].as_str().map(str::to_owned),
        provider_name: d["provider_name"].as_str().map(str::to_owned),
        total_cost_usd: d["total_cost"].as_f64(),
        tokens_prompt: d["tokens_prompt"].as_u64(),
        tokens_completion: d["tokens_completion"].as_u64(),
        native_tokens_cached: d["native_tokens_cached"].as_u64(),
        latency_ms: d["latency"].as_u64(),
    })
}

/// Credits bought and used on the account (`GET /credits`), in USD.
pub async fn credits(
    manifest: &ProviderManifest,
    key: SecretString,
) -> Result<(f64, f64), ProviderError> {
    let v = Http::new(manifest, Some(key))?.get_json("/credits").await?;
    let d = &v["data"];
    match (d["total_credits"].as_f64(), d["total_usage"].as_f64()) {
        (Some(total), Some(used)) => Ok((total, used)),
        _ => Err(ProviderError::new(
            &manifest.id,
            ProviderErrorKind::Protocol,
            "unexpected /credits response",
        )),
    }
}

/// Read a generation id from any chat chunk (`{"id": "gen-..."}`).
pub fn generation_id(chunk: &Value) -> Option<&str> {
    chunk["id"].as_str().filter(|s| s.starts_with("gen-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn generation_stats_are_parsed() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/generation"))
            .and(query_param("id", "gen-123"))
            .and(header("authorization", "Bearer sk-or-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"id": "gen-123", "model": "openai/gpt-4o", "total_cost": 0.0012,
                         "tokens_prompt": 10, "tokens_completion": 5, "native_tokens_cached": 4,
                         "latency": 321, "provider_name": "OpenAI"}
            })))
            .mount(&s)
            .await;
        let mut m = crate::Registry::builtin()
            .get("openrouter")
            .unwrap()
            .clone();
        m.base_url = format!("{}/api/v1", s.uri());
        let g = generation(&m, SecretString::from("sk-or-test"), "gen-123")
            .await
            .unwrap();
        assert_eq!(g.total_cost_usd, Some(0.0012));
        assert_eq!(
            (g.tokens_prompt, g.native_tokens_cached),
            (Some(10), Some(4))
        );
        assert!(
            generation(&m, SecretString::from("k"), "a&b")
                .await
                .is_err()
        );
        assert_eq!(
            generation_id(&serde_json::json!({"id": "gen-9"})),
            Some("gen-9")
        );
    }

    #[tokio::test]
    async fn credits_are_parsed() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/credits"))
            .and(header("authorization", "Bearer sk-or-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"total_credits": 20.0, "total_usage": 4.5}
            })))
            .mount(&s)
            .await;
        let mut m = crate::Registry::builtin()
            .get("openrouter")
            .unwrap()
            .clone();
        m.base_url = format!("{}/api/v1", s.uri());
        assert_eq!(
            credits(&m, SecretString::from("sk-or-test")).await.unwrap(),
            (20.0, 4.5)
        );
    }
}
