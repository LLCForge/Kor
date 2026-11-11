//! Gemini `generateContent` / `streamGenerateContent` (`POST /v1beta/models/{model}:…`), port of
//! the `gemini` request side and the `*_gemini_response` translators. Function calls carry no ids
//! in Gemini, so ids are synthesized and responses are matched to calls by name, in order.

use std::collections::{BTreeMap, VecDeque};

use kor_core::{
    ContentPart, FinishReason, ImageSource, Message, ProviderError, Role, StreamEvent, ToolSpec,
    Usage,
};
use kor_providers::ChatRequest;
use serde_json::{Value, json};

use super::{
    Collected, Encoder, Inbound, ParseResult, args_value, new_id, opt_f32, opt_u32, sse_data,
};

/// `camelCase` as the SDKs send it, `snake_case` as the REST docs also accept.
fn field<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake))
}

fn finish_str(r: FinishReason) -> &'static str {
    match r {
        FinishReason::MaxTokens => "MAX_TOKENS",
        FinishReason::ContentFiltered => "SAFETY",
        FinishReason::Error => "OTHER",
        FinishReason::Stop | FinishReason::ToolUse | FinishReason::Cancelled => "STOP",
    }
}

#[allow(clippy::too_many_lines)] // flat field-by-field mapping of the request schema; splitting adds no clarity
pub fn parse(body: &Value, model: &str) -> ParseResult {
    if model.is_empty() {
        return Err("model is missing from the path".into());
    }
    let contents = body
        .get("contents")
        .and_then(Value::as_array)
        .ok_or("`contents` must be an array")?;
    let mut calls = 0u32;
    // Call ids not answered yet, per function name.
    let mut pending: BTreeMap<String, VecDeque<String>> = BTreeMap::new();
    let mut messages = Vec::with_capacity(contents.len());
    for c in contents {
        let role = match c.get("role").and_then(Value::as_str) {
            Some("model") => Role::Assistant,
            _ => Role::User,
        };
        let mut parts = Vec::new();
        for p in c
            .get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = p.get("text").and_then(Value::as_str) {
                if p.get("thought").and_then(Value::as_bool) == Some(true) {
                    parts.push(ContentPart::Thinking {
                        text: text.to_owned(),
                        signature: None,
                    });
                } else {
                    parts.push(ContentPart::Text {
                        text: text.to_owned(),
                    });
                }
            } else if let Some(d) = field(p, "inlineData", "inline_data") {
                parts.push(ContentPart::Image {
                    source: ImageSource::Base64 {
                        media_type: field(d, "mimeType", "mime_type")
                            .and_then(Value::as_str)
                            .unwrap_or("image/png")
                            .to_owned(),
                        data: d
                            .get("data")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    },
                });
            } else if let Some(f) = field(p, "fileData", "file_data") {
                let url = field(f, "fileUri", "file_uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                parts.push(ContentPart::Image {
                    source: ImageSource::Url {
                        url: url.to_owned(),
                    },
                });
            } else if let Some(f) = field(p, "functionCall", "function_call") {
                let name = f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let id = f.get("id").and_then(Value::as_str).map_or_else(
                    || {
                        calls += 1;
                        format!("call_{calls}")
                    },
                    str::to_owned,
                );
                pending
                    .entry(name.clone())
                    .or_default()
                    .push_back(id.clone());
                parts.push(ContentPart::ToolUse {
                    id,
                    name,
                    input: f.get("args").cloned().unwrap_or_else(|| json!({})),
                });
            } else if let Some(f) = field(p, "functionResponse", "function_response") {
                let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
                let id = f
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| pending.get_mut(name).and_then(VecDeque::pop_front))
                    .ok_or_else(|| {
                        format!("functionResponse `{name}` has no matching functionCall")
                    })?;
                let resp = f.get("response").cloned().unwrap_or(Value::Null);
                let is_error = resp.get("error").is_some();
                let content = ["content", "result", "output", "error"]
                    .iter()
                    .find_map(|k| resp.get(*k).and_then(Value::as_str))
                    .map_or_else(|| resp.to_string(), str::to_owned);
                parts.push(ContentPart::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error,
                });
            }
            // thoughtSignature, executableCode and the like have no portable meaning: skipped.
        }
        messages.push(Message { role, parts });
    }
    let system = field(body, "systemInstruction", "system_instruction")
        .and_then(|s| s.get("parts"))
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty());
    let mut tools = Vec::new();
    for t in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for d in field(t, "functionDeclarations", "function_declarations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = d.get("name").and_then(Value::as_str) else {
                continue;
            };
            tools.push(ToolSpec {
                name: name.to_owned(),
                description: d
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: field(d, "parameters", "parametersJsonSchema")
                    .or_else(|| d.get("parameters_json_schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            });
        }
    }
    let gen_cfg = field(body, "generationConfig", "generation_config")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Inbound {
        req: ChatRequest {
            model: model.to_owned(),
            system,
            messages,
            tools,
            max_tokens: opt_u32(&gen_cfg, "maxOutputTokens")
                .or_else(|| opt_u32(&gen_cfg, "max_output_tokens")),
            temperature: opt_f32(&gen_cfg, "temperature"),
            reasoning: None,
        },
        model: model.to_owned(),
        stream: false,
    })
}

fn usage_json(u: &Usage) -> Value {
    let mut v = json!({
        "promptTokenCount": u.input,
        "candidatesTokenCount": u.output,
        "totalTokenCount": u.input.saturating_add(u.output),
    });
    if u.reasoning > 0 {
        v["thoughtsTokenCount"] = u.reasoning.into();
    }
    if u.cache_read > 0 {
        v["cachedContentTokenCount"] = u.cache_read.into();
    }
    v
}

fn response(
    id: &str,
    model: &str,
    parts: Vec<Value>,
    finish: Option<FinishReason>,
    usage: Option<&Usage>,
) -> Value {
    let mut cand = json!({"content": {"role": "model"}, "index": 0});
    cand["content"]["parts"] = parts.into();
    if let Some(r) = finish {
        cand["finishReason"] = finish_str(r).into();
    }
    let mut v = json!({"candidates": [cand], "modelVersion": model, "responseId": id});
    if let Some(u) = usage {
        v["usageMetadata"] = usage_json(u);
    }
    v
}

fn call_part(id: &str, name: &str, args: &str) -> Value {
    json!({"functionCall": {"id": id, "name": name, "args": args_value(args)}})
}

pub fn complete(model: &str, c: &Collected) -> Value {
    let mut parts = Vec::new();
    if !c.thinking.is_empty() {
        parts.push(json!({"text": c.thinking, "thought": true}));
    }
    if !c.text.is_empty() {
        parts.push(json!({"text": c.text}));
    }
    parts.extend(c.tools.iter().map(|t| call_part(&t.id, &t.name, &t.args)));
    if parts.is_empty() {
        parts.push(json!({"text": ""}));
    }
    response(
        &new_id(""),
        model,
        parts,
        Some(c.finish_reason()),
        Some(&c.usage),
    )
}

pub fn error_json(status: u16, message: &str) -> Value {
    let s = match status {
        400 | 413 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        504 => "DEADLINE_EXCEEDED",
        503 => "UNAVAILABLE",
        _ => "INTERNAL",
    };
    json!({"error": {"code": status, "message": message, "status": s}})
}

/// `alt=sse` gives SSE frames; without it Gemini streams one JSON array.
#[derive(Debug)]
pub struct Stream {
    id: String,
    model: String,
    sse: bool,
    sent: usize,
    /// Tool calls are sent whole, once their arguments are complete.
    tools: BTreeMap<u32, (String, String, String)>,
    usage: Usage,
    finish: Option<FinishReason>,
}

impl Stream {
    pub fn new(model: &str, sse: bool) -> Self {
        Self {
            id: new_id(""),
            model: model.to_owned(),
            sse,
            sent: 0,
            tools: BTreeMap::new(),
            usage: Usage::default(),
            finish: None,
        }
    }

    fn frame(&mut self, v: &Value) -> String {
        self.sent += 1;
        if self.sse {
            sse_data(v)
        } else if self.sent == 1 {
            format!("[{v}")
        } else {
            format!(",\r\n{v}")
        }
    }

    fn flush_tools(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.tools)
            .into_values()
            .map(|(id, name, args)| call_part(&id, &name, &args))
            .collect()
    }
}

impl Encoder for Stream {
    fn event(&mut self, ev: &StreamEvent) -> Vec<String> {
        let part = match ev {
            StreamEvent::TextDelta { text } => json!({"text": text}),
            StreamEvent::ThinkingDelta { text } => json!({"text": text, "thought": true}),
            StreamEvent::ToolCallStart { index, id, name } => {
                self.tools
                    .insert(*index, (id.clone(), name.clone(), String::new()));
                return Vec::new();
            }
            StreamEvent::ToolCallDelta { index, json: part } => {
                if let Some(t) = self.tools.get_mut(index) {
                    t.2.push_str(part);
                }
                return Vec::new();
            }
            StreamEvent::ToolCallEnd { index } => match self.tools.remove(index) {
                Some((id, name, args)) => call_part(&id, &name, &args),
                None => return Vec::new(),
            },
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
        let v = response(&self.id, &self.model, vec![part], None, None);
        vec![self.frame(&v)]
    }

    fn finish(&mut self) -> Vec<String> {
        let mut parts = self.flush_tools();
        if parts.is_empty() {
            parts.push(json!({"text": ""}));
        }
        let usage = self.usage;
        let v = response(
            &self.id,
            &self.model,
            parts,
            Some(self.finish.unwrap_or(FinishReason::Stop)),
            Some(&usage),
        );
        let mut out = vec![self.frame(&v)];
        if !self.sse {
            out.push("]".to_owned());
        }
        out
    }

    fn error(&mut self, e: &ProviderError) -> Vec<String> {
        let v = error_json(super::status_of(e), &e.message);
        let mut out = vec![self.frame(&v)];
        if !self.sse {
            out.push("]".to_owned());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contents_and_matches_function_responses_by_name() {
        let b = json!({
            "systemInstruction": {"parts": [{"text": "sys"}]},
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}, {"inline_data": {"mime_type": "image/jpeg", "data": "AA"}}]},
                {"role": "model", "parts": [{"functionCall": {"name": "f", "args": {"a": 1}}},
                                             {"functionCall": {"name": "f", "args": {"a": 2}}}]},
                {"role": "user", "parts": [{"functionResponse": {"name": "f", "response": {"result": "one"}}},
                                            {"functionResponse": {"name": "f", "response": {"x": 2}}}]}
            ],
            "tools": [{"functionDeclarations": [{"name": "f", "parameters": {"type": "object"}}]}],
            "generationConfig": {"maxOutputTokens": 9, "temperature": 0.5}
        });
        let p = parse(&b, "gemini-2.5-flash").unwrap();
        assert_eq!(p.req.system.as_deref(), Some("sys"));
        assert_eq!(p.req.max_tokens, Some(9));
        let ids: Vec<String> = p.req.messages[2]
            .parts
            .iter()
            .map(|x| match x {
                ContentPart::ToolResult { tool_use_id, .. } => tool_use_id.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(ids, ["call_1", "call_2"]);
        assert!(
            matches!(&p.req.messages[2].parts[0], ContentPart::ToolResult { content, .. } if content == "one")
        );
        assert!(
            parse(
                &json!({"contents": [{"role": "user", "parts": [
            {"functionResponse": {"name": "nope", "response": {}}}]}]}),
                "m"
            )
            .is_err()
        );
    }

    #[test]
    fn json_array_stream_is_valid_json() {
        let mut s = Stream::new("m", false);
        let mut out = String::new();
        for ev in [
            StreamEvent::TextDelta { text: "a".into() },
            StreamEvent::ToolCallStart {
                index: 0,
                id: "c".into(),
                name: "f".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: "{\"q\":1}".into(),
            },
            StreamEvent::Finish {
                reason: FinishReason::ToolUse,
            },
        ] {
            out.extend(s.event(&ev));
        }
        out.extend(s.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v[0]["candidates"][0]["content"]["parts"][0]["text"], "a");
        assert_eq!(
            v[1]["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["q"],
            1
        );
        assert_eq!(v[1]["candidates"][0]["finishReason"], "STOP");
    }
}
