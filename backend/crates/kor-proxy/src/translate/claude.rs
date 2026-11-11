//! Anthropic Messages (`POST /v1/messages`), port of the `claude` request side and the
//! `*_claude_response` translators: thinking blocks with signatures, `tool_use` /
//! `tool_result` blocks, and the `message_start … message_stop` SSE sequence.

use kor_core::{
    ContentPart, FinishReason, ImageSource, Message, ProviderError, Role, StreamEvent, ToolSpec,
    Usage,
};
use kor_providers::ChatRequest;
use serde_json::{Value, json};

use super::{
    Collected, Encoder, Inbound, ParseResult, args_value, new_id, opt_f32, opt_u32, sse_event,
    text_of,
};

pub(crate) fn stop_str(r: FinishReason) -> &'static str {
    match r {
        FinishReason::MaxTokens => "max_tokens",
        FinishReason::ToolUse => "tool_use",
        FinishReason::ContentFiltered => "refusal",
        FinishReason::Stop | FinishReason::Cancelled | FinishReason::Error => "end_turn",
    }
}

fn image(b: &Value) -> Result<ContentPart, String> {
    let src = b.get("source").ok_or("image block without source")?;
    let source = match src.get("type").and_then(Value::as_str) {
        Some("base64") => ImageSource::Base64 {
            media_type: src
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png")
                .to_owned(),
            data: src
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        Some("url") => ImageSource::Url {
            url: src
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        other => {
            return Err(format!(
                "image source `{}` is not supported",
                other.unwrap_or("?")
            ));
        }
    };
    Ok(ContentPart::Image { source })
}

fn parts(content: &Value) -> Result<Vec<ContentPart>, String> {
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
        let s = |k: &str| {
            b.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match b.get("type").and_then(Value::as_str).unwrap_or("text") {
            "text" => out.push(ContentPart::Text { text: s("text") }),
            "image" => out.push(image(b)?),
            "tool_use" | "server_tool_use" => out.push(ContentPart::ToolUse {
                id: s("id"),
                name: s("name"),
                input: b.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            "tool_result" => out.push(ContentPart::ToolResult {
                tool_use_id: s("tool_use_id"),
                content: text_of(b.get("content").unwrap_or(&Value::Null)),
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            }),
            "thinking" => out.push(ContentPart::Thinking {
                text: s("thinking"),
                signature: b
                    .get("signature")
                    .and_then(Value::as_str)
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned),
            }),
            // Opaque to every other upstream; dropping it is what the original does too.
            "redacted_thinking" => {}
            other => return Err(format!("content block `{other}` is not supported")),
        }
    }
    Ok(out)
}

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
    let mut messages = Vec::with_capacity(msgs.len());
    for m in msgs {
        let role = match m.get("role").and_then(Value::as_str) {
            Some("assistant") => Role::Assistant,
            Some("user") => Role::User,
            other => return Err(format!("role `{}` is not supported", other.unwrap_or("?"))),
        };
        messages.push(Message {
            role,
            parts: parts(m.get("content").unwrap_or(&Value::Null))?,
        });
    }
    let system = body.get("system").map(text_of).filter(|s| !s.is_empty());
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        // Server tools (web search, code execution) have no schema and no portable meaning.
        .filter(|t| t.get("input_schema").is_some())
        .filter_map(|t| {
            Some(ToolSpec {
                name: t.get("name")?.as_str()?.to_owned(),
                description: t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: t.get("input_schema").cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect();
    Ok(Inbound {
        req: ChatRequest {
            model: model.clone(),
            system,
            messages,
            tools,
            max_tokens: opt_u32(body, "max_tokens"),
            temperature: opt_f32(body, "temperature"),
            reasoning: None,
        },
        model,
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn usage_json(u: &Usage) -> Value {
    json!({
        "input_tokens": u.input,
        "output_tokens": u.output,
        "cache_read_input_tokens": u.cache_read,
        "cache_creation_input_tokens": u.cache_write,
    })
}

pub fn complete(model: &str, c: &Collected) -> Value {
    let mut content = Vec::new();
    if !c.thinking.is_empty() || c.signature.is_some() {
        content.push(json!({"type": "thinking", "thinking": c.thinking,
                            "signature": c.signature.clone().unwrap_or_default()}));
    }
    if !c.text.is_empty() {
        content.push(json!({"type": "text", "text": c.text}));
    }
    for t in &c.tools {
        content.push(
            json!({"type": "tool_use", "id": t.id, "name": t.name, "input": args_value(&t.args)}),
        );
    }
    json!({
        "id": new_id("msg_"),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_str(c.finish_reason()),
        "stop_sequence": null,
        "usage": usage_json(&c.usage),
    })
}

pub fn error_json(status: u16, message: &str) -> Value {
    let kind = match status {
        400 | 413 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        529 | 503 => "overloaded_error",
        _ => "api_error",
    };
    json!({"type": "error", "error": {"type": kind, "message": message}})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    None,
    Text,
    Thinking,
    Tool(u32),
}

#[derive(Debug)]
pub struct Stream {
    id: String,
    model: String,
    started: bool,
    index: u32,
    open: Open,
    tools: bool,
    usage: Usage,
    finish: Option<FinishReason>,
}

impl Stream {
    pub fn new(model: &str) -> Self {
        Self {
            id: new_id("msg_"),
            model: model.to_owned(),
            started: false,
            index: 0,
            open: Open::None,
            tools: false,
            usage: Usage::default(),
            finish: None,
        }
    }

    fn start(&mut self, out: &mut Vec<String>) {
        if !self.started {
            self.started = true;
            out.push(sse_event(
                "message_start",
                &json!({"type": "message_start", "message": {
                    "id": self.id, "type": "message", "role": "assistant", "model": self.model,
                    "content": [], "stop_reason": null, "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0}}}),
            ));
        }
    }

    fn close(&mut self, out: &mut Vec<String>) {
        if self.open != Open::None {
            out.push(sse_event(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": self.index}),
            ));
            self.index += 1;
            self.open = Open::None;
        }
    }

    fn open(&mut self, kind: Open, block: Value, out: &mut Vec<String>) {
        if self.open == kind {
            return;
        }
        self.close(out);
        self.open = kind;
        let mut ev = json!({"type": "content_block_start", "index": self.index});
        ev["content_block"] = block;
        out.push(sse_event("content_block_start", &ev));
    }

    fn delta(&self, delta: Value) -> String {
        let mut ev = json!({"type": "content_block_delta", "index": self.index});
        ev["delta"] = delta;
        sse_event("content_block_delta", &ev)
    }
}

impl Encoder for Stream {
    fn event(&mut self, ev: &StreamEvent) -> Vec<String> {
        let mut out = Vec::new();
        self.start(&mut out);
        match ev {
            StreamEvent::TextDelta { text } => {
                self.open(Open::Text, json!({"type": "text", "text": ""}), &mut out);
                out.push(self.delta(json!({"type": "text_delta", "text": text})));
            }
            StreamEvent::ThinkingDelta { text } => {
                self.open(
                    Open::Thinking,
                    json!({"type": "thinking", "thinking": "", "signature": ""}),
                    &mut out,
                );
                out.push(self.delta(json!({"type": "thinking_delta", "thinking": text})));
            }
            StreamEvent::ThinkingSignature { signature } => {
                self.open(
                    Open::Thinking,
                    json!({"type": "thinking", "thinking": "", "signature": ""}),
                    &mut out,
                );
                out.push(self.delta(json!({"type": "signature_delta", "signature": signature})));
            }
            StreamEvent::ToolCallStart { index, id, name } => {
                self.tools = true;
                self.close(&mut out);
                self.open(
                    Open::Tool(*index),
                    json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
                    &mut out,
                );
            }
            StreamEvent::ToolCallDelta { json: part, .. } => {
                out.push(self.delta(json!({"type": "input_json_delta", "partial_json": part})));
            }
            StreamEvent::ToolCallEnd { index } => {
                if self.open == Open::Tool(*index) {
                    self.close(&mut out);
                }
            }
            StreamEvent::Usage { usage } => self.usage += *usage,
            StreamEvent::Finish { reason } => self.finish = Some(*reason),
            StreamEvent::Citation { .. } | StreamEvent::Raw { .. } => {}
        }
        out
    }

    fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.start(&mut out);
        self.close(&mut out);
        let reason = match self.finish {
            Some(FinishReason::Stop) | None if self.tools => FinishReason::ToolUse,
            Some(r) => r,
            None => FinishReason::Stop,
        };
        out.push(sse_event(
            "message_delta",
            &json!({"type": "message_delta",
                    "delta": {"stop_reason": stop_str(reason), "stop_sequence": null},
                    "usage": usage_json(&self.usage)}),
        ));
        out.push(sse_event("message_stop", &json!({"type": "message_stop"})));
        out
    }

    fn error(&mut self, e: &ProviderError) -> Vec<String> {
        vec![sse_event(
            "error",
            &error_json(super::status_of(e), &e.message),
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks_and_skips_server_tools() {
        let b = json!({
            "model": "claude-x", "max_tokens": 100, "stream": true,
            "system": [{"type": "text", "text": "sys"}],
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "t", "signature": "sig"},
                    {"type": "tool_use", "id": "tu", "name": "f", "input": {"a": 1}}]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "tu", "content": [{"type": "text", "text": "r"}], "is_error": true}]}
            ],
            "tools": [{"name": "f", "input_schema": {"type": "object"}},
                      {"type": "web_search_20250305", "name": "web_search"}]
        });
        let p = parse(&b).unwrap();
        assert_eq!(p.req.system.as_deref(), Some("sys"));
        assert_eq!(p.req.tools.len(), 1);
        assert!(
            matches!(&p.req.messages[1].parts[0], ContentPart::Thinking { signature: Some(s), .. } if s == "sig")
        );
        assert!(matches!(&p.req.messages[2].parts[0],
            ContentPart::ToolResult { content, is_error: true, .. } if content == "r"));
        assert!(
            parse(&json!({"model": "m", "messages": [{"role": "system", "content": "x"}]}))
                .is_err()
        );
    }

    #[test]
    fn stream_sequence_matches_the_messages_api() {
        let mut s = Stream::new("m");
        let mut out = Vec::new();
        for ev in [
            StreamEvent::ThinkingDelta { text: "a".into() },
            StreamEvent::ThinkingSignature {
                signature: "sig".into(),
            },
            StreamEvent::TextDelta { text: "b".into() },
            StreamEvent::ToolCallStart {
                index: 0,
                id: "t".into(),
                name: "f".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: "{}".into(),
            },
            StreamEvent::ToolCallEnd { index: 0 },
            StreamEvent::Usage {
                usage: Usage {
                    input: 3,
                    output: 4,
                    ..Usage::default()
                },
            },
        ] {
            out.extend(s.event(&ev));
        }
        out.extend(s.finish());
        let names: Vec<&str> = out
            .iter()
            .map(|f| f.lines().next().unwrap().trim_start_matches("event: "))
            .collect();
        assert_eq!(
            names,
            [
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert!(
            out[11].contains("\"stop_reason\":\"tool_use\"")
                && out[11].contains("\"output_tokens\":4")
        );
        assert!(out[8].contains("\"index\":2"));
    }
}
