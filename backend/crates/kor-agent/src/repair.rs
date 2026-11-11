//! Self-repairing tool calls (spec 15.12).
//!
//! A call whose JSON does not parse goes through three attempts before the step fails:
//! 1. strict `serde_json`;
//! 2. [`lenient`]: trailing text, single quotes, Python literals, trailing commas and missing
//!    closing quotes/brackets are fixed locally;
//! 3. one round-trip to the same model with only the schema and the broken text
//!    ([`repair_prompt`]).
//!
//! Field names and value types are then fitted to the tool schema (`schema::coerce`) and the
//! result is validated (`schema::validate`) before any tool runs.

use std::fmt::Write as _;

use futures::StreamExt as _;
use kor_core::{Message, Role, StreamEvent, ToolSpec};
use kor_providers::{CancellationToken, ChatRequest};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::AgentEvent;

/// Tool-call outcomes of one agent, for `kor bench` success rates.
///
/// Each call counts once: `ok` (valid as sent), `repaired` (valid after a repair step) or
/// `failed` (still unparseable, unknown tool, or schema-invalid; the tool did not run).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolStats {
    pub tool_calls_ok: u64,
    pub tool_calls_repaired: u64,
    pub tool_calls_failed: u64,
}

/// Attempts made on a broken call before it is reported as failed.
pub const MAX_ATTEMPTS: u32 = 3;

/// Parse the first JSON value in `text`, repairing common model mistakes. `None` when even
/// the repaired text is not JSON.
pub fn lenient(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
    let fixed = normalize(&text[start..]);
    let mut it = serde_json::Deserializer::from_str(&fixed).into_iter::<Value>();
    it.next()?.ok()
}

/// Single-pass rewrite: `'x'` → `"x"`, `True/False/None` → JSON, trailing commas dropped,
/// unterminated strings and brackets closed. Text after the first complete value is cut.
fn normalize(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            match c {
                '\\' => {
                    match chars.next() {
                        // `\'` inside a single-quoted string is just a quote.
                        Some('\'') => out.push('\''),
                        Some(n) => {
                            out.push('\\');
                            out.push(n);
                        }
                        None => {}
                    }
                    continue;
                }
                '"' if q == '\'' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                c if c == q => {
                    out.push('"');
                    quote = None;
                }
                c => out.push(c),
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                out.push('"');
            }
            '{' => {
                stack.push('}');
                out.push(c);
            }
            '[' => {
                stack.push(']');
                out.push(c);
            }
            '}' | ']' => {
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                if stack.last() == Some(&c) {
                    stack.pop();
                }
                out.push(c);
                if stack.is_empty() {
                    return out;
                }
            }
            c if c.is_ascii_alphabetic() => {
                let mut word = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        word.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let next = chars.clone().find(|c| !c.is_whitespace());
                match word.as_str() {
                    "True" => out.push_str("true"),
                    "False" => out.push_str("false"),
                    "None" => out.push_str("null"),
                    // `{path: "x"}`: a bare key.
                    w if next == Some(':') => {
                        let _ = write!(out, "\"{w}\"");
                    }
                    w => out.push_str(w),
                }
            }
            c => out.push(c),
        }
    }
    if quote.is_some() {
        out.push('"');
    }
    let trimmed = out.trim_end().len();
    if out[..trimmed].ends_with(',') {
        out.truncate(trimmed - 1);
    }
    while let Some(c) = stack.pop() {
        out.push(c);
    }
    out
}

/// The system prompt of the repair round-trip: only the schema(s), no conversation.
/// `hint` is the tool name when the caller knows it.
pub fn repair_prompt(hint: &str, specs: &[ToolSpec]) -> String {
    let mut p = String::from(
        "You repair malformed tool calls. Reply with exactly one JSON object and nothing else:\n\
         {\"name\": \"TOOL_NAME\", \"arguments\": {...}}\n\
         The arguments must match the tool's JSON schema. Keep the values the user wrote; \
         only fix the syntax, field names and types.\n\nTools:\n",
    );
    let chosen: Vec<&ToolSpec> = specs
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(hint))
        .collect();
    let list = if chosen.is_empty() {
        specs.iter().collect()
    } else {
        chosen
    };
    for s in list {
        let _ = writeln!(p, "- {}: {}", s.name, s.input_schema);
    }
    p
}

/// Attempt 2: local repair of the raw text. `name` is empty when the whole call
/// (`{"name", "arguments"}`) is in `raw`.
fn local(name: &str, raw: &str) -> Option<(String, Value)> {
    if name.is_empty() {
        crate::toolcall::call_from_text(raw)
    } else {
        lenient(raw)
            .filter(Value::is_object)
            .map(|v| (name.to_owned(), v))
    }
}

impl crate::Agent {
    /// Turn one model tool call into a validated `(tool, input)`, repairing it if needed,
    /// and count the outcome in [`ToolStats`]. `Err` is the message for the model.
    pub(crate) async fn prepare_call(
        &mut self,
        name: String,
        input: Result<Value, String>,
        specs: &[ToolSpec],
        events: &UnboundedSender<AgentEvent>,
        cancel: &CancellationToken,
    ) -> (String, Result<Value, String>) {
        let (name, value, repaired) = match input {
            Ok(v) => (name, v, false),
            Err(raw) => match self.repair_raw(&name, &raw, specs, events, cancel).await {
                Ok((n, v)) => (n, v, true),
                Err(e) => {
                    self.stats.tool_calls_failed += 1;
                    return (name, Err(e));
                }
            },
        };
        match crate::toolcall::prepare(&name, value, specs) {
            Ok((n, v, changed)) => {
                kor_tools::reveal(&self.ctx, &n);
                if repaired || changed {
                    self.stats.tool_calls_repaired += 1;
                } else {
                    self.stats.tool_calls_ok += 1;
                }
                (n, Ok(v))
            }
            Err(e) => {
                self.stats.tool_calls_failed += 1;
                (name, Err(e))
            }
        }
    }

    /// Attempts 2 and 3 on text that is not JSON. After the third failure the step is
    /// reported to the user (notice) and to the model (the error).
    async fn repair_raw(
        &mut self,
        name: &str,
        raw: &str,
        specs: &[ToolSpec],
        events: &UnboundedSender<AgentEvent>,
        cancel: &CancellationToken,
    ) -> Result<(String, Value), String> {
        if let Some(call) = local(name, raw) {
            return Ok(call);
        }
        if !cancel.is_cancelled()
            && let Some(reply) = self.ask_repair(name, raw, specs, cancel).await
        {
            let fixed = crate::toolcall::call_from_text(&reply).or_else(|| local(name, &reply));
            if let Some((n, v)) = fixed {
                return Ok((if n.is_empty() { name.to_owned() } else { n }, v));
            }
        }
        let shown = if name.is_empty() { "a tool" } else { name };
        let _ = events.send(AgentEvent::Notice(format!(
            "{} wrote a call to {shown} that could not be repaired after {MAX_ATTEMPTS} attempts; the step failed",
            self.model
        )));
        Err(format!(
            "the call to {shown} was not run: its arguments are not valid JSON (tried {MAX_ATTEMPTS} repairs). \
             Send the call again as one valid JSON object that matches the tool's schema."
        ))
    }

    /// One cheap round-trip to the same model: only the schema and the broken output.
    async fn ask_repair(
        &mut self,
        name: &str,
        raw: &str,
        specs: &[ToolSpec],
        cancel: &CancellationToken,
    ) -> Option<String> {
        let req = ChatRequest {
            model: self.model.clone(),
            system: Some(repair_prompt(name, specs)),
            messages: vec![Message::text(
                Role::User,
                if name.is_empty() {
                    raw.to_owned()
                } else {
                    format!("Tool: {name}\nArguments: {raw}")
                },
            )],
            tools: Vec::new(),
            max_tokens: Some(2_000),
            temperature: None,
            reasoning: None,
        };
        let mut stream = self.provider.complete(req, cancel.clone()).await.ok()?;
        let mut text = String::new();
        while let Some(ev) = stream.next().await {
            match ev.ok()? {
                StreamEvent::TextDelta { text: t } => text.push_str(&t),
                StreamEvent::Usage { usage } => self.usage += usage,
                _ => {}
            }
        }
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repairs_common_model_mistakes() {
        let cases = [
            (r#"{"a": 1}"#, json!({"a": 1})),
            (r#"{"a": 1} and then I will continue"#, json!({"a": 1})),
            (r#"{"a": {"b": [1, 2"#, json!({"a": {"b": [1, 2]}})),
            (r#"{"a": "unterminated"#, json!({"a": "unterminated"})),
            ("{'a': 'it\\'s \"q\"'}", json!({"a": "it's \"q\""})),
            (r#"{"a": True, "b": None, "c": [1,],}"#, json!({"a": true, "b": null, "c": [1]})),
            (r#"{file_path: "x.rs"}"#, json!({"file_path": "x.rs"})),
            ("Sure! {\"a\": \"line\nbreak\"}", json!({"a": "line\nbreak"})),
        ];
        for (src, want) in cases {
            assert_eq!(lenient(src), Some(want), "{src}");
        }
        assert_eq!(lenient("no json here"), None);
        assert_eq!(lenient("{\"a\": ]"), None);
    }

    #[test]
    fn repair_prompt_names_only_the_hinted_tool() {
        let spec = |n: &str| ToolSpec {
            name: n.into(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        };
        let specs = [spec("Read"), spec("Write")];
        let p = repair_prompt("read", &specs);
        assert!(p.contains("- Read:") && !p.contains("- Write:"));
        assert!(repair_prompt("", &specs).contains("- Write:"));
    }
}
