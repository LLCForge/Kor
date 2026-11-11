//! `OpenAI` Chat Completions protocol, spoken by most third-party providers.

use std::collections::BTreeSet;

use async_trait::async_trait;
use kor_core::{
    ContentPart, FinishReason, ProviderError, ProviderErrorKind, Role, StreamEvent, Usage,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{data_url, parse, str_at, system_text, turns, u64_at, unsupported};
use crate::http::{Http, error_message};
use crate::stream::{self, Frame, Framing, Mapper};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

#[derive(Debug)]
pub(crate) struct OpenAiCompat {
    manifest: ProviderManifest,
    http: Http,
}

impl OpenAiCompat {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut messages = Vec::new();
    if let Some(s) = system_text(req) {
        messages.push(json!({"role": "system", "content": s}));
    }
    for m in turns(req) {
        let mut content = Vec::new();
        let mut tool_calls = Vec::new();
        for p in &m.parts {
            match p {
                ContentPart::Text { text } => content.push(json!({"type": "text", "text": text})),
                ContentPart::Image { source } => content
                    .push(json!({"type": "image_url", "image_url": {"url": data_url(source)}})),
                ContentPart::ToolUse { id, name, input } => tool_calls.push(json!({
                    "id": id, "type": "function",
                    "function": {"name": name, "arguments": input.to_string()},
                })),
                ContentPart::ToolResult {
                    tool_use_id,
                    content: out,
                    ..
                } => messages
                    .push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": out})),
                ContentPart::Thinking { .. } => {}
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            }
        }
        let role = match m.role {
            Role::Assistant => "assistant",
            _ => "user",
        };
        if content.is_empty() && tool_calls.is_empty() {
            continue;
        }
        // Plain string content is the most widely supported shape.
        let all_text = content.iter().all(|c| c["type"] == "text");
        let content = if all_text {
            Value::String(
                content
                    .iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(""),
            )
        } else {
            Value::Array(content)
        };
        let mut msg = json!({"role": role, "content": content});
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        messages.push(msg);
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}}))
            .collect();
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = n.into();
    }
    if let Some(t) = req.temperature {
        body["temperature"] = t.into();
    }
    // Chat Completions `reasoning_effort` (none..xhigh, model-dependent); a budget maps to the
    // level covering it (`ReasoningEffort::from_budget`).
    if let Some(e) = req.reasoning.and_then(kor_core::Reasoning::level) {
        body["reasoning_effort"] = e.as_str().into();
    }
    Ok(body)
}

#[derive(Debug, Default)]
pub(crate) struct ChatMapper {
    provider: String,
    open: BTreeSet<u32>,
    finish: Option<FinishReason>,
    /// Latest usage. Some providers repeat a running total in every chunk, so it is emitted once,
    /// at the end, instead of per chunk (which would add the prompt up once per chunk).
    usage: Option<Usage>,
    /// The provider's own `usage` object when it carries a `cost` (`OpenRouter` usage
    /// accounting), passed on untouched as [`StreamEvent::Raw`] for the cost ledger.
    cost_usage: Option<serde_json::Value>,
}

impl ChatMapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }

    fn close_all(&mut self, out: &mut Vec<StreamEvent>) {
        out.extend(
            std::mem::take(&mut self.open)
                .into_iter()
                .map(|index| StreamEvent::ToolCallEnd { index }),
        );
    }
}

impl Mapper for ChatMapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        if f.data.trim() == "[DONE]" {
            return Ok(Vec::new());
        }
        let v = parse(&self.provider, &f.data)?;
        if v.get("error").is_some() {
            let msg = error_message(&v).unwrap_or_else(|| "stream error".into());
            return Err(ProviderError::new(
                &self.provider,
                ProviderErrorKind::Unavailable,
                msg,
            ));
        }
        let mut out = Vec::new();
        if let Some(choice) = v.pointer("/choices/0") {
            let delta = &choice["delta"];
            if let Some(t) = str_at(delta, "/content").filter(|t| !t.is_empty()) {
                out.push(StreamEvent::TextDelta { text: t.to_owned() });
            }
            let thinking =
                str_at(delta, "/reasoning_content").or_else(|| str_at(delta, "/reasoning"));
            if let Some(t) = thinking.filter(|t| !t.is_empty()) {
                out.push(StreamEvent::ThinkingDelta { text: t.to_owned() });
            }
            for tc in delta["tool_calls"].as_array().into_iter().flatten() {
                let index = u32::try_from(u64_at(tc, "/index")).unwrap_or(0);
                if !self.open.contains(&index) {
                    self.open.insert(index);
                    out.push(StreamEvent::ToolCallStart {
                        index,
                        id: str_at(tc, "/id").unwrap_or_default().to_owned(),
                        name: str_at(tc, "/function/name").unwrap_or_default().to_owned(),
                    });
                }
                if let Some(a) = str_at(tc, "/function/arguments").filter(|a| !a.is_empty()) {
                    out.push(StreamEvent::ToolCallDelta {
                        index,
                        json: a.to_owned(),
                    });
                }
            }
            if let Some(r) = str_at(choice, "/finish_reason") {
                self.close_all(&mut out);
                self.finish = Some(match r {
                    "length" => FinishReason::MaxTokens,
                    "tool_calls" | "function_call" => FinishReason::ToolUse,
                    "content_filter" => FinishReason::ContentFiltered,
                    _ => FinishReason::Stop,
                });
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            if u.get("cost").is_some_and(serde_json::Value::is_number) {
                self.cost_usage = Some(u.clone());
            }
            self.usage = Some(Usage {
                input: u64_at(u, "/prompt_tokens"),
                output: u64_at(u, "/completion_tokens"),
                cache_read: u64_at(u, "/prompt_tokens_details/cached_tokens")
                    .max(u64_at(u, "/prompt_cache_hit_tokens")),
                cache_write: 0,
                reasoning: u64_at(u, "/completion_tokens_details/reasoning_tokens"),
            });
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        let mut out = Vec::new();
        self.close_all(&mut out);
        let reason = self
            .finish
            .ok_or_else(|| super::protocol(&self.provider, "stream ended without finish_reason"))?;
        // The cost comes before the token counts so a consumer can pair them.
        if let Some(value) = self.cost_usage.take() {
            out.push(StreamEvent::Raw { value });
        }
        if let Some(usage) = self.usage.take() {
            out.push(StreamEvent::Usage { usage });
        }
        out.push(StreamEvent::Finish { reason });
        Ok(out)
    }
}

/// `{"data": [...]}` (`OpenAI`) or a bare array (Together, some local servers).
pub(crate) fn models(v: &Value) -> Vec<ModelInfo> {
    v.as_array()
        .or_else(|| v["data"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["id"].as_str()?.to_owned(),
                display_name: m["name"].as_str().map(str::to_owned),
                context_window: m["context_length"]
                    .as_u64()
                    .or_else(|| m["context_window"].as_u64())
                    .or_else(|| {
                        m.pointer("/model_spec/availableContextTokens")
                            .and_then(Value::as_u64)
                    }),
                max_output: None,
            })
        })
        .collect()
}

#[async_trait]
impl Provider for OpenAiCompat {
    fn preconnect(&self) {
        self.http.preconnect();
    }

    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let m = &self.manifest;
        if m.models_endpoint.is_none() && !m.static_models.is_empty() {
            return Ok(super::static_models(m));
        }
        let path = m.models_endpoint.as_deref().unwrap_or("/models");
        Ok(models(&self.http.get_json(path).await?))
    }

    async fn complete(
        &self,
        mut req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        if let Some(wire) = self.manifest.quirks.model_map.get(&req.model) {
            req.model.clone_from(wire);
        }
        let body = body(&req, self.http.provider())?;
        let resp = self.http.post_json("/chat/completions", body, &[]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Sse,
            ChatMapper::new(self.http.provider()),
            cancel,
        ))
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::stream::{Frame, Mapper};

    fn frame(data: &str) -> Frame {
        Frame {
            event: None,
            data: data.to_owned(),
        }
    }

    /// Venice repeats a running usage total in every chunk; it must be counted once, not summed.
    #[test]
    fn repeated_usage_is_reported_once_at_the_end() {
        let mut m = ChatMapper::new("venice");
        let mut events = Vec::new();
        for (text, out) in [("a", 1), ("b", 2), ("c", 3)] {
            let chunk = format!(
                r#"{{"choices":[{{"delta":{{"content":"{text}"}}}}],"usage":{{"prompt_tokens":20000,"completion_tokens":{out}}}}}"#
            );
            events.extend(m.frame(frame(&chunk)).unwrap());
        }
        events.extend(
            m.frame(frame(
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            ))
            .unwrap(),
        );
        events.extend(m.finish().unwrap());
        let usages: Vec<&Usage> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Usage { usage } => Some(usage),
                _ => None,
            })
            .collect();
        assert_eq!(usages.len(), 1);
        assert_eq!((usages[0].input, usages[0].output), (20_000, 3));
        assert!(!events.iter().any(|e| matches!(e, StreamEvent::Raw { .. })));
    }

    /// `OpenRouter` usage accounting: the reported cost is passed on just before the usage.
    #[test]
    fn provider_cost_is_passed_on_before_usage() {
        let mut m = ChatMapper::new("openrouter");
        let mut events = m
            .frame(frame(
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"cost":0.0042}}"#,
            ))
            .unwrap();
        events.extend(m.finish().unwrap());
        let i = events
            .iter()
            .position(|e| matches!(e, StreamEvent::Raw { value } if value["cost"] == 0.0042))
            .unwrap();
        assert!(matches!(events[i + 1], StreamEvent::Usage { .. }));
    }
}
