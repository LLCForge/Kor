//! OpenAI Responses (`POST /v1/responses`), as Codex and newer OpenAI SDKs speak it. Port of the
//! `openai/responses` request side and the `*_openai-responses_response` translators: input
//! items, `function_call` / `function_call_output`, reasoning summaries, and the
//! `response.created … response.completed` event sequence with `sequence_number`s.

use kor_core::{ContentPart, FinishReason, Message, ProviderError, Role, StreamEvent, Usage};
use kor_providers::ChatRequest;
use serde_json::{Value, json};

use super::{
    Collected, Encoder, Inbound, ParseResult, image_from_url, new_id, now_secs, opt_f32, opt_u32,
    sse_event, text_of,
};

fn content_parts(content: &Value) -> Result<Vec<ContentPart>, String> {
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
        match b
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("input_text")
        {
            "input_text" | "output_text" | "text" | "summary_text" => out.push(ContentPart::Text {
                text: b
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            "input_image" => {
                let url = b
                    .get("image_url")
                    .and_then(Value::as_str)
                    .ok_or("input_image without image_url (file ids are not supported)")?;
                out.push(ContentPart::Image {
                    source: image_from_url(url),
                });
            }
            "refusal" => {}
            other => return Err(format!("input content `{other}` is not supported")),
        }
    }
    Ok(out)
}

/// Append `part` to the last message when it has `role`, else start a new message.
fn push(messages: &mut Vec<Message>, role: Role, part: ContentPart) {
    match messages.last_mut() {
        Some(m) if m.role == role => m.parts.push(part),
        _ => messages.push(Message {
            role,
            parts: vec![part],
        }),
    }
}

pub fn parse(body: &Value) -> ParseResult {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or("`model` is required")?
        .to_owned();
    let mut system: Vec<String> = body
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .into_iter()
        .collect();
    let mut messages: Vec<Message> = Vec::new();
    match body.get("input") {
        Some(Value::String(s)) => messages.push(Message::text(Role::User, s.clone())),
        Some(Value::Array(items)) => {
            for it in items {
                let ty = it.get("type").and_then(Value::as_str).unwrap_or("message");
                let s = |k: &str| {
                    it.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                match ty {
                    "message" => {
                        let content = it.get("content").unwrap_or(&Value::Null);
                        match it.get("role").and_then(Value::as_str).unwrap_or("user") {
                            "system" | "developer" => system.push(text_of(content)),
                            "assistant" => {
                                for p in content_parts(content)? {
                                    push(&mut messages, Role::Assistant, p);
                                }
                            }
                            _ => messages.push(Message {
                                role: Role::User,
                                parts: content_parts(content)?,
                            }),
                        }
                    }
                    "function_call" | "custom_tool_call" => push(
                        &mut messages,
                        Role::Assistant,
                        ContentPart::ToolUse {
                            id: s("call_id"),
                            name: s("name"),
                            input: super::args_value(
                                it.get("arguments")
                                    .or_else(|| it.get("input"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                            ),
                        },
                    ),
                    "function_call_output" | "custom_tool_call_output" => push(
                        &mut messages,
                        Role::User,
                        ContentPart::ToolResult {
                            tool_use_id: s("call_id"),
                            content: text_of(it.get("output").unwrap_or(&Value::Null)),
                            is_error: false,
                        },
                    ),
                    // Encrypted reasoning only round-trips to the vendor that issued it.
                    "reasoning" | "item_reference" => {}
                    other => return Err(format!("input item `{other}` is not supported")),
                }
            }
        }
        None | Some(Value::Null) => {}
        Some(_) => return Err("`input` must be a string or an array".into()),
    }
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|t| t.get("type").and_then(Value::as_str) == Some("function"))
                .cloned()
                .collect::<Vec<_>>()
        })
        .map(Value::Array);
    Ok(Inbound {
        req: ChatRequest {
            model: model.clone(),
            system: (!system.is_empty()).then(|| system.join("\n\n")),
            messages,
            tools: super::openai::tools_from(tools.as_ref()),
            max_tokens: opt_u32(body, "max_output_tokens"),
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
        "input_tokens_details": {"cached_tokens": u.cache_read},
        "output_tokens": u.output,
        "output_tokens_details": {"reasoning_tokens": u.reasoning},
        "total_tokens": u.input.saturating_add(u.output),
    })
}

fn envelope(
    id: &str,
    model: &str,
    created: u64,
    status: &str,
    output: Vec<Value>,
    usage: Option<&Usage>,
) -> Value {
    let mut v = json!({
        "id": id, "object": "response", "created_at": created, "status": status,
        "model": model, "parallel_tool_calls": true,
    });
    v["output"] = output.into();
    if let Some(u) = usage {
        v["usage"] = usage_json(u);
    }
    if status == "incomplete" {
        v["incomplete_details"] = json!({"reason": "max_output_tokens"});
    }
    v
}

fn status_of(r: FinishReason) -> &'static str {
    if r == FinishReason::MaxTokens {
        "incomplete"
    } else {
        "completed"
    }
}

fn message_item(id: &str, text: &str) -> Value {
    json!({"id": id, "type": "message", "role": "assistant", "status": "completed",
           "content": [{"type": "output_text", "text": text, "annotations": []}]})
}

fn reasoning_item(id: &str, text: &str) -> Value {
    json!({"id": id, "type": "reasoning", "summary": [{"type": "summary_text", "text": text}]})
}

fn call_item(id: &str, call_id: &str, name: &str, args: &str) -> Value {
    json!({"id": id, "type": "function_call", "status": "completed", "call_id": call_id, "name": name,
           "arguments": if args.is_empty() { "{}" } else { args }})
}

pub fn complete(model: &str, c: &Collected) -> Value {
    let mut out = Vec::new();
    if !c.thinking.is_empty() {
        out.push(reasoning_item(&new_id("rs_"), &c.thinking));
    }
    if !c.text.is_empty() {
        out.push(message_item(&new_id("msg_"), &c.text));
    }
    for t in &c.tools {
        out.push(call_item(&new_id("fc_"), &t.id, &t.name, &t.args));
    }
    let mut v = envelope(
        &new_id("resp_"),
        model,
        now_secs(),
        status_of(c.finish_reason()),
        out,
        Some(&c.usage),
    );
    v["output_text"] = c.text.clone().into();
    v
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Reasoning {
        id: String,
        text: String,
    },
    Message {
        id: String,
        text: String,
    },
    Call {
        id: String,
        index: u32,
        call_id: String,
        name: String,
        args: String,
    },
}

impl Item {
    fn done(&self) -> Value {
        match self {
            Self::Reasoning { id, text } => reasoning_item(id, text),
            Self::Message { id, text } => message_item(id, text),
            Self::Call {
                id,
                call_id,
                name,
                args,
                ..
            } => call_item(id, call_id, name, args),
        }
    }
}

#[derive(Debug)]
pub struct Stream {
    id: String,
    model: String,
    created: u64,
    seq: u64,
    started: bool,
    /// Finished items, then the open one (last) while `open` is true.
    items: Vec<Item>,
    open: bool,
    usage: Usage,
    finish: Option<FinishReason>,
}

impl Stream {
    pub fn new(model: &str) -> Self {
        Self {
            id: new_id("resp_"),
            model: model.to_owned(),
            created: now_secs(),
            seq: 0,
            started: false,
            items: Vec::new(),
            open: false,
            usage: Usage::default(),
            finish: None,
        }
    }

    fn ev(&mut self, ty: &str, mut v: Value) -> String {
        v["type"] = ty.into();
        v["sequence_number"] = self.seq.into();
        self.seq += 1;
        sse_event(ty, &v)
    }

    fn start(&mut self, out: &mut Vec<String>) {
        if !self.started {
            self.started = true;
            let r = envelope(
                &self.id,
                &self.model,
                self.created,
                "in_progress",
                Vec::new(),
                None,
            );
            out.push(self.ev("response.created", json!({"response": r})));
            out.push(self.ev("response.in_progress", json!({"response": r})));
        }
    }

    fn close(&mut self, out: &mut Vec<String>) {
        if !self.open {
            return;
        }
        self.open = false;
        let n = self.items.len() - 1;
        let item = self.items[n].clone();
        match &item {
            Item::Reasoning { id, text } => {
                out.push(self.ev(
                    "response.reasoning_summary_text.done",
                    json!({"item_id": id, "output_index": n, "summary_index": 0, "text": text}),
                ));
            }
            Item::Message { id, text } => {
                out.push(self.ev(
                    "response.output_text.done",
                    json!({"item_id": id, "output_index": n, "content_index": 0, "text": text}),
                ));
                out.push(self.ev("response.content_part.done", json!({"item_id": id, "output_index": n,
                    "content_index": 0, "part": {"type": "output_text", "text": text, "annotations": []}})));
            }
            Item::Call { id, args, .. } => {
                out.push(self.ev(
                    "response.function_call_arguments.done",
                    json!({"item_id": id, "output_index": n, "arguments": args}),
                ));
            }
        }
        out.push(self.ev(
            "response.output_item.done",
            json!({"output_index": n, "item": item.done()}),
        ));
    }

    fn open_item(&mut self, item: Item, out: &mut Vec<String>) {
        self.close(out);
        let n = self.items.len();
        let added = match &item {
            Item::Reasoning { id, .. } => json!({"id": id, "type": "reasoning", "summary": []}),
            Item::Message { id, .. } => {
                json!({"id": id, "type": "message", "role": "assistant", "status": "in_progress", "content": []})
            }
            Item::Call {
                id, call_id, name, ..
            } => json!({"id": id, "type": "function_call",
                "status": "in_progress", "call_id": call_id, "name": name, "arguments": ""}),
        };
        out.push(self.ev(
            "response.output_item.added",
            json!({"output_index": n, "item": added}),
        ));
        if let Item::Message { id, .. } = &item {
            out.push(self.ev("response.content_part.added", json!({"item_id": id, "output_index": n,
                "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}})));
        }
        self.items.push(item);
        self.open = true;
    }

    fn current(&mut self) -> Option<&mut Item> {
        if self.open {
            self.items.last_mut()
        } else {
            None
        }
    }
}

impl Encoder for Stream {
    fn event(&mut self, ev: &StreamEvent) -> Vec<String> {
        let mut out = Vec::new();
        self.start(&mut out);
        match ev {
            StreamEvent::TextDelta { text } => {
                if !matches!(self.current(), Some(Item::Message { .. })) {
                    self.open_item(
                        Item::Message {
                            id: new_id("msg_"),
                            text: String::new(),
                        },
                        &mut out,
                    );
                }
                let n = self.items.len() - 1;
                if let Some(Item::Message { id, text: acc }) = self.current() {
                    acc.push_str(text);
                    let id = id.clone();
                    out.push(self.ev("response.output_text.delta",
                        json!({"item_id": id, "output_index": n, "content_index": 0, "delta": text})));
                }
            }
            StreamEvent::ThinkingDelta { text } => {
                if !matches!(self.current(), Some(Item::Reasoning { .. })) {
                    self.open_item(
                        Item::Reasoning {
                            id: new_id("rs_"),
                            text: String::new(),
                        },
                        &mut out,
                    );
                }
                let n = self.items.len() - 1;
                if let Some(Item::Reasoning { id, text: acc }) = self.current() {
                    acc.push_str(text);
                    let id = id.clone();
                    out.push(self.ev("response.reasoning_summary_text.delta",
                        json!({"item_id": id, "output_index": n, "summary_index": 0, "delta": text})));
                }
            }
            StreamEvent::ToolCallStart { index, id, name } => self.open_item(
                Item::Call {
                    id: new_id("fc_"),
                    index: *index,
                    call_id: id.clone(),
                    name: name.clone(),
                    args: String::new(),
                },
                &mut out,
            ),
            StreamEvent::ToolCallDelta { index, json: part } => {
                let n = self.items.len().saturating_sub(1);
                if let Some(Item::Call {
                    id, index: i, args, ..
                }) = self.current()
                    && i == index
                {
                    args.push_str(part);
                    let id = id.clone();
                    out.push(self.ev(
                        "response.function_call_arguments.delta",
                        json!({"item_id": id, "output_index": n, "delta": part}),
                    ));
                }
            }
            StreamEvent::ToolCallEnd { .. } => self.close(&mut out),
            StreamEvent::Usage { usage } => self.usage += *usage,
            StreamEvent::Finish { reason } => self.finish = Some(*reason),
            _ => {}
        }
        out
    }

    fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.start(&mut out);
        self.close(&mut out);
        let status = status_of(self.finish.unwrap_or(FinishReason::Stop));
        let output = self.items.iter().map(Item::done).collect();
        let r = envelope(
            &self.id,
            &self.model,
            self.created,
            status,
            output,
            Some(&self.usage),
        );
        let ty = if status == "incomplete" {
            "response.incomplete"
        } else {
            "response.completed"
        };
        out.push(self.ev(ty, json!({"response": r})));
        out
    }

    fn error(&mut self, e: &ProviderError) -> Vec<String> {
        let mut out = Vec::new();
        self.start(&mut out);
        let mut r = envelope(
            &self.id,
            &self.model,
            self.created,
            "failed",
            Vec::new(),
            None,
        );
        r["error"] = json!({"code": format!("{:?}", e.kind).to_lowercase(), "message": e.message});
        out.push(self.ev("response.failed", json!({"response": r})));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_items_and_instructions() {
        let b = json!({
            "model": "gpt-5", "instructions": "be terse", "stream": true, "max_output_tokens": 64,
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "reasoning", "encrypted_content": "xx", "summary": []},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]},
                {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{\"cmd\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "c1", "output": "a.rs"}
            ],
            "tools": [{"type": "function", "name": "shell", "parameters": {"type": "object"}},
                      {"type": "web_search"}]
        });
        let p = parse(&b).unwrap();
        assert_eq!(p.req.system.as_deref(), Some("be terse"));
        assert_eq!(p.req.messages.len(), 3);
        assert_eq!(
            p.req.messages[1].parts.len(),
            2,
            "text and call share the assistant turn"
        );
        assert_eq!(p.req.tools.len(), 1);
        assert_eq!(p.req.max_tokens, Some(64));
        assert_eq!(
            parse(&json!({"model": "m", "input": "x"}))
                .unwrap()
                .req
                .messages
                .len(),
            1
        );
    }

    #[test]
    fn stream_events_are_ordered_and_complete() {
        let mut s = Stream::new("m");
        let mut out = Vec::new();
        for ev in [
            StreamEvent::ThinkingDelta { text: "r".into() },
            StreamEvent::TextDelta { text: "hi".into() },
            StreamEvent::ToolCallStart {
                index: 0,
                id: "c1".into(),
                name: "f".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: "{}".into(),
            },
            StreamEvent::ToolCallEnd { index: 0 },
            StreamEvent::Usage {
                usage: Usage {
                    input: 1,
                    output: 2,
                    ..Usage::default()
                },
            },
            StreamEvent::Finish {
                reason: FinishReason::ToolUse,
            },
        ] {
            out.extend(s.event(&ev));
        }
        out.extend(s.finish());
        let evs: Vec<Value> = out
            .iter()
            .map(|f| {
                serde_json::from_str(f.lines().nth(1).unwrap().trim_start_matches("data: "))
                    .unwrap()
            })
            .collect();
        for (i, e) in evs.iter().enumerate() {
            assert_eq!(e["sequence_number"], i);
        }
        assert_eq!(evs[0]["type"], "response.created");
        let last = evs.last().unwrap();
        assert_eq!(last["type"], "response.completed");
        let output = last["response"]["output"].as_array().unwrap();
        assert_eq!(
            output
                .iter()
                .map(|o| o["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["reasoning", "message", "function_call"]
        );
        assert_eq!(output[2]["call_id"], "c1");
        assert_eq!(last["response"]["usage"]["total_tokens"], 3);
    }
}
