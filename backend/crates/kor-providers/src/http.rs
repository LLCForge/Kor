//! Shared HTTP plumbing: client policy, auth headers, error normalization.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::{ExposeSecret, Redactor, SecretString};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{Method, RequestBuilder, Response, StatusCode, redirect};
use serde_json::Value;

use crate::{AuthKind, ProviderManifest};

/// Error bodies beyond this are truncated before parsing.
const MAX_ERROR_BODY: usize = 64 * 1024;
/// Non-streaming JSON bodies (model lists) beyond this are rejected.
const MAX_JSON_BODY: usize = 16 * 1024 * 1024;

static REDACTOR: LazyLock<Redactor> = LazyLock::new(Redactor::default);

#[derive(Clone)]
pub(crate) struct Http {
    client: reqwest::Client,
    base: String,
    headers: HeaderMap,
    provider: String,
    extra_body: Option<Arc<Value>>,
    secret: Option<SecretString>,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Http")
            .field("provider", &self.provider)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Http {
    /// See [`crate::Provider::preconnect`]: a `HEAD` on the base URL, without auth headers, whose
    /// only purpose is the pooled connection it leaves behind.
    pub(crate) fn preconnect(&self) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let req = self
            .client
            .head(&self.base)
            .timeout(Duration::from_secs(10));
        rt.spawn(async move {
            let _ = req.send().await;
        });
    }

    pub(crate) fn new(
        m: &ProviderManifest,
        key: Option<SecretString>,
    ) -> Result<Self, ProviderError> {
        let invalid =
            |msg: String| ProviderError::new(&m.id, ProviderErrorKind::InvalidRequest, msg);
        let client = kor_core::net::client_builder()
            .user_agent(format!("kor/{}", kor_core::VERSION))
            .connect_timeout(Duration::from_secs(15))
            // §32.2 (ADR 0062): keep the connection to the provider warm between turns.
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(30))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_while_idle(true)
            // Gap between bytes on a stream, not total duration.
            .read_timeout(Duration::from_secs(300))
            // A redirect could carry the key to another host.
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|e| invalid(e.to_string()))?;

        let mut headers = HeaderMap::new();
        for (k, v) in &m.quirks.extra_headers {
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| invalid(e.to_string()))?;
            let value = HeaderValue::from_str(v).map_err(|e| invalid(e.to_string()))?;
            headers.insert(name, value);
        }
        if let (Some(key), true) = (&key, m.needs_key()) {
            let (name, raw) = match m.auth.kind {
                AuthKind::Bearer => (
                    m.auth
                        .header
                        .as_deref()
                        .map_or(Ok(AUTHORIZATION), |h| HeaderName::from_bytes(h.as_bytes())),
                    format!(
                        "{} {}",
                        m.auth.prefix.as_deref().unwrap_or("Bearer"),
                        key.expose_secret()
                    ),
                ),
                _ => (
                    HeaderName::from_bytes(m.auth.header.as_deref().unwrap_or_default().as_bytes()),
                    key.expose_secret().to_owned(),
                ),
            };
            let name = name.map_err(|e| invalid(e.to_string()))?;
            let mut value = HeaderValue::from_str(&raw).map_err(|_| {
                invalid("API key contains characters not allowed in a header".into())
            })?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }

        Ok(Self {
            client,
            base: m.base_url.trim_end_matches('/').to_owned(),
            headers,
            provider: m.id.clone(),
            extra_body: m.quirks.extra_body.clone().map(Arc::new),
            secret: key,
        })
    }

    pub(crate) fn provider(&self) -> &str {
        &self.provider
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, self.url(path))
            .headers(self.headers.clone())
    }

    /// POST a JSON body (with manifest `extra_body` merged in) and return the 2xx response.
    pub(crate) async fn post_json(
        &self,
        path: &str,
        mut body: Value,
        extra_headers: &[(&'static str, &'static str)],
    ) -> Result<Response, ProviderError> {
        mask_body(&mut body);
        if let Some(extra) = &self.extra_body {
            merge(&mut body, extra);
        }
        let mut req = self
            .request(Method::POST, path)
            .header("content-type", "application/json");
        for (k, v) in extra_headers {
            req = req.header(*k, *v);
        }
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| self.err(ProviderErrorKind::Protocol, e.to_string()))?;
        self.send(req.body(bytes)).await
    }

    /// POST a JSON body as is. For endpoints other than chat (image, video): the manifest's
    /// `extra_body` holds chat parameters those endpoints reject.
    pub(crate) async fn post_plain_json(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<Response, ProviderError> {
        let req = self
            .request(Method::POST, path)
            .header("content-type", "application/json");
        let mut body = body.clone();
        mask_body(&mut body);
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| self.err(ProviderErrorKind::Protocol, e.to_string()))?;
        self.send(req.body(bytes)).await
    }

    pub(crate) async fn get_json(&self, path: &str) -> Result<Value, ProviderError> {
        let resp = self.send(self.request(Method::GET, path)).await?;
        self.read_json(resp).await
    }

    pub(crate) async fn read_json(&self, resp: Response) -> Result<Value, ProviderError> {
        let body = read_bounded(resp, MAX_JSON_BODY)
            .await
            .map_err(|e| self.transport(&e))?;
        serde_json::from_slice(&body)
            .map_err(|e| self.err(ProviderErrorKind::Protocol, e.to_string()))
    }

    pub(crate) async fn send(&self, req: RequestBuilder) -> Result<Response, ProviderError> {
        // The resolver guards names; an IP-literal base URL (a LAN server) is checked here.
        kor_core::net::check_url(&self.base)
            .map_err(|b| self.err(ProviderErrorKind::InvalidRequest, b.message))?;
        let resp = req.send().await.map_err(|e| self.transport(&e))?;
        crate::health::record_headers(&self.provider, resp.headers());
        crate::quota::record(&self.provider, self.secret.as_ref(), resp.headers());
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = read_bounded(resp, MAX_ERROR_BODY).await.unwrap_or_default();
        let mut e = self.classify(status, &body);
        e.retry_after = retry_after;
        Err(e)
    }

    fn classify(&self, status: StatusCode, body: &[u8]) -> ProviderError {
        let text = String::from_utf8_lossy(body);
        let message = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| error_message(&v))
            .unwrap_or_else(|| text.chars().take(500).collect());
        let mut kind = ProviderError::kind_from_status(status.as_u16());
        let lower = text.to_ascii_lowercase();
        if [
            "context_length_exceeded",
            "maximum context length",
            "prompt is too long",
            "too many tokens",
            "context window",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            kind = ProviderErrorKind::ContextTooLong;
        } else if status == StatusCode::BAD_REQUEST
            && ["content_filter", "content_policy", "safety"]
                .iter()
                .any(|s| lower.contains(s))
        {
            kind = ProviderErrorKind::ContentFiltered;
        } else if lower.contains("overloaded") {
            kind = ProviderErrorKind::Unavailable;
        }
        let mut e = self.err(kind, message);
        e.http_status = Some(status.as_u16());
        e
    }

    pub(crate) fn transport(&self, e: &reqwest::Error) -> ProviderError {
        // Refused by the network guard: not retryable, no fallback, the guard's own message.
        if let Some(b) = kor_core::net::blocked_in(e) {
            return self.err(ProviderErrorKind::InvalidRequest, &b.message);
        }
        let kind = if e.is_timeout() {
            ProviderErrorKind::Timeout
        } else if e.is_connect() || e.is_request() {
            ProviderErrorKind::Unavailable
        } else {
            ProviderErrorKind::Protocol
        };
        // Strip the URL: it may carry query parameters.
        let mut msg = e.to_string();
        if let Some(u) = e.url() {
            msg = msg.replace(u.as_str(), "<url>");
        }
        self.err(kind, msg)
    }

    /// Build an error with the message redacted (provider bodies may echo keys).
    pub(crate) fn err(&self, kind: ProviderErrorKind, message: impl AsRef<str>) -> ProviderError {
        let mut m = REDACTOR.redact(message.as_ref()).into_owned();
        if let Some(s) = &self.secret
            && !s.expose_secret().is_empty()
        {
            m = m.replace(s.expose_secret(), "[REDACTED]");
        }
        ProviderError::new(&self.provider, kind, m)
    }
}

/// `{"error": {"message": ...}}`, `{"error": "..."}`, `{"message": ...}`.
/// Venice adds `details` (which field is wrong) to 400s; it is appended, shortened.
pub(crate) fn error_message(v: &Value) -> Option<String> {
    let e = v.get("error").unwrap_or(v);
    let message = e
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .map(str::to_owned)?;
    let details = match v.get("details") {
        Some(Value::String(s)) => s.clone(),
        Some(d @ (Value::Object(_) | Value::Array(_))) => d.to_string(),
        _ => return Some(message),
    };
    let details: String = details.chars().take(300).collect();
    Some(format!("{message}: {details}"))
}

async fn read_bounded(mut resp: Response, max: usize) -> Result<Vec<u8>, reqwest::Error> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        let room = max.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

/// Fields that carry binary data or opaque provider tokens (base64 images, reasoning signatures,
/// encrypted reasoning); they must round-trip byte for byte and hold no user text.
const OPAQUE_KEYS: &[&str] = &[
    "data",
    "signature",
    "encrypted_content",
    "thought_signature",
    "thoughtSignature",
];

/// Mask secrets in every string of a model-bound request body (§6.6). This is the one choke
/// point: chat, tool results, file reads, MCP output, recall, repo map, summaries, sub-agents
/// and bench all reach a provider through [`Http::post_json`] / [`Http::post_plain_json`]
/// (counted in [`kor_secrets::outbound_masked_total`]).
pub(crate) fn mask_body(v: &mut Value) {
    match v {
        Value::String(s) if !s.starts_with("data:") => {
            if let std::borrow::Cow::Owned(masked) = kor_secrets::mask_outbound(s) {
                *s = masked;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(mask_body),
        Value::Object(m) => m
            .iter_mut()
            .filter(|(k, _)| !OPAQUE_KEYS.contains(&k.as_str()))
            .for_each(|(_, v)| mask_body(v)),
        _ => {}
    }
}

/// Deep-merge `extra` into `base`; objects merge, everything else replaces.
pub(crate) fn merge(base: &mut Value, extra: &Value) {
    match (base, extra) {
        (Value::Object(b), Value::Object(e)) => {
            for (k, v) in e {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, e) => *b = e.clone(),
    }
}

/// Model ids are interpolated into URL paths by some adapters.
pub(crate) fn check_model_id(provider: &str, model: &str) -> Result<(), ProviderError> {
    let ok = !model.is_empty()
        && model.len() <= 256
        && !model.contains("..")
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:/@".contains(&b));
    if ok {
        Ok(())
    } else {
        Err(ProviderError::new(
            provider,
            ProviderErrorKind::InvalidRequest,
            format!("invalid model id `{model}`"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_is_deep() {
        let mut a = json!({"a": {"x": 1}, "b": 1});
        merge(&mut a, &json!({"a": {"y": 2}, "b": {"z": 3}}));
        assert_eq!(a, json!({"a": {"x": 1, "y": 2}, "b": {"z": 3}}));
    }

    #[test]
    fn request_bodies_are_masked_except_opaque_fields() {
        kor_secrets::register_outbound(SecretString::from("vault-secret-for-http-test"));
        let sig = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWx";
        let mut body = json!({
            "system": "conn postgres://app:s3cretpw@db/app",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "token vault-secret-for-http-test"}]},
                {"role": "tool", "content": "read .env: OPENAI_API_KEY=sk-proj-AbCdEfGhIjKlMnOpQrStUv"},
                {"type": "thinking", "signature": sig},
                {"type": "image", "source": {"data": sig}},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{sig}")}}
            ]
        });
        mask_body(&mut body);
        let text = body.to_string();
        for leaked in ["s3cretpw", "vault-secret-for-http-test", "sk-proj-"] {
            assert!(!text.contains(leaked), "{leaked} in {text}");
        }
        assert_eq!(body["messages"][2]["signature"], sig);
        assert_eq!(body["messages"][3]["source"]["data"], sig);
        assert!(
            body["messages"][4]["image_url"]["url"]
                .as_str()
                .unwrap()
                .ends_with(sig)
        );
    }

    #[tokio::test]
    async fn post_json_sends_the_masked_body() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let m = crate::ProviderManifest::parse(
            &format!(
                "id = \"t\"\nname = \"T\"\ncategory = \"local\"\nadapter = \"openai_compat\"\n\
                 base_url = \"{}\"\nauth = {{ kind = \"none\" }}\n\
                 [privacy]\ntier = \"local\"\nretention = \"x\"\n",
                server.uri()
            ),
            "t",
        )
        .unwrap();
        let http = Http::new(&m, None).unwrap();
        http.post_json("/chat", json!({"m": "AKIAABCDEFGHIJKLMNOP"}), &[])
            .await
            .unwrap();
        let got = &server.received_requests().await.unwrap()[0];
        let sent = String::from_utf8_lossy(&got.body);
        assert!(
            !sent.contains("AKIA") && sent.contains("[REDACTED]"),
            "{sent}"
        );
    }

    #[test]
    fn model_ids() {
        assert!(check_model_id("p", "gemini-2.5-pro").is_ok());
        assert!(check_model_id("p", "meta-llama/Llama-3:groq").is_ok());
        for bad in ["", "../x", "a?b", "a#b", "a b"] {
            assert!(check_model_id("p", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn error_message_shapes() {
        assert_eq!(
            error_message(&json!({"error": {"message": "m"}})).as_deref(),
            Some("m")
        );
        assert_eq!(error_message(&json!({"error": "m"})).as_deref(), Some("m"));
        assert_eq!(
            error_message(&json!({"message": "m"})).as_deref(),
            Some("m")
        );
    }
}
