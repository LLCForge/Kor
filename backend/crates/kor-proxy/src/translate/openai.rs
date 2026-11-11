//! OpenAI Chat Completions (`POST /v1/chat/completions`), port of the `openai/chat-completions`
//! request side and the `*_openai_response` translators: `reasoning_content` carries thinking,
//! tool calls stream as `delta.tool_calls[index]`, usage rides on the final chunk.

use std::collections::BTreeMap;

use kor_core::{
    ContentPart, FinishReason, Message, ProviderError, Role, StreamEvent, ToolSpec, Usage,
};
use kor_providers::ChatRequest;
use serde_json::{Value, json};

use super::{
    Collected, Encoder, Inbound, ParseResult, args_value, image_from_url, new_id, now_secs,
    opt_f32, opt_u32, sse_data, text_of,
};

pub(crate) fn finish_str(r: FinishReason) -> &'static str {
    match r {
        FinishReason::MaxTokens => "length",
        FinishReason::ToolUse => "tool_calls",
        FinishReason::ContentFiltered => "content_filter",
        FinishReason::Stop | FinishReason::Cancelled | FinishReason::Error => "stop",
    }
}

fn user_parts(content: &Value) -> Result<Vec<ContentPart>, String> {
    let Value::Array(blocks) = content else {
        let text = text_of(content);
        return Ok(if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentPart::Text { text }]
        });
    };
    let mut out = Vec::new();
    for b in blocks {
        match b.get("type").and_then(Value::as_str).unwrap_or("text") {
            "text" | "input_text" => {
                let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
                out.push(ContentPart::Text {
                    text: text.to_owned(),
                });
            }
            "image_url" => {
                let url = b
                    .pointer("/image_url/url")
                    .or_else(|| b.get("image_url"))
                    .and_then(Value::as_str)
                    .ok_or("image_url without url")?;
                out.push(ContentPart::Image {
                    source: image_from_url(url),
                });
            }
            other => return Err(format!("content type `{other}` is not supported")),
        }
    }
    Ok(out)
}

pub(crate) fn tools_from(v: Option<&Value>) -> Vec<ToolSpec> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            // Chat shape `{type, function:{…}}`, or the flat Responses shape `{type, name, …}`.
            let f = t.get("function").unwrap_or(t);
            let name = f.get("name")?.as_str()?.to_owned();
            Some(ToolSpec {
                name,
                description: f
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: f
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            })
        })
        .collect()
}

#[allow(clippy::too_many_lines)] // flat field-by-field mapping of the request schema; splitting adds no clarity
pub fn parse(body: &Value) -> ParseResult {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or("`model` is required")?
        .to_owned();
    let msgs = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("`messages` must be an array")?;
    let mut system = Vec::new();
    let mut messages: Vec<Message> = Vec::new();
    for m in msgs {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
        let content = m.get("content").unwrap_or(&Value::Null);
        match role {
            "system" | "developer" => system.push(text_of(content)),
            "user" => messages.push(Message {
                role: Role::User,
                parts: user_parts(content)?,
            }),
            "assistant" => {
                let mut parts = Vec::new();
                if let Some(r) = m.get("reasoning_content").and_then(Value::as_str)
                    && !r.is_empty()
                {
                    parts.push(ContentPart::Thinking {
                        text: r.to_owned(),
                        signature: None,
                    });
                }
                let text = text_of(content);
                if !text.is_empty() {
                    parts.push(ContentPart::Text { text });
                }
                for c in m
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    parts.push(ContentPart::ToolUse {
                        id: c
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: c
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        input: args_value(
                            c.pointer("/function/arguments")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                    });
                }
                messages.push(Message {
                    role: Role::Assistant,
                    parts,
                });
            }
            "tool" | "function" => {
                let part = ContentPart::ToolResult {
                    tool_use_id: m
                        .get("tool_call_id")
                        .or_else(|| m.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    content: text_of(content),
                    is_error: false,
                };
                // Consecutive tool results share one user turn (Claude and Gemini need that).
                match messages.last_mut() {
                    Some(last)
                        if last.role == Role::User
                            && last
                                .parts
                                .iter()
                                .all(|p| matches!(p, ContentPart::ToolResult { .. })) =>
                    {
                        last.parts.push(part);
                    }
                    _ => messages.push(Message {
                        role: Role::User,
                        parts: vec![part],
                    }),
                }
            }
            other => return Err(format!("role `{other}` is not supported")),
        }
    }
    Ok(Inbound {
        req: ChatRequest {
            model: model.clone(),
            system: (!system.is_empty()).then(|| system.join("\n\n")),
            messages,
            tools: tools_from(body.get("tools")),
            max_tokens: opt_u32(body, "max_completion_tokens")
                .or_else(|| opt_u32(body, "max_tokens")),
            temperature: opt_f32(body, "temperature"),
            reasoning: None,
        },
        model,
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn usage_json(u: &Usage) -> Value {
    let mut v = json!({
        "prompt_tokens": u.input,
        "completion_tokens": u.output,
        "total_tokens": u.input.saturating_add(u.output),
    });
    if u.cache_read > 0 {
        v["prompt_tokens_details"] = json!({"cached_tokens": u.cache_read});
    }
    if u.reasoning > 0 {
        v["completion_tokens_details"] = json!({"reasoning_tokens": u.reasoning});
    }
    v
}

pub fn complete(model: &str, c: &Collected) -> Value {
    let mut message = json!({"role": "assistant", "content": c.text});
    if !c.thinking.is_empty() {
        message["reasoning_content"] = c.thinking.clone().into();
    }
    if !c.tools.is_empty() {
        message["tool_calls"] = c
            .tools
            .iter()
            .map(|t| {
                json!({"id": t.id, "type": "function",
                       "function": {"name": t.name, "arguments": if t.args.is_empty() { "{}" } else { &t.args }}})
            })
            .collect();
        if c.text.is_empty() {
            message["content"] = Value::Null;
        }
    }
    json!({
        "id": new_id("chatcmpl-"),
        "object": "chat.completion",
        "created": now_secs(),
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish_str(c.finish_reason())}],
        "usage": usage_json(&c.usage),
    })
}

pub fn error_json(status: u16, message: &str) -> Value {
    let kind = match status {
        401 | 403 => "authentication_error",
        429 => "rate_limit_error",
        400..=499 => "invalid_request_error",
        _ => "server_error",
    };
    json!({"error": {"message": message, "type": kind, "code": status}})
}

#[derive(Debug)]
pub struct Stream {
    id: String,
    created: u64,
    model: String,
    started: bool,
    /// Kor tool index → OpenAI `tool_calls[].index`.
    tools: BTreeMap<u32, usize>,
    usage: Usage,
    finish: Option<FinishReason>,
}

impl Stream {
    pub fn new(model: &str) -> Self {
        Self {
            id: new_id("chatcmpl-"),
            created: now_secs(),
            model: model.to_owned(),
            started: false,
            tools: BTreeMap::new(),
            usage: Usage::default(),
            finish: None,
        }
    }

    fn chunk(&mut self, mut delta: Value, finish: Option<&str>) -> String {
        if !self.started {
            self.started = true;
            delta["role"] = "assistant".into();
        }
        sse_data(&json!({
            "id": self.id, "object": "chat.completion.chunk", "created": self.created,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        }))
    }
}

impl Encoder for Stream {
    fn event(&mut self, ev: &StreamEvent) -> Vec<String> {
        let delta = match ev {
            StreamEvent::TextDelta { text } => json!({"content": text}),
            StreamEvent::ThinkingDelta { text } => json!({"reasoning_content": text}),
            StreamEvent::ToolCallStart { index, id, name } => {
                let n = self.tools.len();
                self.tools.insert(*index, n);
                json!({"tool_calls": [{"index": n, "id": id, "type": "function",
                                        "function": {"name": name, "arguments": ""}}]})
            }
            StreamEvent::ToolCallDelta { index, json: part } => {
                let n = self.tools.get(index).copied().unwrap_or(0);
                json!({"tool_calls": [{"index": n, "function": {"arguments": part}}]})
            }
            StreamEvent::Usage { usage } => {
                self.usage += *usage;
                return Vec::new();
            }
            StreamEvent::Finish { reason } => {
                self.finish = Some(*reason);
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        vec![self.chunk(delta, None)]
    }

    fn finish(&mut self) -> Vec<String> {
        let reason = match self.finish {
            Some(FinishReason::Stop) | None if !self.tools.is_empty() => FinishReason::ToolUse,
            Some(r) => r,
            None => FinishReason::Stop,
        };
        let mut last = self.chunk(json!({}), Some(finish_str(reason)));
        // Usage on the closing chunk, as CLIProxyAPI sends it (`stream_options.include_usage`).
        if let Some(body) = last.strip_prefix("data: ").map(str::trim)
            && let Ok(mut v) = serde_json::from_str::<Value>(body)
        {
            v["usage"] = usage_json(&self.usage);
            last = sse_data(&v);
        }
        vec![last, "data: [DONE]\n\n".to_owned()]
    }

    fn error(&mut self, e: &ProviderError) -> Vec<String> {
        vec![
            sse_data(&error_json(super::status_of(e), &e.message)),
            "data: [DONE]\n\n".to_owned(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_tools_images_and_merges_tool_results() {
        let b = json!({
            "model": "gpt-4o", "stream": true, "max_completion_tokens": 50,
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "developer", "content": [{"type": "text", "text": "use tools"}]},
                {"role": "user", "content": [
                    {"type": "text", "text": "look"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AA"}}]},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{\"x\":1}"}},
                    {"id": "c2", "type": "function", "function": {"name": "g", "arguments": ""}}]},
                {"role": "tool", "tool_call_id": "c1", "content": "one"},
                {"role": "tool", "tool_call_id": "c2", "content": "two"}
            ],
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}]
        });
        let p = parse(&b).unwrap();
        assert!(p.stream);
        assert_eq!(p.req.system.as_deref(), Some("be brief\n\nuse tools"));
        assert_eq!(p.req.max_tokens, Some(50));
        assert_eq!(p.req.messages.len(), 3);
        assert_eq!(p.req.messages[2].parts.len(), 2, "tool results merged");
        assert!(
            matches!(&p.req.messages[1].parts[0], ContentPart::ToolUse { input, .. } if input["x"] == 1)
        );
        assert_eq!(p.req.tools[0].name, "f");
        assert!(parse(&json!({"messages": []})).is_err());
        assert!(parse(&json!({"model": "m", "messages": [{"role": "x"}]})).is_err());
    }

    #[test]
    fn stream_emits_role_reasoning_tool_calls_usage_and_done() {
        let mut s = Stream::new("m");
        let mut out = Vec::new();
        for ev in [
            StreamEvent::ThinkingDelta { text: "hmm".into() },
            StreamEvent::TextDelta { text: "hi".into() },
            StreamEvent::ToolCallStart {
                index: 3,
                id: "c".into(),
                name: "f".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 3,
                json: "{}".into(),
            },
            StreamEvent::Usage {
                usage: Usage {
                    input: 4,
                    output: 2,
                    ..Usage::default()
                },
            },
            StreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ] {
            out.extend(s.event(&ev));
        }
        out.extend(s.finish());
        let chunks: Vec<Value> = out
            .iter()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|l| !l.starts_with("[DONE]"))
            .map(|l| serde_json::from_str(l.trim()).unwrap())
            .collect();
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(chunks[0]["choices"][0]["delta"]["reasoning_content"], "hmm");
        assert_eq!(
            chunks[2]["choices"][0]["delta"]["tool_calls"][0]["index"],
            0
        );
        let last = chunks.last().unwrap();
        assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(last["usage"]["total_tokens"], 6);
        assert_eq!(out.last().unwrap(), "data: [DONE]\n\n");
    }
}
