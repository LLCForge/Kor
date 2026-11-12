//! `OpenAI` Responses API (`POST /responses`). Requests are sent with `store: false`.

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
pub(crate) struct OpenAi {
    manifest: ProviderManifest,
    http: Http,
}

impl OpenAi {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut input = Vec::new();
    for m in turns(req) {
        let assistant = m.role == Role::Assistant;
        let mut content = Vec::new();
        for p in &m.parts {
            match p {
                ContentPart::Text { text } => content.push(if assistant {
                    json!({"type": "output_text", "text": text})
                } else {
                    json!({"type": "input_text", "text": text})
                }),
                ContentPart::Image { source } => {
                    content.push(json!({"type": "input_image", "image_url": data_url(source)}));
                }
                ContentPart::ToolUse { id, name, input: args } => input.push(json!({
                    "type": "function_call", "call_id": id, "name": name, "arguments": args.to_string(),
                })),
                ContentPart::ToolResult {
                    tool_use_id,
                    content: out,
                    ..
                } => input.push(json!({"type": "function_call_output", "call_id": tool_use_id, "output": out})),
                ContentPart::Thinking { .. } => {}
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            }
        }
        if !content.is_empty() {
            let role = if assistant { "assistant" } else { "user" };
            input.push(json!({"role": role, "content": content}));
        }
    }
    let mut body = json!({"model": req.model, "input": input, "stream": true, "store": false});
    if let Some(s) = system_text(req) {
        body["instructions"] = s.into();
    }
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.input_schema}))
            .collect();
    }
    if let Some(n) = req.max_tokens {
        body["max_output_tokens"] = n.into();
    }
    if let Some(t) = req.temperature {
        body["temperature"] = t.into();
    }
    // Responses `reasoning.effort`; a budget maps to the level covering it.
    if let Some(e) = req.reasoning.and_then(kor_core::Reasoning::level) {
        body["reasoning"] = json!({"effort": e.as_str()});
    }
    if let Some(s) = crate::response_schema() {
        body["text"] = json!({"format": {"type": "json_schema", "name": s.name, "schema": s.schema, "strict": s.strict}});
    }
    Ok(body)
}

#[derive(Debug, Default)]
pub(crate) struct ResponsesMapper {
    provider: String,
    saw_tool: bool,
    done: bool,
}

impl ResponsesMapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }
}

fn index(v: &Value) -> u32 {
    u32::try_from(u64_at(v, "/output_index")).unwrap_or(0)
}

impl Mapper for ResponsesMapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = parse(&self.provider, &f.data)?;
        let kind = str_at(&v, "/type")
            .or(f.event.as_deref())
            .unwrap_or_default();
        let text = |k: &str| str_at(&v, k).unwrap_or_default().to_owned();
        let ev = match kind {
            "response.output_text.delta" => StreamEvent::TextDelta {
                text: text("/delta"),
            },
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                StreamEvent::ThinkingDelta {
                    text: text("/delta"),
                }
            }
            "response.output_item.added" if str_at(&v, "/item/type") == Some("function_call") => {
                self.saw_tool = true;
                StreamEvent::ToolCallStart {
                    index: index(&v),
                    id: text("/item/call_id"),
                    name: text("/item/name"),
                }
            }
            "response.function_call_arguments.delta" => StreamEvent::ToolCallDelta {
                index: index(&v),
                json: text("/delta"),
            },
            "response.output_item.done" if str_at(&v, "/item/type") == Some("function_call") => {
                StreamEvent::ToolCallEnd { index: index(&v) }
            }
            "response.completed" | "response.incomplete" => {
                self.done = true;
                let r = &v["response"];
                let reason = match str_at(r, "/incomplete_details/reason") {
                    Some("max_output_tokens") => FinishReason::MaxTokens,
                    Some("content_filter") => FinishReason::ContentFiltered,
                    _ if self.saw_tool => FinishReason::ToolUse,
                    _ => FinishReason::Stop,
                };
                let u = &r["usage"];
                return Ok(vec![
                    StreamEvent::Usage {
                        usage: Usage {
                            input: u64_at(u, "/input_tokens"),
                            output: u64_at(u, "/output_tokens"),
                            cache_read: u64_at(u, "/input_tokens_details/cached_tokens"),
                            cache_write: 0,
                            reasoning: u64_at(u, "/output_tokens_details/reasoning_tokens"),
                        },
                    },
                    StreamEvent::Finish { reason },
                ]);
            }
            "response.failed" | "error" => {
                let msg = error_message(v.get("response").unwrap_or(&v))
                    .or_else(|| error_message(&v))
                    .unwrap_or_else(|| "response failed".into());
                return Err(ProviderError::new(
                    &self.provider,
                    ProviderErrorKind::Unavailable,
                    msg,
                ));
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
                "stream ended before response.completed",
            ))
        }
    }
}

#[async_trait]
impl Provider for OpenAi {
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
            .unwrap_or("/models");
        Ok(super::openai_compat::models(
            &self.http.get_json(path).await?,
        ))
    }

    async fn complete(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let body = body(&req, self.http.provider())?;
        let resp = self.http.post_json("/responses", body, &[]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Sse,
            ResponsesMapper::new(self.http.provider()),
            cancel,
        ))
    }
}
