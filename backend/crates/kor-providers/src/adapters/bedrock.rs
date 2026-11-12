//! §4.2: AWS with IAM credentials; shared by the `bedrock` and `sagemaker` adapters (SigV4, [`crate::sigv4`]).
//! - Bedrock Converse (`POST /model/{id}/converse`): every Bedrock chat model through one API,
//!   <https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html>.
//! - SageMaker (`POST /endpoints/{name}/invocations`): an endpoint serving an OpenAI-compatible
//!   container (vLLM, LMI, TGI); the model id is the endpoint name.
//!
//! Both answer whole; the reply is turned into one burst of stream events.

use async_trait::async_trait;
use futures::stream;
use kor_core::{
    ContentPart, FinishReason, ImageSource, ProviderError, ProviderErrorKind, Role, StreamEvent,
    Usage,
};
use reqwest::Method;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{system_text, turns, unsupported};
use crate::http::Http;
use crate::sigv4::{self, Credentials};
use crate::{ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Service {
    Bedrock,
    Sagemaker,
}

impl Service {
    fn signing_name(self) -> &'static str {
        match self {
            Self::Bedrock => "bedrock",
            Self::Sagemaker => "sagemaker",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Aws {
    manifest: ProviderManifest,
    http: Http,
    service: Service,
    host: String,
    region: String,
}

/// The region in `bedrock-runtime.<region>.amazonaws.com` / `runtime.sagemaker.<region>...`.
pub(crate) fn region_of(base_url: &str) -> Option<String> {
    let host = base_url.split("://").nth(1)?.split('/').next()?;
    let parts: Vec<&str> = host.split('.').collect();
    let i = parts.iter().position(|p| *p == "amazonaws")?;
    (i >= 1)
        .then(|| parts[i - 1].to_owned())
        .filter(|r| r.contains('-'))
}

/// `manifest` with its regional endpoint filled in from `AWS_REGION` when its URL is still the
/// placeholder and its URL variable is unset; `None` when nothing changes.
pub(crate) fn regional(manifest: &ProviderManifest) -> Option<ProviderManifest> {
    use crate::AdapterKind;
    let host = match manifest.adapter {
        AdapterKind::Bedrock => "bedrock-runtime",
        AdapterKind::Sagemaker => "runtime.sagemaker",
        _ => return None,
    };
    let env_set = manifest
        .base_url_env
        .as_deref()
        .is_some_and(|v| std::env::var(v).is_ok_and(|s| !s.is_empty()));
    if env_set || !manifest.base_url.contains(".invalid") {
        return None;
    }
    let region = sigv4::region_from_env()?;
    let mut m = manifest.clone();
    m.base_url = format!("https://{host}.{region}.amazonaws.com");
    m.base_url_env = None;
    Some(m)
}

impl Aws {
    pub(crate) fn new(
        manifest: ProviderManifest,
        http: Http,
        service: Service,
    ) -> Result<Self, ProviderError> {
        let host = manifest
            .base_url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .unwrap_or_default()
            .to_owned();
        let region = region_of(&manifest.base_url)
            .or_else(sigv4::region_from_env)
            .ok_or_else(|| {
                ProviderError::new(
                    &manifest.id,
                    ProviderErrorKind::InvalidRequest,
                    "no AWS region: set AWS_REGION (or a regional endpoint URL)",
                )
            })?;
        Ok(Self {
            manifest,
            http,
            service,
            host,
            region,
        })
    }

    /// The base URL's own path prefix (empty for the regional hosts).
    fn prefix(&self) -> String {
        self.manifest
            .base_url
            .split("://")
            .nth(1)
            .and_then(|r| r.split_once('/'))
            .map(|(_, p)| format!("/{}", p.trim_end_matches('/')))
            .unwrap_or_default()
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, ProviderError> {
        let creds = Credentials::load().ok_or_else(|| {
            self.http.err(
                ProviderErrorKind::AuthFailed,
                "no AWS credentials: set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or use ~/.aws/credentials",
            )
        })?;
        let bytes = serde_json::to_vec(body)
            .map_err(|e| self.http.err(ProviderErrorKind::Protocol, e.to_string()))?;
        let full_path = format!("{}{path}", self.prefix());
        let signed = sigv4::sign(
            &sigv4::Request {
                method: "POST",
                host: &self.host,
                path: &full_path,
                query: "",
                body: &bytes,
                headers: &[("content-type", "application/json")],
            },
            &creds,
            &self.region,
            self.service.signing_name(),
            &sigv4::now(),
        );
        let mut req = self
            .http
            .request(Method::POST, path)
            .header("content-type", "application/json");
        for (k, v) in signed {
            req = req.header(k, v);
        }
        let resp = self.http.send(req.body(bytes)).await?;
        self.http.read_json(resp).await
    }
}

/// The Converse request body.
pub(crate) fn converse_body(req: &ChatRequest, provider: &str) -> Result<Value, ProviderError> {
    let mut messages: Vec<Value> = Vec::new();
    for m in turns(req) {
        let role = if m.role == Role::Assistant {
            "assistant"
        } else {
            "user"
        };
        let mut blocks = Vec::new();
        for p in &m.parts {
            blocks.push(match p {
                ContentPart::Text { text } if !text.is_empty() => json!({"text": text}),
                ContentPart::Text { .. } | ContentPart::Thinking { .. } => continue,
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => {
                    let format = media_type.rsplit('/').next().unwrap_or("png");
                    json!({"image": {"format": format, "source": {"bytes": data}}})
                }
                ContentPart::Image {
                    source: ImageSource::Url { .. },
                } => {
                    return Err(unsupported(provider, "image URL (send the image itself)"));
                }
                ContentPart::ToolUse { id, name, input } => {
                    json!({"toolUse": {"toolUseId": id, "name": name, "input": input}})
                }
                ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => json!({"toolResult": {
                    "toolUseId": tool_use_id,
                    "content": [{"text": content}],
                    "status": if *is_error { "error" } else { "success" },
                }}),
                ContentPart::File { .. } => return Err(unsupported(provider, "file attachment")),
            });
        }
        if blocks.is_empty() {
            continue;
        }
        // Converse wants alternating roles; consecutive turns of one role are merged.
        match messages.last_mut() {
            Some(last) if last["role"] == role => {
                if let Some(a) = last["content"].as_array_mut() {
                    a.extend(blocks);
                }
            }
            _ => messages.push(json!({"role": role, "content": blocks})),
        }
    }
    let mut body = json!({"messages": messages});
    if let Some(s) = system_text(req) {
        body["system"] = json!([{"text": s}]);
    }
    let mut config = serde_json::Map::new();
    if let Some(n) = req.max_tokens {
        config.insert("maxTokens".into(), n.into());
    }
    if let Some(t) = req.temperature {
        config.insert("temperature".into(), t.into());
    }
    if !config.is_empty() {
        body["inferenceConfig"] = Value::Object(config);
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"toolSpec": {"name": t.name, "description": t.description, "inputSchema": {"json": t.input_schema}}}))
            .collect();
        body["toolConfig"] = json!({"tools": tools});
    }
    // §15.13: constrained replies as a forced tool, as for Anthropic models.
    if let Some(s) = crate::response_schema() {
        let mut tools = body["toolConfig"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        tools.push(json!({"toolSpec": {"name": s.name, "description": "Give the whole answer through this tool, in exactly this structure.", "inputSchema": {"json": s.schema}}}));
        body["toolConfig"] = json!({"tools": tools, "toolChoice": {"tool": {"name": s.name}}});
    }
    Ok(body)
}

/// A Converse answer as stream events; `answer_tool` is the forced tool of a structured request,
/// whose input is the reply text.
pub(crate) fn converse_events(v: &Value, answer_tool: Option<&str>) -> Option<Vec<StreamEvent>> {
    let content = v["output"]["message"]["content"].as_array()?;
    let mut out = Vec::new();
    let mut index = 0u32;
    let mut answered = false;
    for c in content {
        if let Some(t) = c["text"].as_str() {
            out.push(StreamEvent::TextDelta { text: t.to_owned() });
        } else if let Some(u) = c.get("toolUse") {
            let name = u["name"].as_str().unwrap_or_default();
            if answer_tool == Some(name) {
                answered = true;
                out.push(StreamEvent::TextDelta {
                    text: u["input"].to_string(),
                });
                continue;
            }
            out.push(StreamEvent::ToolCallStart {
                index,
                id: u["toolUseId"].as_str().unwrap_or_default().to_owned(),
                name: name.to_owned(),
            });
            out.push(StreamEvent::ToolCallDelta {
                index,
                json: u["input"].to_string(),
            });
            out.push(StreamEvent::ToolCallEnd { index });
            index += 1;
        }
    }
    let usage = Usage {
        input: v["usage"]["inputTokens"].as_u64().unwrap_or(0),
        output: v["usage"]["outputTokens"].as_u64().unwrap_or(0),
        ..Usage::default()
    };
    out.push(StreamEvent::Usage { usage });
    let reason = match v["stopReason"].as_str() {
        Some("tool_use") if !answered => FinishReason::ToolUse,
        Some("max_tokens") => FinishReason::MaxTokens,
        Some("guardrail_intervened" | "content_filtered") => FinishReason::ContentFiltered,
        _ => FinishReason::Stop,
    };
    out.push(StreamEvent::Finish { reason });
    Some(out)
}

#[async_trait]
impl Provider for Aws {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    /// No list endpoint is called: Bedrock's catalogue is a different API (control plane), and a
    /// SageMaker endpoint serves one model. The manifest's `static_models` are offered.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(self
            .manifest
            .static_models
            .iter()
            .map(|id| ModelInfo {
                id: id.clone(),
                display_name: None,
                context_window: None,
                max_output: None,
                size_bytes: None,
            })
            .collect())
    }

    async fn complete(
        &self,
        req: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let id = sigv4::encode_segment(&req.model);
        let events = match self.service {
            Service::Bedrock => {
                let body = converse_body(&req, self.http.provider())?;
                let v = self.post(&format!("/model/{id}/converse"), &body).await?;
                let answer = crate::response_schema().map(|s| s.name);
                converse_events(&v, answer.as_deref())
            }
            Service::Sagemaker => {
                let mut body = super::openai_compat::body(&req, self.http.provider())?;
                body["stream"] = false.into();
                if let Some(o) = body.as_object_mut() {
                    o.remove("stream_options");
                }
                let v = self
                    .post(&format!("/endpoints/{id}/invocations"), &body)
                    .await?;
                super::sagemaker::chat_events(&v)
            }
        }
        .ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "malformed answer")
        })?;
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_comes_from_the_regional_host() {
        assert_eq!(
            region_of("https://bedrock-runtime.eu-central-1.amazonaws.com").as_deref(),
            Some("eu-central-1")
        );
        assert_eq!(
            region_of("https://runtime.sagemaker.us-east-1.amazonaws.com").as_deref(),
            Some("us-east-1")
        );
        assert_eq!(
            region_of("https://bedrock-runtime.your-region.invalid"),
            None
        );
    }

    #[test]
    fn converse_round_trip() {
        let req = ChatRequest {
            model: "anthropic.claude-x:0".into(),
            system: Some("be brief".into()),
            messages: vec![
                kor_core::Message::text(Role::User, "hi"),
                kor_core::Message::text(Role::Assistant, "hello"),
            ],
            max_tokens: Some(100),
            ..Default::default()
        };
        let b = converse_body(&req, "bedrock").unwrap();
        assert_eq!(b["system"][0]["text"], "be brief");
        assert_eq!(b["messages"][1]["role"], "assistant");
        assert_eq!(b["inferenceConfig"]["maxTokens"], 100);
        let v = json!({
            "output": {"message": {"role": "assistant", "content": [
                {"text": "Reading."},
                {"toolUse": {"toolUseId": "t1", "name": "Read", "input": {"path": "a.rs"}}}
            ]}},
            "stopReason": "tool_use",
            "usage": {"inputTokens": 12, "outputTokens": 5}
        });
        let ev = converse_events(&v, None).unwrap();
        assert_eq!(
            ev[0],
            StreamEvent::TextDelta {
                text: "Reading.".into()
            }
        );
        assert!(matches!(&ev[1], StreamEvent::ToolCallStart { name, .. } if name == "Read"));
        assert!(ev.contains(&StreamEvent::Finish {
            reason: FinishReason::ToolUse
        }));
        // A structured reply's forced tool is the text, and the turn ends.
        let s = json!({"output": {"message": {"content": [{"toolUse": {"toolUseId": "x", "name": "plan", "input": {"a": 1}}}]}}, "stopReason": "tool_use", "usage": {}});
        let ev = converse_events(&s, Some("plan")).unwrap();
        assert_eq!(
            ev[0],
            StreamEvent::TextDelta {
                text: "{\"a\":1}".into()
            }
        );
        assert!(ev.contains(&StreamEvent::Finish {
            reason: FinishReason::Stop
        }));
    }
}
