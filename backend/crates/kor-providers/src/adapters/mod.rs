//! Protocol adapters. Each converts `ChatRequest` to a wire body and maps the
//! response stream back into `StreamEvent`s.

pub(crate) mod anthropic;
pub(crate) mod bedrock;
pub(crate) mod cohere;
pub(crate) mod gemini;
pub(crate) mod ollama;
pub(crate) mod openai;
pub(crate) mod openai_compat;
#[cfg(test)]
mod reasoning_tests;
pub(crate) mod sagemaker;

use std::collections::HashMap;

use kor_core::{ContentPart, ImageSource, Message, ProviderError, ProviderErrorKind, Role};
use serde_json::Value;

use crate::ChatRequest;

/// `req.system` plus any `System`-role messages, joined.
pub(crate) fn system_text(req: &ChatRequest) -> Option<String> {
    let parts: Vec<String> = req
        .system
        .iter()
        .cloned()
        .chain(
            req.messages
                .iter()
                .filter(|m| m.role == Role::System)
                .map(Message::plain_text),
        )
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Non-system messages, in order.
pub(crate) fn turns(req: &ChatRequest) -> impl Iterator<Item = &Message> {
    req.messages.iter().filter(|m| m.role != Role::System)
}

/// `tool_use` id → tool name, for protocols that key results by name.
pub(crate) fn tool_names(req: &ChatRequest) -> HashMap<&str, &str> {
    req.messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect()
}

/// The manifest's `static_models` as a model list.
pub(crate) fn static_models(m: &crate::ProviderManifest) -> Vec<crate::ModelInfo> {
    m.static_models
        .iter()
        .map(|id| crate::ModelInfo {
            id: id.clone(),
            display_name: None,
            context_window: m.max_context_default,
            max_output: None,
            size_bytes: None,
        })
        .collect()
}

pub(crate) fn data_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        ImageSource::Url { url } => url.clone(),
    }
}

pub(crate) fn unsupported(provider: &str, what: &str) -> ProviderError {
    ProviderError::new(
        provider,
        ProviderErrorKind::InvalidRequest,
        format!("{what} is not supported by this provider adapter yet"),
    )
}

pub(crate) fn protocol(provider: &str, msg: impl Into<String>) -> ProviderError {
    ProviderError::new(provider, ProviderErrorKind::Protocol, msg)
}

pub(crate) fn parse(provider: &str, data: &str) -> Result<Value, ProviderError> {
    serde_json::from_str(data).map_err(|e| protocol(provider, format!("bad JSON frame: {e}")))
}

pub(crate) fn u64_at(v: &Value, ptr: &str) -> u64 {
    v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn str_at<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}
