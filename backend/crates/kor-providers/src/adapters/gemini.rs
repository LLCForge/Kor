//! Google Gemini API (`generateContent` / `streamGenerateContent`).

use async_trait::async_trait;
use kor_core::{
    ContentPart, FinishReason, ImageSource, ProviderError, ProviderErrorKind, Role, StreamEvent,
    Usage,
};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::{parse, str_at, system_text, tool_names, turns, u64_at, unsupported};
use crate::http::{Http, check_model_id, error_message};
use crate::stream::{self, Frame, Framing, Mapper};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

#[derive(Debug)]
pub(crate) struct Gemini {
    manifest: ProviderManifest,
    http: Http,
}

impl Gemini {
    pub(crate) fn new(manifest: ProviderManifest, http: Http) -> Self {
        Self { manifest, http }
    }
}

/// The `parameters` field accepts an `OpenAPI` subset; drop keys it rejects.
fn clean_schema(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "$schema" | "additionalProperties" | "$id"))
                .map(|(k, v)| (k.clone(), clean_schema(v)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(clean_schema).collect()),
        v => v.clone(),
    }
}

fn contents(req: &ChatRequest, provider: &str) -> Result<Vec<Value>, ProviderError> {
    let names = tool_names(req);
    let mut out: Vec<Value> = Vec::new();
    for m in turns(req) {
        let role = if m.role == Role::Assistant {
            "model"
        } else {
            "user"
        };
        let mut parts = Vec::new();
        for p in &m.parts {
            parts.push(match p {
                ContentPart::Text { text } => json!({"text": text}),
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => {
                    json!({"inlineData": {"mimeType": media_type, "data": data}})
                }
                ContentPart::Image {
                    source: ImageSource::Url { .. },
                } => {
                    return Err(unsupported(provider, "image by URL"));
                }
                ContentPart::ToolUse { name, input, .. } => {
                    json!({"functionCall": {"name": name, "args": input}})
                }
                ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let name = names.get(tool_use_id.as_str()).copied().ok_or_else(|| {
                        ProviderError::new(
                            provider,
                            ProviderErrorKind::InvalidRequest,
                            format!("tool result `{tool_use_id}` has no matching tool call"),
                        )
                    })?;
                    let key = if *is_error { "error" } else { "content" };
                    json!({"functionResponse": {"name": name, "response": {key: content}}})
                }
                ContentPart::Thinking { .. } => continue,
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            });
        }
        if parts.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last["role"] == role => {
                if let Some(arr) = last["parts"].as_array_mut() {
                    arr.extend(parts);
                }
            }
            _ => out.push(json!({"role": role, "parts": parts})),
        }
    }
    Ok(out)
}

pub(crate) fn body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut body = json!({"contents": contents(req, provider)?});
    if let Some(s) = system_text(req) {
        body["systemInstruction"] = json!({"parts": [{"text": s}]});
    }
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "parameters": clean_schema(&t.input_schema)}))
            .collect();
        body["tools"] = json!([{"functionDeclarations": decls}]);
    }
    let mut gen_config = Map::new();
    if let Some(n) = req.max_tokens {
        gen_config.insert("maxOutputTokens".into(), n.into());
    }
    if let Some(t) = req.temperature {
        gen_config.insert("temperature".into(), t.into());
    }
    if let Some(t) = req.reasoning.and_then(|r| thinking_config(r, &req.model)) {
        gen_config.insert("thinkingConfig".into(), t);
    }
    // §15.13: constrained decoding (the same schema subset as tool parameters).
    if let Some(s) = crate::response_schema() {
        gen_config.insert("responseMimeType".into(), "application/json".into());
        gen_config.insert("responseSchema".into(), clean_schema(&s.schema));
    }
    if !gen_config.is_empty() {
        body["generationConfig"] = Value::Object(gen_config);
    }
    Ok(body)
}

/// `generationConfig.thinkingConfig`: a budget goes as `thinkingBudget`. An effort goes as
/// `thinkingLevel` (MINIMAL..HIGH; none → MINIMAL, xhigh/max → HIGH), except on Gemini 2.x,
/// which rejects `thinkingLevel` and gets the effort's budget (`ReasoningEffort::budget`).
fn thinking_config(r: kor_core::Reasoning, model: &str) -> Option<Value> {
    use kor_core::ReasoningEffort as E;
    if r.budget_tokens.is_some() || model.contains("gemini-2") {
        return r.tokens().map(|b| json!({"thinkingBudget": b}));
    }
    let level = match r.effort? {
        E::None | E::Minimal => "MINIMAL",
        E::Low => "LOW",
        E::Medium => "MEDIUM",
        E::High | E::Xhigh | E::Max => "HIGH",
    };
    Some(json!({"thinkingLevel": level}))
}

#[derive(Debug, Default)]
pub(crate) struct GeminiMapper {
    provider: String,
    next_tool: u32,
    usage: Option<Usage>,
    finish: Option<FinishReason>,
}

impl GeminiMapper {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            ..Self::default()
        }
    }
}

impl Mapper for GeminiMapper {
    fn frame(&mut self, f: Frame) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = parse(&self.provider, &f.data)?;
        if v.get("error").is_some() {
            let msg = error_message(&v).unwrap_or_else(|| "stream error".into());
            return Err(ProviderError::new(
                &self.provider,
                ProviderErrorKind::Unavailable,
                msg,
            ));
        }
        let mut out = Vec::new();
        let parts = v
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array);
        for p in parts.into_iter().flatten() {
            if let Some(t) = p["text"].as_str() {
                out.push(if p["thought"].as_bool() == Some(true) {
                    StreamEvent::ThinkingDelta { text: t.to_owned() }
                } else {
                    StreamEvent::TextDelta { text: t.to_owned() }
                });
            }
            if let Some(fc) = p.get("functionCall") {
                let index = self.next_tool;
                self.next_tool += 1;
                out.push(StreamEvent::ToolCallStart {
                    index,
                    id: str_at(fc, "/id").map_or_else(|| format!("call_{index}"), str::to_owned),
                    name: str_at(fc, "/name").unwrap_or_default().to_owned(),
                });
                out.push(StreamEvent::ToolCallDelta {
                    index,
                    json: fc.get("args").unwrap_or(&json!({})).to_string(),
                });
                out.push(StreamEvent::ToolCallEnd { index });
            }
        }
        if str_at(&v, "/promptFeedback/blockReason").is_some() {
            self.finish = Some(FinishReason::ContentFiltered);
        }
        if let Some(r) = str_at(&v, "/candidates/0/finishReason") {
            self.finish = Some(match r {
                "MAX_TOKENS" => FinishReason::MaxTokens,
                "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII"
                | "IMAGE_SAFETY" => FinishReason::ContentFiltered,
                _ if self.next_tool > 0 => FinishReason::ToolUse,
                _ => FinishReason::Stop,
            });
        }
        if let Some(u) = v.get("usageMetadata") {
            self.usage = Some(Usage {
                input: u64_at(u, "/promptTokenCount"),
                output: u64_at(u, "/candidatesTokenCount"),
                cache_read: u64_at(u, "/cachedContentTokenCount"),
                cache_write: 0,
                reasoning: u64_at(u, "/thoughtsTokenCount"),
            });
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        let reason = self
            .finish
            .ok_or_else(|| super::protocol(&self.provider, "stream ended without finishReason"))?;
        let mut out: Vec<_> = self
            .usage
            .map(|usage| StreamEvent::Usage { usage })
            .into_iter()
            .collect();
        out.push(StreamEvent::Finish { reason });
        Ok(out)
    }
}

#[async_trait]
impl Provider for Gemini {
    fn preconnect(&self) {
        self.http.preconnect();
    }

    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let v = self.http.get_json("/models?pageSize=1000").await?;
        Ok(v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x == "generateContent"))
            })
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m["name"].as_str()?.trim_start_matches("models/").to_owned(),
                    display_name: m["displayName"].as_str().map(str::to_owned),
                    context_window: m["inputTokenLimit"].as_u64(),
                    max_output: m["outputTokenLimit"].as_u64(),
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
        check_model_id(self.http.provider(), &req.model)?;
        let body = body(&req, self.http.provider())?;
        let path = format!("/models/{}:streamGenerateContent?alt=sse", req.model);
        let resp = self.http.post_json(&path, body, &[]).await?;
        Ok(stream::spawn(
            &self.http,
            resp,
            Framing::Sse,
            GeminiMapper::new(self.http.provider()),
            cancel,
        ))
    }

    async fn count_tokens(&self, req: &ChatRequest) -> Result<Option<u64>, ProviderError> {
        check_model_id(self.http.provider(), &req.model)?;
        let b = json!({"contents": contents(req, self.http.provider())?});
        let resp = self
            .http
            .post_json(&format!("/models/{}:countTokens", req.model), b, &[])
            .await?;
        Ok(self.http.read_json(resp).await?["totalTokens"].as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_cleaning_is_recursive() {
        let s = json!({"$schema": "x", "type": "object", "additionalProperties": false,
            "properties": {"a": {"type": "object", "additionalProperties": true}}});
        assert_eq!(
            clean_schema(&s),
            json!({"type": "object", "properties": {"a": {"type": "object"}}})
        );
    }
}
