//! Direct media providers (§20): OpenAI Images, Google Imagen and Veo (Gemini API), Stability AI,
//! Runway, Luma Dream Machine and a local ComfyUI. Each is one [`Direct`] client with the same
//! submit → poll → outputs shape, so the agent tools treat them alike; synchronous APIs answer
//! [`Job::Done`] at once, the others [`Job::Pending`] with an id to poll.
//!
//! - OpenAI: `POST /images/generations` (`b64_json`).
//! - Imagen: `POST /models/{model}:predict` (`predictions[].bytesBase64Encoded`).
//! - Veo: `POST /models/{model}:predictLongRunning`, then `GET /{operation}` until `done`; the
//!   video URI is fetched with the key (it is on the API host).
//! - Stability: `POST /v2beta/stable-image/generate/{core|ultra|sd3}` (multipart, JSON answer).
//! - Runway: `POST /v1/text_to_image` or `/v1/image_to_video`, then `GET /v1/tasks/{id}`.
//! - Luma: `POST /dream-machine/v1/generations[/image]`, then `GET …/generations/{id}`.
//! - ComfyUI: `POST /prompt` with an API-format workflow, `GET /history/{id}`, `GET /view`.
//!
//! Keys stay inside the HTTP client; error messages are redacted.

#![allow(clippy::doc_markdown)] // product names (ComfyUI, DALL·E, OpenAI) read better as written

use base64::Engine as _;
use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use reqwest::Method;
use serde_json::{Value, json};

use crate::ProviderManifest;
use crate::http::Http;

/// Largest generated file accepted.
pub const MAX_OUTPUT: usize = 1024 * 1024 * 1024;

/// The providers this module speaks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectKind {
    OpenAi,
    Imagen,
    Veo,
    Stability,
    Runway,
    Luma,
    ComfyUi,
}

impl DirectKind {
    pub const ALL: [Self; 7] = [
        Self::OpenAi,
        Self::Imagen,
        Self::Veo,
        Self::Stability,
        Self::Runway,
        Self::Luma,
        Self::ComfyUi,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Imagen => "imagen",
            Self::Veo => "veo",
            Self::Stability => "stability",
            Self::Runway => "runway",
            Self::Luma => "luma",
            Self::ComfyUi => "comfyui",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s.trim())
    }

    /// Makes videos (the rest make images; Runway and Luma make both).
    pub fn videos(self) -> bool {
        matches!(self, Self::Veo | Self::Runway | Self::Luma)
    }

    pub fn images(self) -> bool {
        !matches!(self, Self::Veo)
    }

    pub fn default_model(self, video: bool) -> &'static str {
        match (self, video) {
            (Self::OpenAi, _) => "gpt-image-1",
            (Self::Imagen, _) => "imagen-4.0-generate-001",
            (Self::Veo, _) => "veo-3.0-fast-generate-001",
            (Self::Stability, _) => "core",
            (Self::Runway, false) => "gen4_image",
            (Self::Runway, true) => "gen4_turbo",
            (Self::Luma, false) => "photon-1",
            (Self::Luma, true) => "ray-2",
            (Self::ComfyUi, _) => "workflow",
        }
    }

    /// Manifest for the providers Kor has no chat manifest for (key env var, base URL, headers).
    pub fn manifest_toml(self) -> Option<&'static str> {
        Some(match self {
            Self::Stability => {
                "id = \"stability\"\nname = \"Stability AI\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\n\
                 base_url = \"https://api.stability.ai\"\nauth = { kind = \"bearer\", env = \"STABILITY_API_KEY\" }\n\
                 verified = false\ndocs = \"https://platform.stability.ai/docs/api-reference\"\n\
                 [privacy]\ntier = \"unknown\"\nretention = \"Not published for the generation API (verify).\"\n"
            }
            Self::Runway => {
                "id = \"runway\"\nname = \"Runway\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\n\
                 base_url = \"https://api.dev.runwayml.com\"\nauth = { kind = \"bearer\", env = \"RUNWAYML_API_SECRET\" }\n\
                 verified = false\ndocs = \"https://docs.dev.runwayml.com/api\"\n\
                 [quirks]\nextra_headers = { \"X-Runway-Version\" = \"2024-11-06\" }\n\
                 [privacy]\ntier = \"unknown\"\nretention = \"Not published for the generation API (verify).\"\n"
            }
            Self::Luma => {
                "id = \"luma\"\nname = \"Luma AI\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\n\
                 base_url = \"https://api.lumalabs.ai\"\nauth = { kind = \"bearer\", env = \"LUMAAI_API_KEY\" }\n\
                 verified = false\ndocs = \"https://docs.lumalabs.ai/reference\"\n\
                 [privacy]\ntier = \"unknown\"\nretention = \"Not published for the generation API (verify).\"\n"
            }
            Self::ComfyUi => {
                "id = \"comfyui\"\nname = \"ComfyUI (local)\"\ncategory = \"local\"\nadapter = \"openai_compat\"\n\
                 base_url = \"http://127.0.0.1:8188\"\nauth = { kind = \"none\" }\n\
                 verified = false\ndocs = \"https://docs.comfy.org/development/comfyui-server/comms_routes\"\n\
                 [privacy]\ntier = \"local\"\nretention = \"Never leaves this machine.\"\n"
            }
            Self::OpenAi | Self::Imagen | Self::Veo => return None,
        })
    }
}

/// One generation request; each provider uses the fields it knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub model: String,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    /// `16:9`, `1:1`…
    pub aspect_ratio: Option<String>,
    /// OpenAI `1024x1024`, Runway `1280:720`, Luma `720p`.
    pub size: Option<String>,
    pub duration_s: Option<u32>,
    pub n: u8,
    pub seed: Option<i64>,
    /// Start frame / reference image as a `data:` or `https://` URL.
    pub image_url: Option<String>,
    /// ComfyUI: the API-format workflow; `{{prompt}}`, `{{negative}}`, `{{seed}}` placeholders
    /// in string values are filled from this request.
    pub workflow: Option<Value>,
    pub video: bool,
}

/// A finished output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Bytes {
        data: Vec<u8>,
        mime: String,
    },
    /// A link to fetch (with the key when it is on the provider's host: Veo, ComfyUI).
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Done(Vec<Output>),
    Pending(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Running(String),
    Done(Vec<Output>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Direct {
    pub kind: DirectKind,
    http: Http,
}

/// List price in USD for one request, when the provider publishes a fixed one (estimate).
/// OpenAI's token-priced `gpt-image-1` is charged from the response's `usage` instead.
pub fn estimate(kind: DirectKind, req: &Request) -> Option<f64> {
    let n = f64::from(req.n.max(1));
    let secs = f64::from(req.duration_s.unwrap_or(5));
    let m = req.model.as_str();
    let each = match kind {
        DirectKind::OpenAi => match m {
            "dall-e-3" => 0.04,
            "dall-e-2" => 0.02,
            _ => return None,
        },
        DirectKind::Imagen if m.contains("ultra") => 0.06,
        DirectKind::Imagen if m.contains("fast") => 0.02,
        DirectKind::Imagen => 0.04,
        DirectKind::Veo if m.contains("fast") => 0.15 * secs,
        DirectKind::Veo => 0.40 * secs,
        // Credits at $0.01: Stable Image Core 3, Ultra 8, SD 3.5 Large 6.5.
        DirectKind::Stability => match m {
            "ultra" => 0.08,
            "sd3" => 0.065,
            _ => 0.03,
        },
        // Credits at $0.01: gen4_turbo 5/s, gen4_image 5 (720p) or 8 (1080p).
        DirectKind::Runway if req.video => 0.05 * secs,
        DirectKind::Runway
            if req
                .size
                .as_deref()
                .is_some_and(|s| s.contains("1080") || s.contains("1920")) =>
        {
            0.08
        }
        DirectKind::Runway => 0.05,
        DirectKind::ComfyUi => 0.0,
        // No fixed list price in the API; nothing is charged rather than a guess.
        DirectKind::Luma => return None,
    };
    Some(each * n)
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn b64(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

/// Fill `{{prompt}}`, `{{negative}}` and `{{seed}}` in every string of a ComfyUI workflow; a
/// string that is exactly `{{seed}}` becomes a number.
pub fn fill_workflow(w: &Value, req: &Request) -> Value {
    match w {
        Value::String(t) if t == "{{seed}}" => json!(req.seed.unwrap_or(0)),
        Value::String(t) => Value::String(
            t.replace("{{prompt}}", &req.prompt)
                .replace("{{negative}}", req.negative_prompt.as_deref().unwrap_or(""))
                .replace("{{seed}}", &req.seed.unwrap_or(0).to_string()),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| fill_workflow(x, req)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), fill_workflow(x, req)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A plain text-to-image ComfyUI workflow (API format) for `checkpoint`.
pub fn default_workflow(checkpoint: &str) -> Value {
    json!({
        "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": checkpoint}},
        "5": {"class_type": "EmptyLatentImage", "inputs": {"width": 1024, "height": 1024, "batch_size": 1}},
        "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}", "clip": ["4", 1]}},
        "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{negative}}", "clip": ["4", 1]}},
        "3": {"class_type": "KSampler", "inputs": {"seed": "{{seed}}", "steps": 25, "cfg": 7.0,
            "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0,
            "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["5", 0]}},
        "8": {"class_type": "VAEDecode", "inputs": {"samples": ["3", 0], "vae": ["4", 2]}},
        "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "kor", "images": ["8", 0]}}
    })
}

fn multipart(fields: &[(&str, String)], boundary: &str) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut b = String::new();
    for (k, v) in fields {
        let _ = write!(
            b,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
        );
    }
    let _ = write!(b, "--{boundary}--\r\n");
    b.into_bytes()
}

impl Direct {
    /// `manifest`: the `openai` or `google` manifest for OpenAI/Imagen/Veo, else the one from
    /// [`DirectKind::manifest_toml`] (a user file may override its base URL).
    pub fn new(
        kind: DirectKind,
        manifest: &ProviderManifest,
        key: Option<SecretString>,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            kind,
            http: Http::new(manifest, key)?,
        })
    }

    /// The built-in manifest of a keyed provider without a chat manifest.
    pub fn builtin_manifest(kind: DirectKind) -> Option<ProviderManifest> {
        ProviderManifest::parse(kind.manifest_toml()?, kind.id()).ok()
    }

    fn proto(&self, m: impl AsRef<str>) -> ProviderError {
        self.http.err(ProviderErrorKind::Protocol, m)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, ProviderError> {
        let r = self.http.post_plain_json(path, body).await?;
        self.http.read_json(r).await
    }

    /// Start a generation.
    #[allow(clippy::too_many_lines)] // one arm per provider
    pub async fn submit(&self, req: &Request) -> Result<Job, ProviderError> {
        if req.prompt.trim().is_empty() && req.workflow.is_none() {
            return Err(self
                .http
                .err(ProviderErrorKind::InvalidRequest, "empty prompt"));
        }
        crate::http::check_model_id(self.http.provider(), &req.model)?;
        match self.kind {
            DirectKind::OpenAi => self.openai(req).await,
            DirectKind::Imagen => self.imagen(req).await,
            DirectKind::Veo => {
                let mut params = json!({});
                if let Some(a) = &req.aspect_ratio {
                    params["aspectRatio"] = json!(a);
                }
                if let Some(n) = &req.negative_prompt {
                    params["negativePrompt"] = json!(n);
                }
                if let Some(d) = req.duration_s {
                    params["durationSeconds"] = json!(d);
                }
                let v = self
                    .post(
                        &format!("models/{}:predictLongRunning", req.model),
                        &json!({"instances": [{"prompt": req.prompt}], "parameters": params}),
                    )
                    .await?;
                Ok(Job::Pending(s(&v, "/name")))
            }
            DirectKind::Stability => self.stability(req).await,
            DirectKind::Runway => {
                let ratio = req
                    .size
                    .clone()
                    .unwrap_or_else(|| if req.video { "1280:720" } else { "1920:1080" }.to_owned());
                let (path, mut body) = if req.video {
                    let Some(img) = &req.image_url else {
                        return Err(self.http.err(
                            ProviderErrorKind::InvalidRequest,
                            "Runway video needs a start image (image_to_video)",
                        ));
                    };
                    (
                        "v1/image_to_video",
                        json!({"model": req.model, "promptImage": img, "promptText": req.prompt,
                               "ratio": ratio, "duration": req.duration_s.unwrap_or(5)}),
                    )
                } else {
                    (
                        "v1/text_to_image",
                        json!({"model": req.model, "promptText": req.prompt, "ratio": ratio}),
                    )
                };
                if let Some(sd) = req.seed {
                    body["seed"] = json!(sd);
                }
                let v = self.post(path, &body).await?;
                Ok(Job::Pending(s(&v, "/id")))
            }
            DirectKind::Luma => {
                let (path, mut body) = if req.video {
                    let mut b = json!({"prompt": req.prompt, "model": req.model,
                        "resolution": req.size.clone().unwrap_or_else(|| "720p".into()),
                        "duration": format!("{}s", req.duration_s.unwrap_or(5))});
                    if let Some(img) = &req.image_url {
                        b["keyframes"] = json!({"frame0": {"type": "image", "url": img}});
                    }
                    ("dream-machine/v1/generations", b)
                } else {
                    (
                        "dream-machine/v1/generations/image",
                        json!({"prompt": req.prompt, "model": req.model}),
                    )
                };
                if let Some(a) = &req.aspect_ratio {
                    body["aspect_ratio"] = json!(a);
                }
                let v = self.post(path, &body).await?;
                Ok(Job::Pending(s(&v, "/id")))
            }
            DirectKind::ComfyUi => {
                let w = req.workflow.as_ref().ok_or_else(|| {
                    self.http
                        .err(ProviderErrorKind::InvalidRequest, "no ComfyUI workflow")
                })?;
                let v = self
                    .post(
                        "prompt",
                        &json!({"prompt": fill_workflow(w, req), "client_id": "kor"}),
                    )
                    .await?;
                if let Some(e) = v
                    .get("node_errors")
                    .filter(|e| e.as_object().is_some_and(|o| !o.is_empty()))
                {
                    return Err(self.proto(format!("workflow errors: {e}")));
                }
                Ok(Job::Pending(s(&v, "/prompt_id")))
            }
        }
    }

    async fn openai(&self, req: &Request) -> Result<Job, ProviderError> {
        let mut body = json!({"model": req.model, "prompt": req.prompt, "n": req.n.clamp(1, 4)});
        if let Some(sz) = &req.size {
            body["size"] = json!(sz);
        }
        // DALL·E answers with URLs unless asked; gpt-image-1 always returns base64.
        if req.model.starts_with("dall-e") {
            body["response_format"] = json!("b64_json");
        }
        let v = self.post("images/generations", &body).await?;
        let out: Vec<Output> = v
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| {
                d.get("b64_json")
                    .and_then(Value::as_str)
                    .and_then(b64)
                    .map(|data| Output::Bytes {
                        data,
                        mime: "image/png".into(),
                    })
                    .or_else(|| {
                        d.get("url")
                            .and_then(Value::as_str)
                            .map(|u| Output::Url(u.into()))
                    })
            })
            .collect();
        if out.is_empty() {
            return Err(self.proto("no image in the response"));
        }
        Ok(Job::Done(out))
    }

    /// gpt-image-1 token usage priced at $5/M text input and $40/M image output (estimate).
    pub fn openai_usage_cost(v: &Value) -> Option<f64> {
        let u = v.get("usage")?;
        #[allow(clippy::cast_precision_loss)] // token counts
        let cost = u.get("input_tokens")?.as_u64()? as f64 * 5e-6
            + u.get("output_tokens")?.as_u64()? as f64 * 40e-6;
        Some(cost)
    }

    async fn imagen(&self, req: &Request) -> Result<Job, ProviderError> {
        let mut params = json!({"sampleCount": req.n.clamp(1, 4)});
        if let Some(a) = &req.aspect_ratio {
            params["aspectRatio"] = json!(a);
        }
        let v = self
            .post(
                &format!("models/{}:predict", req.model),
                &json!({"instances": [{"prompt": req.prompt}], "parameters": params}),
            )
            .await?;
        let out: Vec<Output> = v
            .get("predictions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some(Output::Bytes {
                    data: b64(p.get("bytesBase64Encoded")?.as_str()?)?,
                    mime: Some(s(p, "/mimeType"))
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "image/png".into()),
                })
            })
            .collect();
        if out.is_empty() {
            return Err(self.proto("no image in the response (filtered by safety settings?)"));
        }
        Ok(Job::Done(out))
    }

    async fn stability(&self, req: &Request) -> Result<Job, ProviderError> {
        let mut fields = vec![
            ("prompt", req.prompt.clone()),
            ("output_format", "png".to_owned()),
        ];
        if let Some(n) = &req.negative_prompt {
            fields.push(("negative_prompt", n.clone()));
        }
        if let Some(a) = &req.aspect_ratio {
            fields.push(("aspect_ratio", a.clone()));
        }
        if let Some(sd) = req.seed {
            fields.push(("seed", sd.to_string()));
        }
        let boundary = format!("kor{:x}", std::process::id());
        let path = format!("v2beta/stable-image/generate/{}", req.model);
        let r = self
            .http
            .send(
                self.http
                    .request(Method::POST, &path)
                    .header("accept", "application/json")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(multipart(&fields, &boundary)),
            )
            .await?;
        let v = self.http.read_json(r).await?;
        if s(&v, "/finish_reason") == "CONTENT_FILTERED" {
            return Err(self
                .http
                .err(ProviderErrorKind::ContentFiltered, "the image was filtered"));
        }
        let data = b64(&s(&v, "/image")).ok_or_else(|| self.proto("no image in the response"))?;
        Ok(Job::Done(vec![Output::Bytes {
            data,
            mime: "image/png".into(),
        }]))
    }

    /// The state of a pending job.
    #[allow(clippy::too_many_lines)] // one arm per provider
    pub async fn poll(&self, id: &str) -> Result<Poll, ProviderError> {
        if id.is_empty()
            || id.contains("..")
            || id
                .chars()
                .any(|c| c.is_whitespace() || c == '?' || c == '#')
        {
            return Err(self.proto("bad job id"));
        }
        match self.kind {
            DirectKind::Veo => {
                let v = self.http.get_json(id).await?;
                if v.get("done").and_then(Value::as_bool) != Some(true) {
                    return Ok(Poll::Running(String::new()));
                }
                if let Some(e) = v.get("error") {
                    return Ok(Poll::Failed(s(e, "/message")));
                }
                let uris: Vec<Output> = v
                    .pointer("/response/generateVideoResponse/generatedSamples")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|x| Output::Url(s(x, "/video/uri")))
                    .filter(|o| *o != Output::Url(String::new()))
                    .collect();
                Ok(if uris.is_empty() {
                    Poll::Failed("no video in the response (filtered?)".into())
                } else {
                    Poll::Done(uris)
                })
            }
            DirectKind::Runway => {
                let v = self.http.get_json(&format!("v1/tasks/{id}")).await?;
                Ok(match s(&v, "/status").as_str() {
                    "SUCCEEDED" => Poll::Done(
                        v.get("output")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|u| Output::Url(u.into()))
                            .collect(),
                    ),
                    "FAILED" | "CANCELLED" => Poll::Failed(s(&v, "/failure")),
                    other => Poll::Running(match v.get("progress").and_then(Value::as_f64) {
                        Some(p) => format!("{other} {:.0}%", p * 100.0),
                        None => other.to_owned(),
                    }),
                })
            }
            DirectKind::Luma => {
                let v = self
                    .http
                    .get_json(&format!("dream-machine/v1/generations/{id}"))
                    .await?;
                Ok(match s(&v, "/state").as_str() {
                    "completed" => {
                        let u = Some(s(&v, "/assets/video"))
                            .filter(|u| !u.is_empty())
                            .unwrap_or_else(|| s(&v, "/assets/image"));
                        Poll::Done(vec![Output::Url(u)])
                    }
                    "failed" => Poll::Failed(s(&v, "/failure_reason")),
                    other => Poll::Running(other.to_owned()),
                })
            }
            DirectKind::ComfyUi => {
                let v = self.http.get_json(&format!("history/{id}")).await?;
                let Some(h) = v.get(id) else {
                    return Ok(Poll::Running("queued".into()));
                };
                if s(h, "/status/status_str") == "error" {
                    return Ok(Poll::Failed("the workflow failed in ComfyUI".into()));
                }
                let mut out = Vec::new();
                for node in h
                    .get("outputs")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                    .map(|(_, n)| n)
                {
                    for key in ["images", "gifs", "videos"] {
                        for f in node
                            .get(key)
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            let mut u = reqwest::Url::parse(&self.http.url("view"))
                                .map_err(|e| self.proto(e.to_string()))?;
                            u.query_pairs_mut()
                                .append_pair("filename", &s(f, "/filename"))
                                .append_pair("subfolder", &s(f, "/subfolder"))
                                .append_pair("type", &s(f, "/type"));
                            out.push(Output::Url(u.into()));
                        }
                    }
                }
                Ok(if out.is_empty() {
                    Poll::Running("running".into())
                } else {
                    Poll::Done(out)
                })
            }
            DirectKind::OpenAi | DirectKind::Imagen | DirectKind::Stability => {
                Err(self.proto("this provider answers at once; nothing to poll"))
            }
        }
    }

    /// The bytes and media type of an output. Links on the provider's own host (Veo files,
    /// ComfyUI `/view`) are fetched with the client's auth; others (CDNs) without any key.
    pub async fn fetch(&self, o: &Output) -> Result<(Vec<u8>, Option<String>), ProviderError> {
        match o {
            Output::Bytes { data, mime } => Ok((data.clone(), Some(mime.clone()))),
            Output::Url(u) => match self.http.request_url(Method::GET, u) {
                Ok(rb) => {
                    let r = self.http.send(rb).await?;
                    let ct = r
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let mut r = r;
                    let mut out = Vec::new();
                    while let Some(c) = r.chunk().await.map_err(|e| self.http.transport(&e))? {
                        if out.len() + c.len() > MAX_OUTPUT {
                            return Err(self.proto("output larger than 1 GB"));
                        }
                        out.extend_from_slice(&c);
                    }
                    Ok((out, ct))
                }
                Err(_) => crate::fal::download(u, MAX_OUTPUT).await,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(kind: DirectKind, server: &MockServer, toml: &str) -> Direct {
        let text = toml.replace("BASE", &server.uri());
        let m = ProviderManifest::parse(&text, "test").unwrap();
        Direct::new(kind, &m, Some(SecretString::from("k-123".to_owned()))).unwrap()
    }

    const BEARER: &str = "id = \"t\"\nname = \"T\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\nbase_url = \"BASE\"\nauth = { kind = \"bearer\", env = \"T_KEY\" }\n[privacy]\ntier = \"unknown\"\nretention = \"x\"\n";
    const GOOGLE: &str = "id = \"google\"\nname = \"G\"\ncategory = \"standard\"\nadapter = \"gemini\"\nbase_url = \"BASE/v1beta\"\nauth = { kind = \"header\", env = \"G\", header = \"x-goog-api-key\" }\n[privacy]\ntier = \"unknown\"\nretention = \"x\"\n";

    fn png() -> String {
        base64::engine::general_purpose::STANDARD.encode(b"\x89PNG")
    }

    fn req(model: &str, video: bool) -> Request {
        Request {
            model: model.into(),
            prompt: "a cat".into(),
            n: 1,
            video,
            ..Request::default()
        }
    }

    #[tokio::test]
    async fn synchronous_image_providers() {
        let s = MockServer::start().await;
        Mock::given(path("/images/generations"))
            .and(header("authorization", "Bearer k-123"))
            .and(body_partial_json(
                json!({"model": "dall-e-3", "response_format": "b64_json"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": png()}]})),
            )
            .mount(&s)
            .await;
        Mock::given(path("/v1beta/models/imagen-4.0-generate-001:predict"))
            .and(header("x-goog-api-key", "k-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"predictions": [{"bytesBase64Encoded": png(), "mimeType": "image/png"}]}),
            ))
            .mount(&s)
            .await;
        Mock::given(path("/v2beta/stable-image/generate/core"))
            .and(body_string_contains("name=\"prompt\"\r\n\r\na cat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"image": png(), "finish_reason": "SUCCESS"})),
            )
            .mount(&s)
            .await;
        let o = client(DirectKind::OpenAi, &s, BEARER)
            .submit(&req("dall-e-3", false))
            .await
            .unwrap();
        let Job::Done(o) = o else { panic!() };
        assert!(matches!(&o[0], Output::Bytes { data, .. } if data.starts_with(b"\x89PNG")));
        let i = client(DirectKind::Imagen, &s, GOOGLE)
            .submit(&req("imagen-4.0-generate-001", false))
            .await
            .unwrap();
        assert!(matches!(i, Job::Done(v) if v.len() == 1));
        let st = client(DirectKind::Stability, &s, BEARER)
            .submit(&req("core", false))
            .await
            .unwrap();
        assert!(matches!(st, Job::Done(_)));
        assert!(
            Direct::openai_usage_cost(
                &json!({"usage": {"input_tokens": 100, "output_tokens": 1000}})
            )
            .unwrap()
                > 0.04
        );
        assert_eq!(
            estimate(DirectKind::Stability, &req("ultra", false)),
            Some(0.08)
        );
        assert_eq!(estimate(DirectKind::Luma, &req("ray-2", true)), None);
        for k in [
            DirectKind::Stability,
            DirectKind::Runway,
            DirectKind::Luma,
            DirectKind::ComfyUi,
        ] {
            assert!(Direct::builtin_manifest(k).is_some(), "{}", k.id());
        }
    }

    #[tokio::test]
    async fn veo_runway_luma_poll_and_fetch() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/v1beta/models/veo-3.0-fast-generate-001:predictLongRunning",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"name": "models/veo/operations/op1"})),
            )
            .mount(&s)
            .await;
        let file = format!("{}/v1beta/files/v1:download?alt=media", s.uri());
        Mock::given(path("/v1beta/models/veo/operations/op1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"done": true, "response":
                {"generateVideoResponse": {"generatedSamples": [{"video": {"uri": file}}]}}})),
            )
            .mount(&s)
            .await;
        Mock::given(path("/v1beta/files/v1:download"))
            .and(header("x-goog-api-key", "k-123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(b"MP4".to_vec()),
            )
            .mount(&s)
            .await;
        let veo = client(DirectKind::Veo, &s, GOOGLE);
        let Job::Pending(op) = veo
            .submit(&req("veo-3.0-fast-generate-001", true))
            .await
            .unwrap()
        else {
            panic!()
        };
        let Poll::Done(out) = veo.poll(&op).await.unwrap() else {
            panic!()
        };
        assert_eq!(veo.fetch(&out[0]).await.unwrap().0, b"MP4");

        Mock::given(path("/v1/image_to_video"))
            .and(header("x-runway-version", "2024-11-06"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "t1"})))
            .mount(&s)
            .await;
        Mock::given(path("/v1/tasks/t1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status": "FAILED", "failure": "moderation"})),
            )
            .mount(&s)
            .await;
        let rw = Direct::builtin_manifest(DirectKind::Runway).unwrap();
        let mut toml = DirectKind::Runway
            .manifest_toml()
            .unwrap()
            .replace("https://api.dev.runwayml.com", "BASE");
        toml.push('\n');
        let runway = client(DirectKind::Runway, &s, &toml);
        assert_eq!(rw.id, "runway");
        assert!(
            runway.submit(&req("gen4_turbo", true)).await.is_err(),
            "needs a start image"
        );
        let mut r = req("gen4_turbo", true);
        r.image_url = Some("https://x/y.png".into());
        let Job::Pending(id) = runway.submit(&r).await.unwrap() else {
            panic!()
        };
        assert_eq!(
            runway.poll(&id).await.unwrap(),
            Poll::Failed("moderation".into())
        );

        Mock::given(path("/dream-machine/v1/generations"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": "g1", "state": "queued"})),
            )
            .mount(&s)
            .await;
        Mock::given(path("/dream-machine/v1/generations/g1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"state": "dreaming"})))
            .mount(&s)
            .await;
        let luma = client(DirectKind::Luma, &s, BEARER);
        let Job::Pending(id) = luma.submit(&req("ray-2", true)).await.unwrap() else {
            panic!()
        };
        assert!(matches!(luma.poll(&id).await.unwrap(), Poll::Running(_)));
        assert!(luma.poll("../x").await.is_err());
    }

    #[tokio::test]
    async fn comfyui_workflow_history_and_view() {
        let s = MockServer::start().await;
        Mock::given(path("/prompt"))
            .and(body_partial_json(json!({"prompt": {"6": {"inputs": {"text": "a cat"}}, "3": {"inputs": {"seed": 7}}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prompt_id": "p1", "number": 1, "node_errors": {}})))
            .mount(&s)
            .await;
        Mock::given(path("/history/p1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"p1": {"status": {"status_str": "success", "completed": true},
                "outputs": {"9": {"images": [{"filename": "kor_0001.png", "subfolder": "", "type": "output"}]}}}})))
            .mount(&s)
            .await;
        Mock::given(path("/view"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG".to_vec()))
            .mount(&s)
            .await;
        let toml = "id = \"comfyui\"\nname = \"C\"\ncategory = \"local\"\nadapter = \"openai_compat\"\nbase_url = \"BASE\"\nauth = { kind = \"none\" }\n[privacy]\ntier = \"local\"\nretention = \"x\"\n";
        let c = client(DirectKind::ComfyUi, &s, toml);
        let mut r = req("workflow", false);
        r.seed = Some(7);
        r.workflow = Some(default_workflow("sd_xl_base_1.0.safetensors"));
        let Job::Pending(id) = c.submit(&r).await.unwrap() else {
            panic!()
        };
        let Poll::Done(out) = c.poll(&id).await.unwrap() else {
            panic!()
        };
        assert_eq!(c.fetch(&out[0]).await.unwrap().0, b"\x89PNG");
        assert_eq!(estimate(DirectKind::ComfyUi, &r), Some(0.0));
    }
}
