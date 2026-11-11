//! Protocol translation (port of CLIProxyAPI `internal/translator`).
//!
//! CLIProxyAPI keeps one translator per (client format, upstream format) pair. Kor already has a
//! neutral form for both directions, `kor_providers::ChatRequest` and the `StreamEvent` stream,
//! and adapters for every upstream wire format. So each client format here only needs two halves:
//! `parse` (client request → `ChatRequest`) and an encoder (`StreamEvent`s → client response,
//! streamed or whole). Every pair of the original matrix (OpenAI chat, OpenAI Responses, Claude
//! Messages, Gemini ⇄ OpenAI-compatible, Responses/Codex, Claude, Gemini, Ollama upstreams) goes
//! through that hub, so a new format is two files, not one per pair.

pub mod claude;
pub mod gemini;
pub mod openai;
pub mod responses;

use kor_core::{FinishReason, ImageSource, ProviderError, ProviderErrorKind, StreamEvent, Usage};
use kor_providers::ChatRequest;
use serde_json::{Map, Value};

/// Client wire format of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    OpenAi,
    Responses,
    Claude,
    Gemini,
}

/// A parsed client request.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    pub req: ChatRequest,
    /// Model name exactly as the client sent it (aliases are resolved later).
    pub model: String,
    pub stream: bool,
}

/// Client-side error: bad JSON shape or a field Kor cannot carry.
pub type ParseResult = Result<Inbound, String>;

/// Everything a non-streaming response needs, gathered from the event stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collected {
    pub text: String,
    pub thinking: String,
    pub signature: Option<String>,
    pub tools: Vec<ToolCall>,
    pub usage: Usage,
    pub finish: Option<FinishReason>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCall {
    pub index: u32,
    pub id: String,
    pub name: String,
    pub args: String,
}

impl Collected {
    pub fn push(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TextDelta { text } => self.text.push_str(text),
            StreamEvent::ThinkingDelta { text } => self.thinking.push_str(text),
            StreamEvent::ThinkingSignature { signature } => {
                self.signature = Some(signature.clone());
            }
            StreamEvent::ToolCallStart { index, id, name } => self.tools.push(ToolCall {
                index: *index,
                id: id.clone(),
                name: name.clone(),
                args: String::new(),
            }),
            StreamEvent::ToolCallDelta { index, json } => {
                if let Some(t) = self.tools.iter_mut().rev().find(|t| t.index == *index) {
                    t.args.push_str(json);
                }
            }
            StreamEvent::Usage { usage } => self.usage += *usage,
            StreamEvent::Finish { reason } => self.finish = Some(*reason),
            StreamEvent::ToolCallEnd { .. }
            | StreamEvent::Citation { .. }
            | StreamEvent::Raw { .. } => {}
        }
    }

    /// Tool use wins over a plain stop: some upstreams report `stop` after a tool call.
    pub fn finish_reason(&self) -> FinishReason {
        match self.finish {
            Some(FinishReason::Stop) | None if !self.tools.is_empty() => FinishReason::ToolUse,
            Some(r) => r,
            None => FinishReason::Stop,
        }
    }
}

/// Streaming encoder: one per response, fed every upstream event in order.
pub trait Encoder: Send {
    /// Frames to write for `ev` (already SSE-framed where the format uses SSE).
    fn event(&mut self, ev: &StreamEvent) -> Vec<String>;
    /// Frames closing a successful stream.
    fn finish(&mut self) -> Vec<String>;
    /// Frames reporting an error after the stream has started.
    fn error(&mut self, e: &ProviderError) -> Vec<String>;
}

pub fn encoder(format: Format, model: &str, gemini_sse: bool) -> Box<dyn Encoder> {
    match format {
        Format::OpenAi => Box::new(openai::Stream::new(model)),
        Format::Responses => Box::new(responses::Stream::new(model)),
        Format::Claude => Box::new(claude::Stream::new(model)),
        Format::Gemini => Box::new(gemini::Stream::new(model, gemini_sse)),
    }
}

pub fn parse(format: Format, body: &Value, model_from_path: Option<&str>) -> ParseResult {
    match format {
        Format::OpenAi => openai::parse(body),
        Format::Responses => responses::parse(body),
        Format::Claude => claude::parse(body),
        Format::Gemini => gemini::parse(body, model_from_path.unwrap_or_default()),
    }
}

/// Whole (non-streaming) response body.
pub fn complete(format: Format, model: &str, c: &Collected) -> Value {
    match format {
        Format::OpenAi => openai::complete(model, c),
        Format::Responses => responses::complete(model, c),
        Format::Claude => claude::complete(model, c),
        Format::Gemini => gemini::complete(model, c),
    }
}

/// HTTP status for an upstream failure. Upstream auth failures are a gateway problem (502),
/// not the client's: a 401 would tell the client its proxy key is wrong.
pub fn status_of(e: &ProviderError) -> u16 {
    match e.kind {
        ProviderErrorKind::RateLimited => 429,
        ProviderErrorKind::ContextTooLong
        | ProviderErrorKind::InvalidRequest
        | ProviderErrorKind::ContentFiltered => 400,
        ProviderErrorKind::Timeout => 504,
        ProviderErrorKind::AuthFailed | ProviderErrorKind::Protocol => 502,
        ProviderErrorKind::Unavailable | ProviderErrorKind::Cancelled => 503,
    }
}

/// Error body in the client's own format.
pub fn error_body(format: Format, status: u16, message: &str) -> Value {
    match format {
        Format::Claude => claude::error_json(status, message),
        Format::Gemini => gemini::error_json(status, message),
        Format::OpenAi | Format::Responses => openai::error_json(status, message),
    }
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// `data:image/png;base64,AAAA` → base64 source; anything else stays a URL.
pub(crate) fn image_from_url(url: &str) -> ImageSource {
    if let Some(rest) = url.strip_prefix("data:")
        && let Some((meta, data)) = rest.split_once(',')
        && let Some(media_type) = meta.strip_suffix(";base64")
    {
        return ImageSource::Base64 {
            media_type: media_type.to_owned(),
            data: data.to_owned(),
        };
    }
    ImageSource::Url {
        url: url.to_owned(),
    }
}

/// Tool arguments as a JSON value; malformed partial JSON is passed on as a string.
pub(crate) fn args_value(args: &str) -> Value {
    if args.trim().is_empty() {
        return Value::Object(Map::new());
    }
    serde_json::from_str(args).unwrap_or_else(|_| Value::String(args.to_owned()))
}

/// Text of a content value that is a string or a list of `{type:"text", text}` blocks.
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| match b {
                Value::String(s) => Some(s.as_str()),
                b => b.get("text").and_then(Value::as_str),
            })
            .collect::<Vec<_>>()
            .join(""),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(crate) fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key)
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

#[allow(clippy::cast_possible_truncation)] // temperature is a small float
pub(crate) fn opt_f32(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|f| f as f32)
}

pub(crate) fn sse_data(v: &Value) -> String {
    format!("data: {v}\n\n")
}

pub(crate) fn sse_event(name: &str, v: &Value) -> String {
    format!("event: {name}\ndata: {v}\n\n")
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Random hex id with a prefix (`chatcmpl-…`, `msg_…`, `resp_…`).
pub(crate) fn new_id(prefix: &str) -> String {
    let mut b = [0u8; 12];
    if getrandom::fill(&mut b).is_err() {
        // Ids only need to be unique per process; the clock is a fine fallback.
        b[..8].copy_from_slice(&now_nanos().to_le_bytes());
    }
    let hex = b.iter().fold(String::with_capacity(24), |mut s, x| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{x:02x}"));
        s
    });
    format!("{prefix}{hex}")
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collected_promotes_stop_to_tool_use_and_joins_args() {
        let mut c = Collected::default();
        for ev in [
            StreamEvent::ToolCallStart {
                index: 0,
                id: "c1".into(),
                name: "f".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: "{\"a\":".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: "1}".into(),
            },
            StreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ] {
            c.push(&ev);
        }
        assert_eq!(c.finish_reason(), FinishReason::ToolUse);
        assert_eq!(args_value(&c.tools[0].args), serde_json::json!({"a": 1}));
        assert_eq!(args_value("{bad"), Value::String("{bad".into()));
    }

    #[test]
    fn data_urls_round_trip() {
        let src = image_from_url("data:image/png;base64,AAAA");
        assert_eq!(
            src,
            ImageSource::Base64 {
                media_type: "image/png".into(),
                data: "AAAA".into()
            }
        );
        assert!(matches!(
            image_from_url("https://x/y.png"),
            ImageSource::Url { .. }
        ));
        assert!(new_id("msg_").starts_with("msg_") && new_id("x").len() == 25);
    }
}
