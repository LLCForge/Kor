//! Anthropic Messages API.

use std::collections::BTreeSet;

use async_trait::async_trait;
use kor_core::{
    ContentPart, FinishReason, ImageSource, ProviderError, ProviderErrorKind, Role, StreamEvent,
    Usage,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{parse, str_at, system_text, turns, u64_at, unsupported};
use crate::http::{Http, error_message};
use crate::stream::{self, Frame, Framing, Mapper};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

const VERSION: (&str, &str) = ("anthropic-version", "2023-06-01");
/// `max_tokens` is mandatory in this API.
const DEFAULT_MAX_TOKENS: u32 = 8192;

#[derive(Debug)]
pub(crate) struct Anthropic {
    manifest: ProviderManifest,
    http: Http,
}

impl Anthropic {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

fn messages(req: &ChatRequest, provider: &str) -> Result<Vec<Value>, ProviderError> {
    let mut out: Vec<Value> = Vec::new();
    for m in turns(req) {
        let role = if m.role == Role::Assistant {
            "assistant"
        } else {
            "user"
        };
        let mut blocks = Vec::new();
        for p in &m.parts {
            blocks.push(match p {
                ContentPart::Text { text } => json!({"type": "text", "text": text}),
                ContentPart::Image { source: ImageSource::Base64 { media_type, data } } => json!({
                    "type": "image", "source": {"type": "base64", "media_type": media_type, "data": data},
                }),
                ContentPart::Image { source: ImageSource::Url { url } } => {
                    json!({"type": "image", "source": {"type": "url", "url": url}})
                }
                ContentPart::ToolUse { id, name, input } => {
                    json!({"type": "tool_use", "id": id, "name": name, "input": input})
                }
                ContentPart::ToolResult { tool_use_id, content, is_error } => json!({
                    "type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error,
                }),
                ContentPart::Thinking { text, signature: Some(sig) } => {
                    json!({"type": "thinking", "thinking": text, "signature": sig})
                }
                // Unsigned thinking cannot be replayed.
                ContentPart::Thinking { signature: None, .. } => continue,
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            });
        }
        if blocks.is_empty() {
            continue;
        }
        // The API requires alternating roles; merge consecutive same-role turns.
        match out.last_mut() {
            Some(last) if last["role"] == role => {
                if let Some(arr) = last["content"].as_array_mut() {
                    arr.extend(blocks);
                }
            }
            _ => out.push(json!({"role": role, "content": blocks})),
        }
    }
    Ok(out)
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": messages(req, provider)?,
        "stream": true,
    });
    if let Some(s) = system_text(req) {
        body["system"] = json!([{"type": "text", "text": s}]);
    }
    if !req.tools.is_empty() {
        body["tools"] = req.tools.iter().map(tool_body).collect();
    }
    let thinking = add_reasoning(&mut body, req);
    // Thinking is not compatible with a modified temperature.
    if let Some(t) = req.temperature.filter(|_| !thinking) {
        body["temperature"] = t.into();
    }
    add_cache_breakpoints(&mut body);
    Ok(body)
}

/// Smallest `budget_tokens` the API accepts.
const MIN_THINKING_BUDGET: u32 = 1_024;

/// A budget becomes manual thinking (`{type: "enabled", budget_tokens}`, which only pre-4.6
/// models take): raised to the 1024 minimum, and `max_tokens` grown past it because the budget
/// must stay below `max_tokens`. A budget of 0 means "no thinking" and sends nothing. An effort
/// alone becomes adaptive thinking plus `output_config.effort` (low..max; none/minimal → low).
/// Returns whether thinking was turned on.
fn add_reasoning(body: &mut Value, req: &ChatRequest) -> bool {
    use kor_core::ReasoningEffort as E;
    let Some(r) = req.reasoning else { return false };
    match (r.budget_tokens, r.effort) {
        (Some(0), _) | (None, None) => false,
        (Some(b), _) => {
            let budget = b.max(MIN_THINKING_BUDGET);
            let max = body["max_tokens"]
                .as_u64()
                .map_or(DEFAULT_MAX_TOKENS, |m| u32::try_from(m).unwrap_or(u32::MAX));
            if max <= budget {
                body["max_tokens"] = budget.saturating_add(max).into();
            }
            body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            true
        }
        (None, Some(e)) => {
            let effort = match e {
                E::None | E::Minimal => E::Low,
                e => e,
            };
            body["thinking"] = json!({"type": "adaptive"});
            body["output_config"] = json!({"effort": effort.as_str()});
            true
        }
    }
}

/// Prompt caching (§4.5): breakpoints on the system prompt, the last tool and the newest
/// message, so each turn reads the previous turn's prefix from cache. At most 3 of the API's 4.
fn add_cache_breakpoints(body: &mut Value) {
    let mark = json!({"type": "ephemeral"});
    if let Some(s) = body["system"].as_array_mut().and_then(|a| a.last_mut()) {
        s["cache_control"] = mark.clone();
    }
    if let Some(t) = body["tools"].as_array_mut().and_then(|a| a.last_mut()) {
        t["cache_control"] = mark.clone();
    }
    let last_block = body["messages"]
        .as_array_mut()
        .and_then(|a| a.last_mut())
        .and_then(|m| m["content"].as_array_mut())
        .and_then(|c| c.last_mut());
    if let Some(b) = last_block
        && b["type"] != "thinking"
    {
        b["cache_control"] = mark;
    }
}

fn tool_body(t: &kor_core::ToolSpec) -> Value {
    if t.name == "WebSearch" {
        json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        })
    } else {
        json!({"name": t.name, "description": t.description, "input_schema": t.input_schema})
    }
}

#[derive(Debug, Default)]
pub(crate) struct MessagesMapper {
    provider: String,
    tools: BTreeSet<u32>,
    usage: Usage,
    finish: Option<FinishReason>,
    done: bool,
}

impl MessagesMapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }
}

fn error_kind(t: Option<&str>) -> ProviderErrorKind {
    match t {
        Some("rate_limit_error") => ProviderErrorKind::RateLimited,
        Some("authentication_error" | "permission_error") => ProviderErrorKind::AuthFailed,
        Some("invalid_request_error") => ProviderErrorKind::InvalidRequest,
        _ => ProviderErrorKind::Unavailable,
    }
}

impl Mapper for MessagesMapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = parse(&self.provider, &f.data)?;
        let index = u32::try_from(u64_at(&v, "/index")).unwrap_or(0);
        let s = |p: &str| str_at(&v, p).unwrap_or_default().to_owned();
        let ev = match str_at(&v, "/type").unwrap_or_default() {
            "message_start" => {
                let u = &v["message"]["usage"];
                // Anthropic counts cached prompt tokens apart from `input_tokens`; Kor's `input`
                // is the whole prompt (as the other APIs report it), with the cached part inside.
                let (cache_read, cache_write) = (
                    u64_at(u, "/cache_read_input_tokens"),
                    u64_at(u, "/cache_creation_input_tokens"),
                );
                self.usage = Usage {
                    input: u64_at(u, "/input_tokens")
                        .saturating_add(cache_read)
                        .saturating_add(cache_write),
                    output: u64_at(u, "/output_tokens"),
                    cache_read,
                    cache_write,
                    reasoning: 0,
                };
                return Ok(Vec::new());
            }
            "content_block_start" if str_at(&v, "/content_block/type") == Some("tool_use") => {
                self.tools.insert(index);
                StreamEvent::ToolCallStart {
                    index,
                    id: s("/content_block/id"),
                    name: s("/content_block/name"),
                }
            }
            "content_block_delta" => match str_at(&v, "/delta/type").unwrap_or_default() {
                "text_delta" => StreamEvent::TextDelta {
                    text: s("/delta/text"),
                },
                "input_json_delta" => StreamEvent::ToolCallDelta {
                    index,
                    json: s("/delta/partial_json"),
                },
                "thinking_delta" => StreamEvent::ThinkingDelta {
                    text: s("/delta/thinking"),
                },
                "signature_delta" => StreamEvent::ThinkingSignature {
                    signature: s("/delta/signature"),
                },
                _ => return Ok(Vec::new()),
            },
            "content_block_stop" if self.tools.remove(&index) => StreamEvent::ToolCallEnd { index },
            "message_delta" => {
                if let Some(out) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.usage.output = out;
                }
                self.finish = str_at(&v, "/delta/stop_reason").map(|r| match r {
                    "max_tokens" => FinishReason::MaxTokens,
                    "tool_use" => FinishReason::ToolUse,
                    "refusal" => FinishReason::ContentFiltered,
                    _ => FinishReason::Stop,
                });
                return Ok(Vec::new());
            }
            "message_stop" => {
                self.done = true;
                return Ok(vec![
                    StreamEvent::Usage { usage: self.usage },
                    StreamEvent::Finish {
                        reason: self.finish.unwrap_or(FinishReason::Stop),
                    },
                ]);
            }
            "error" => {
                let mut e = ProviderError::new(
                    &self.provider,
                    error_kind(str_at(&v, "/error/type")),
                    error_message(&v).unwrap_or_else(|| "stream error".into()),
                );
                if str_at(&v, "/error/type") == Some("overloaded_error") {
                    e.kind = ProviderErrorKind::Unavailable;
                }
                return Err(e);
            }
            _ => return Ok(Vec::new()),
        };
        Ok(vec![ev])
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done {
            Ok(Vec::new())
        } else {
            Err(super::protocol(
                &self.provider,
                "stream ended before message_stop",
            ))
        }
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn preconnect(&self) {
        self.http.preconnect();
    }

    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let req = self
            .http
            .request(reqwest::Method::GET, "/models?limit=1000")
            .header(VERSION.0, VERSION.1);
        let v = self.http.read_json(self.http.send(req).await?).await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m["id"].as_str()?.to_owned(),
                    display_name: m["display_name"].as_str().map(str::to_owned),
                    context_window: None,
                    max_output: None,
                    size_bytes: None,
                })
            })
            .collect())
    }

    async fn complete(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let body = body(&req, self.http.provider())?;
        let resp = self.http.post_json("/messages", body, &[VERSION]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Sse,
            MessagesMapper::new(self.http.provider()),
            cancel,
        ))
    }

    async fn count_tokens(&self, req: &ChatRequest) -> Result<Option<u64>, ProviderError> {
        let mut b = body(req, self.http.provider())?;
        if let Some(o) = b.as_object_mut() {
            o.remove("stream");
            o.remove("max_tokens");
            o.remove("temperature");
        }
        let resp = self
            .http
            .post_json("/messages/count_tokens", b, &[VERSION])
            .await?;
        let v = self.http.read_json(resp).await?;
        Ok(v["input_tokens"].as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_tool_uses_the_server_tool_shape() {
        let spec = kor_core::ToolSpec {
            name: "WebSearch".into(),
            description: "search".into(),
            input_schema: json!({"type": "object"}),
        };
        let body = tool_body(&spec);
        assert_eq!(body["type"], "web_search_20250305");
        assert_eq!(body["name"], "web_search");
        assert_eq!(body["max_uses"], 8);
        let other = tool_body(&kor_core::ToolSpec {
            name: "Read".into(),
            description: "read".into(),
            input_schema: json!({"type": "object"}),
        });
        assert!(other.get("type").is_none());
        assert_eq!(other["name"], "Read");
    }
}
