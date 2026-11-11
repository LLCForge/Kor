//! Ollama native API (`/api/chat`, NDJSON stream).

use async_trait::async_trait;
use kor_core::{
    ContentPart, FinishReason, ImageSource, ProviderError, ProviderErrorKind, Role, StreamEvent,
    Usage,
};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::{parse, str_at, system_text, tool_names, turns, u64_at, unsupported};
use crate::http::Http;
use crate::stream::{self, Frame, Framing, Mapper};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

#[derive(Debug)]
pub(crate) struct Ollama {
    manifest: ProviderManifest,
    http: Http,
}

impl Ollama {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let names = tool_names(req);
    let mut messages = Vec::new();
    if let Some(s) = system_text(req) {
        messages.push(json!({"role": "system", "content": s}));
    }
    for m in turns(req) {
        let mut text = String::new();
        let mut images = Vec::new();
        let mut calls = Vec::new();
        for p in &m.parts {
            match p {
                ContentPart::Text { text: t } => text.push_str(t),
                ContentPart::Image {
                    source: ImageSource::Base64 { data, .. },
                } => {
                    images.push(Value::String(data.clone()));
                }
                ContentPart::Image {
                    source: ImageSource::Url { .. },
                } => {
                    return Err(unsupported(provider, "image by URL"));
                }
                ContentPart::ToolUse { name, input, .. } => {
                    calls.push(json!({"function": {"name": name, "arguments": input}}));
                }
                ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    let mut msg = json!({"role": "tool", "content": content});
                    if let Some(name) = names.get(tool_use_id.as_str()) {
                        msg["tool_name"] = (*name).into();
                    }
                    messages.push(msg);
                }
                ContentPart::Thinking { .. } => {}
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            }
        }
        if text.is_empty() && images.is_empty() && calls.is_empty() {
            continue;
        }
        let role = if m.role == Role::Assistant {
            "assistant"
        } else {
            "user"
        };
        let mut msg = json!({"role": role, "content": text});
        if !images.is_empty() {
            msg["images"] = Value::Array(images);
        }
        if !calls.is_empty() {
            msg["tool_calls"] = Value::Array(calls);
        }
        messages.push(msg);
    }
    let mut body = json!({"model": req.model, "messages": messages, "stream": true});
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}}))
            .collect();
    }
    let mut options = Map::new();
    if let Some(n) = req.max_tokens {
        options.insert("num_predict".into(), n.into());
    }
    if let Some(t) = req.temperature {
        options.insert("temperature".into(), t.into());
    }
    if !options.is_empty() {
        body["options"] = Value::Object(options);
    }
    Ok(body)
}

#[derive(Debug, Default)]
pub(crate) struct ChatMapper {
    provider: String,
    next_tool: u32,
    done: bool,
}

impl ChatMapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }
}

impl Mapper for ChatMapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = parse(&self.provider, &f.data)?;
        if let Some(e) = v["error"].as_str() {
            return Err(ProviderError::new(
                &self.provider,
                ProviderErrorKind::Unavailable,
                e,
            ));
        }
        let mut out = Vec::new();
        let msg = &v["message"];
        if let Some(t) = str_at(msg, "/thinking").filter(|t| !t.is_empty()) {
            out.push(StreamEvent::ThinkingDelta { text: t.to_owned() });
        }
        if let Some(t) = str_at(msg, "/content").filter(|t| !t.is_empty()) {
            out.push(StreamEvent::TextDelta { text: t.to_owned() });
        }
        for tc in msg["tool_calls"].as_array().into_iter().flatten() {
            let index = self.next_tool;
            self.next_tool += 1;
            out.push(StreamEvent::ToolCallStart {
                index,
                id: str_at(tc, "/id").map_or_else(|| format!("call_{index}"), str::to_owned),
                name: str_at(tc, "/function/name").unwrap_or_default().to_owned(),
            });
            out.push(StreamEvent::ToolCallDelta {
                index,
                json: tc
                    .pointer("/function/arguments")
                    .unwrap_or(&json!({}))
                    .to_string(),
            });
            out.push(StreamEvent::ToolCallEnd { index });
        }
        if v["done"].as_bool() == Some(true) {
            self.done = true;
            let reason = match str_at(&v, "/done_reason") {
                Some("length") => FinishReason::MaxTokens,
                _ if self.next_tool > 0 => FinishReason::ToolUse,
                _ => FinishReason::Stop,
            };
            out.push(StreamEvent::Usage {
                usage: Usage {
                    input: u64_at(&v, "/prompt_eval_count"),
                    output: u64_at(&v, "/eval_count"),
                    ..Usage::default()
                },
            });
            out.push(StreamEvent::Finish { reason });
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done {
            Ok(Vec::new())
        } else {
            Err(super::protocol(
                &self.provider,
                "stream ended before done=true",
            ))
        }
    }
}

#[async_trait]
impl Provider for Ollama {
    fn preconnect(&self) {
        self.http.preconnect();
    }

    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let path = self
            .manifest
            .models_endpoint
            .as_deref()
            .unwrap_or("/api/tags");
        let v = self.http.get_json(path).await?;
        Ok(v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m["name"].as_str()?.to_owned(),
                    display_name: None,
                    context_window: None,
                    max_output: None,
                    size_bytes: m["size"].as_u64(),
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
        let resp = self.http.post_json("/api/chat", body, &[]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Ndjson,
            ChatMapper::new(self.http.provider()),
            cancel,
        ))
    }
}
