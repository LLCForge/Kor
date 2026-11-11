//! Cohere Chat v2 (`POST /v2/chat`, streaming), <https://docs.cohere.com/v2/reference/chat-stream>.
//! Messages and tools share the `OpenAI` shape; the stream uses Cohere's own event types.

use std::collections::BTreeSet;

use async_trait::async_trait;
use kor_core::{FinishReason, ProviderError, ProviderErrorKind, StreamEvent, Usage};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{parse, str_at, u64_at};
use crate::http::{Http, error_message};
use crate::stream::{self, Frame, Framing, Mapper};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

#[derive(Debug)]
pub(crate) struct Cohere {
    manifest: ProviderManifest,
    http: Http,
}

impl Cohere {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut b = super::openai_compat::body(req, provider)?;
    if let Some(o) = b.as_object_mut() {
        o.remove("stream_options");
    }
    // An assistant turn that only calls tools carries no content in v2.
    for m in b["messages"].as_array_mut().into_iter().flatten() {
        if m.get("tool_calls").is_some()
            && m["content"] == ""
            && let Some(o) = m.as_object_mut()
        {
            o.remove("content");
        }
    }
    Ok(b)
}

#[derive(Debug, Default)]
pub(crate) struct ChatV2Mapper {
    provider: String,
    open: BTreeSet<u32>,
    finish: Option<FinishReason>,
}

impl ChatV2Mapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }
}

fn index(v: &Value) -> u32 {
    u32::try_from(u64_at(v, "/index")).unwrap_or(0)
}

impl Mapper for ChatV2Mapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = parse(&self.provider, &f.data)?;
        let kind = str_at(&v, "/type")
            .or(f.event.as_deref())
            .unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "content-delta" => {
                if let Some(t) = str_at(&v, "/delta/message/content/text").filter(|t| !t.is_empty())
                {
                    out.push(StreamEvent::TextDelta { text: t.to_owned() });
                }
            }
            "tool-plan-delta" => {
                if let Some(t) = str_at(&v, "/delta/message/tool_plan").filter(|t| !t.is_empty()) {
                    out.push(StreamEvent::ThinkingDelta { text: t.to_owned() });
                }
            }
            "tool-call-start" => {
                let i = index(&v);
                let tc = &v["delta"]["message"]["tool_calls"];
                self.open.insert(i);
                out.push(StreamEvent::ToolCallStart {
                    index: i,
                    id: str_at(tc, "/id").unwrap_or_default().to_owned(),
                    name: str_at(tc, "/function/name").unwrap_or_default().to_owned(),
                });
                if let Some(a) = str_at(tc, "/function/arguments").filter(|a| !a.is_empty()) {
                    out.push(StreamEvent::ToolCallDelta {
                        index: i,
                        json: a.to_owned(),
                    });
                }
            }
            "tool-call-delta" => {
                if let Some(a) = str_at(&v, "/delta/message/tool_calls/function/arguments")
                    .filter(|a| !a.is_empty())
                {
                    out.push(StreamEvent::ToolCallDelta {
                        index: index(&v),
                        json: a.to_owned(),
                    });
                }
            }
            "tool-call-end" => {
                let i = index(&v);
                if self.open.remove(&i) {
                    out.push(StreamEvent::ToolCallEnd { index: i });
                }
            }
            "message-end" => {
                out.extend(
                    std::mem::take(&mut self.open)
                        .into_iter()
                        .map(|index| StreamEvent::ToolCallEnd { index }),
                );
                let reason = str_at(&v, "/delta/finish_reason").unwrap_or("COMPLETE");
                if matches!(reason, "ERROR" | "TIMEOUT") {
                    let msg = error_message(&v["delta"]).unwrap_or_else(|| reason.to_lowercase());
                    let kind = if reason == "TIMEOUT" {
                        ProviderErrorKind::Timeout
                    } else {
                        ProviderErrorKind::Unavailable
                    };
                    return Err(ProviderError::new(&self.provider, kind, msg));
                }
                let u = &v["delta"]["usage"];
                let tokens = if u["tokens"].is_object() {
                    &u["tokens"]
                } else {
                    &u["billed_units"]
                };
                out.push(StreamEvent::Usage {
                    usage: Usage {
                        input: u64_at(tokens, "/input_tokens"),
                        output: u64_at(tokens, "/output_tokens"),
                        cache_read: u64_at(u, "/cached_tokens"),
                        ..Usage::default()
                    },
                });
                let reason = match reason {
                    "MAX_TOKENS" => FinishReason::MaxTokens,
                    "TOOL_CALL" => FinishReason::ToolUse,
                    _ => FinishReason::Stop,
                };
                self.finish = Some(reason);
                out.push(StreamEvent::Finish { reason });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.finish.is_some() {
            Ok(Vec::new())
        } else {
            Err(super::protocol(
                &self.provider,
                "stream ended before message-end",
            ))
        }
    }
}

#[async_trait]
impl Provider for Cohere {
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
            .unwrap_or("/v1/models?endpoint=chat");
        let v = self.http.get_json(path).await?;
        Ok(v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m["name"].as_str()?.to_owned(),
                    display_name: None,
                    context_window: m["context_length"].as_u64(),
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
        let resp = self.http.post_json("/v2/chat", body, &[]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Sse,
            ChatV2Mapper::new(self.http.provider()),
            cancel,
        ))
    }
}
