//! Saved Claude OAuth accounts become chat credentials.
//!
//! A `type: claude` auth file is one pool entry. Requests whose model name starts with `claude`
//! go to `https://api.anthropic.com/v1/messages` with the subscription bearer token and the
//! messages TLS client. Plain `http` bases exist so tests can stand in for the upstream.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use kor_core::{ContentPart, FinishReason, ImageSource, PrivacyTier, ProviderError, ProviderErrorKind, Role, StreamEvent, Usage};
use kor_providers::{
    AdapterKind, Auth, AuthKind, CancellationToken, Category, ChatRequest, EventStream, ModelInfo, Privacy, Provider, ProviderManifest,
    Quirks,
};
use serde_json::{Value, json};

use super::{Error, err, string_field};
use crate::config::{Group, Kind, Settings};
use crate::routing::Credential;

const BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14";
const USER_AGENT: &str = "claude-cli/2.1.220 (external, cli)";
const MESSAGES: &str = "https://api.anthropic.com/v1";

pub(crate) fn load_subscriptions(dir: &Path, settings: &mut Settings, creds: &mut Vec<Credential>) -> Vec<String> {
    let mut notes = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return notes,
        Err(e) => {
            notes.push(format!("oauth auth dir: {e}"));
            return notes;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match load_file(&path, settings.groups.len()) {
            Ok(Some((group, cred))) => {
                settings.groups.push(group);
                creds.push(cred);
            }
            Ok(None) => {}
            Err(e) => notes.push(format!("{}: {e}", path.display())),
        }
    }
    notes
}

fn load_file(path: &Path, group: usize) -> Result<Option<(Group, Credential)>, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| err(e.to_string()))?;
    if doc.get("type").and_then(Value::as_str) != Some("claude") {
        return Ok(None);
    }
    let access = string_field(&doc, "access_token");
    if access.is_empty() {
        return Err(err("claude auth file has no access token"));
    }
    let email = string_field(&doc, "email");
    let label = if email.is_empty() { "claude".to_owned() } else { email };
    let provider = Arc::new(ClaudeSession {
        manifest: manifest(),
        token: access.clone(),
        base: MESSAGES.to_owned(),
        http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| err(e.to_string()))?,
    });
    let group_spec = Group {
        kind: Kind::Claude,
        name: label.clone(),
        base_url: Some(MESSAGES.to_owned()),
        prefix: None,
        priority: 0,
        models: Vec::new(),
        excluded: Vec::new(),
        keys: Vec::new(),
        disable_cooling: None,
    };
    let cred = Credential::subscription(label, group, Kind::Claude, provider, kor_secrets::mask(&access), MESSAGES.to_owned());
    Ok(Some((group_spec, cred)))
}

fn manifest() -> ProviderManifest {
    ProviderManifest {
        id: "claude-oauth".to_owned(),
        name: "Claude".to_owned(),
        category: Category::Standard,
        adapter: AdapterKind::Anthropic,
        base_url: MESSAGES.to_owned(),
        base_url_env: None,
        auth: Auth { kind: AuthKind::Bearer, env: None, header: None, prefix: None },
        models_endpoint: None,
        static_models: Vec::new(),
        tags: vec!["oauth".to_owned()],
        streaming: true,
        tool_calling: true,
        vision: true,
        json_mode: false,
        thinking: true,
        max_context_default: None,
        verified: false,
        docs: None,
        privacy: Privacy {
            tier: PrivacyTier::NoTrain,
            retention: "Anthropic subscription".to_owned(),
            training_on_data: Some(false),
            jurisdiction: None,
            policy_url: None,
            notes: None,
        },
        quirks: Quirks::default(),
    }
}

struct ClaudeSession {
    manifest: ProviderManifest,
    token: String,
    base: String,
    http: reqwest::Client,
}

#[async_trait]
impl Provider for ClaudeSession {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(&self, req: ChatRequest, _cancel: CancellationToken) -> Result<EventStream, ProviderError> {
        let body = message_body(&req);
        let url = format!("{}/messages", self.base.trim_end_matches('/'));
        let token = self.token.clone();
        if self.base.starts_with("https://") {
            let response = super::fingerprint::messages_client()
                .post(&url)
                .bearer_auth(&token)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", BETA)
                .header("user-agent", USER_AGENT)
                .json(&body)
                .send()
                .await
                .map_err(|e| fail(&e.to_string()))?;
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                let text = response.text().await.unwrap_or_default();
                return Err(status_error(status, &text));
            }
            Ok(sse_stream(response.bytes_stream().map(|c| c.map_err(|e| e.to_string()))))
        } else {
            let response = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", BETA)
                .header("user-agent", USER_AGENT)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .map_err(|e| fail(&e.to_string()))?;
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                let text = response.text().await.unwrap_or_default();
                return Err(status_error(status, &text));
            }
            Ok(sse_stream(response.bytes_stream().map(|c| c.map_err(|e| e.to_string()))))
        }
    }
}

fn message_body(req: &ChatRequest) -> Value {
    let mut messages = Vec::new();
    let mut system = Vec::new();
    if let Some(text) = &req.system {
        system.push(json!({"type": "text", "text": text}));
    }
    for message in &req.messages {
        if message.role == Role::System {
            for part in &message.parts {
                if let ContentPart::Text { text } = part {
                    system.push(json!({"type": "text", "text": text}));
                }
            }
            continue;
        }
        let role = if message.role == Role::Assistant { "assistant" } else { "user" };
        let blocks: Vec<Value> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(json!({"type": "text", "text": text})),
                ContentPart::Image { source: ImageSource::Base64 { media_type, data } } => {
                    Some(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}}))
                }
                ContentPart::ToolUse { id, name, input } => Some(json!({"type": "tool_use", "id": id, "name": name, "input": input})),
                ContentPart::ToolResult { tool_use_id, content, is_error } => {
                    Some(json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error}))
                }
                _ => None,
            })
            .collect();
        if !blocks.is_empty() {
            messages.push(json!({"role": role, "content": blocks}));
        }
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(8192),
        "messages": messages,
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    body
}

fn sse_stream<S>(incoming: S) -> EventStream
where
    S: futures::Stream<Item = Result<bytes::Bytes, String>> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    tokio::spawn(async move {
        let mut incoming = std::pin::pin!(incoming);
        let mut buf = String::new();
        let mut finished = false;
        while let Some(chunk) = incoming.next().await {
            match chunk {
                Ok(bytes) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(split) = buf.find('\n') {
                        let line = buf[..split].trim_end_matches('\r').to_owned();
                        buf.drain(..=split);
                        for event in parse_line(&line) {
                            if matches!(event, Ok(StreamEvent::Finish { .. })) {
                                finished = true;
                            }
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(fail(&e))).await;
                    return;
                }
            }
        }
        if !finished {
            let _ = tx
                .send(Ok(StreamEvent::Finish { reason: FinishReason::Stop }))
                .await;
        }
    });
    Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|event| (event, rx)) }))
}

fn parse_line(line: &str) -> Vec<Result<StreamEvent, ProviderError>> {
    let Some(data) = line.strip_prefix("data:") else {
        return Vec::new();
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "content_block_delta" => match value.pointer("/delta/type").and_then(Value::as_str).unwrap_or("") {
            "text_delta" => vec![Ok(StreamEvent::TextDelta { text: text_at(&value, "/delta/text") })],
            "thinking_delta" => vec![Ok(StreamEvent::ThinkingDelta { text: text_at(&value, "/delta/thinking") })],
            "input_json_delta" => vec![Ok(StreamEvent::ToolCallDelta {
                index: index_of(&value),
                json: text_at(&value, "/delta/partial_json"),
            })],
            _ => Vec::new(),
        },
        "content_block_start" if value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") => {
            vec![Ok(StreamEvent::ToolCallStart {
                index: index_of(&value),
                id: text_at(&value, "/content_block/id"),
                name: text_at(&value, "/content_block/name"),
            })]
        }
        "message_delta" => {
            let reason = match value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                Some("tool_use") => FinishReason::ToolUse,
                Some("max_tokens") => FinishReason::MaxTokens,
                _ => FinishReason::Stop,
            };
            let mut events = Vec::new();
            if let Some(usage) = value.get("usage") {
                events.push(Ok(StreamEvent::Usage { usage: usage_of(usage) }));
            }
            events.push(Ok(StreamEvent::Finish { reason }));
            events
        }
        "error" => vec![Err(fail(&text_at(&value, "/error/message")))],
        _ => Vec::new(),
    }
}

fn usage_of(value: &Value) -> Usage {
    Usage {
        input: value.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output: value.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_read: value.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_write: value.get("cache_creation_input_tokens").and_then(Value::as_u64).unwrap_or(0),
        reasoning: 0,
    }
}

fn text_at(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn index_of(value: &Value) -> u32 {
    value.get("index").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0)
}

fn fail(message: &str) -> ProviderError {
    ProviderError::new("claude-oauth", ProviderErrorKind::Unavailable, message)
}

fn status_error(status: u16, body: &str) -> ProviderError {
    let kind = match status {
        401 | 403 => ProviderErrorKind::AuthFailed,
        429 => ProviderErrorKind::RateLimited,
        400 => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unavailable,
    };
    let snippet: String = body.chars().filter(|c| !c.is_control()).take(180).collect();
    ProviderError::new("claude-oauth", kind, format!("HTTP {status}: {snippet}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_core::Message;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn a_saved_claude_account_answers_a_chat_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n\
                 data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            ))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("claude-a.json");
        std::fs::write(
            &file,
            r#"{"type":"claude","access_token":"subscription-token","refresh_token":"rt","email":"a@b.c","expired":"2099-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        let mut settings = Settings::default();
        let mut creds = Vec::new();
        let notes = load_subscriptions(dir.path(), &mut settings, &mut creds);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(creds.len(), 1);
        assert_eq!(settings.groups[0].kind, Kind::Claude);
        let session = ClaudeSession {
            manifest: manifest(),
            token: "subscription-token".to_owned(),
            base: server.uri(),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        };
        let started = session
            .complete(
                ChatRequest {
                    model: "claude-sonnet-4-5".to_owned(),
                    system: None,
                    messages: vec![Message::text(Role::User, "hi")],
                    tools: Vec::new(),
                    max_tokens: Some(32),
                    temperature: None,
                    reasoning: None,
                },
                CancellationToken::new(),
            )
            .await;
        let mut stream = match started {
            Ok(stream) => stream,
            Err(e) => {
                let got = server.received_requests().await.unwrap();
                panic!("{e}; received {got:?}");
            }
        };
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, StreamEvent::TextDelta { text: "hello".to_owned() });
        let got = server.received_requests().await.unwrap();
        let headers = &got[0].headers;
        assert_eq!(
            headers.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer subscription-token")
        );
        assert!(headers.get("anthropic-beta").and_then(|v| v.to_str().ok()).unwrap_or("").contains("oauth-2025-04-20"));
    }

    #[test]
    fn the_pool_serves_claude_models_from_the_account() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-a.json"),
            r#"{"type":"claude","access_token":"subscription-token","email":"a@b.c"}"#,
        )
        .unwrap();
        let mut settings = Settings::default();
        let mut creds = Vec::new();
        load_subscriptions(dir.path(), &mut settings, &mut creds);
        let pool = crate::routing::Pool::new(settings, creds);
        let order = pool.order("claude-sonnet-4-5", false);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].model, "claude-sonnet-4-5");
    }
}
