//! fal.ai: run any fal model through the queue API, discover models and their input schemas.
//!
//! Queue API (submit, status, result, cancel; `Authorization: Key <FAL_KEY>`):
//! <https://fal.ai/docs/model-apis/model-endpoints/queue>
//! Model search with `OpenAPI` schemas (`GET https://api.fal.ai/v1/models`, keyless):
//! <https://fal.ai/docs/platform-apis/v1/models>
//! Unit prices (`GET https://api.fal.ai/v1/models/pricing`, key required):
//! <https://fal.ai/docs/platform-apis/v1/models/pricing>
//! File inputs as data URIs: <https://fal.ai/docs/documentation/model-apis/fal-cdn>
//!
//! The same client also drives fal models through the Hugging Face router
//! (`https://router.huggingface.co/fal-ai/<id>?_subdomain=queue`, HF token), the way
//! huggingface.js does it (`packages/inference/src/providers/fal-ai.ts`).
//!
//! Tracking URLs from the submit answer are never fetched as given: only their path is kept
//! and joined to our own base, so the key never travels to a host the server names.

use std::time::Duration;

use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use reqwest::Method;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::ProviderManifest;
use crate::http::Http;

pub const QUEUE_BASE: &str = "https://queue.fal.run";
pub const PLATFORM_BASE: &str = "https://api.fal.ai/v1";
pub const HF_ROUTER_FAL: &str = "https://router.huggingface.co/fal-ai";
/// fal's REST API (storage uploads), as `getRestApiUrl()` in fal-js `libs/client/src/config.ts`.
pub const REST_BASE: &str = "https://rest.fal.ai";
/// Largest file sent in one storage PUT; fal-js switches to multipart uploads above this
/// (`MULTIPART_THRESHOLD` in `libs/client/src/storage.ts`).
// shortcut: no multipart upload, so inputs above 90 MiB are refused; add initiate-multipart if needed.
pub const MAX_UPLOAD_BYTES: u64 = 90 * 1024 * 1024;
/// Default text-to-image model (fast and cheap; listed as `text-to-image` by the model API).
pub const DEFAULT_IMAGE_MODEL: &str = "fal-ai/flux/schnell";
/// Default image edit model (listed as `image-to-image`).
pub const DEFAULT_EDIT_MODEL: &str = "fal-ai/nano-banana-pro/edit";
/// Background removal (listed as `image-to-image`).
pub const BACKGROUND_REMOVAL_MODEL: &str = "fal-ai/birefnet/v2";
/// Categories the model API uses, for discovery.
pub const CATEGORIES: &[&str] = &[
    "text-to-image",
    "image-to-image",
    "text-to-video",
    "image-to-video",
    "video-to-video",
    "text-to-speech",
    "text-to-audio",
    "speech-to-text",
    "audio-to-audio",
];
/// A downloaded output larger than this is refused rather than written half-way.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024 * 1024;

/// Answer of a queue submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submitted {
    pub request_id: String,
    /// Path of `response_url` (`/fal-ai/flux/requests/<id>`), joined to our base.
    pub path: String,
    pub queue_position: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueueStatus {
    InQueue {
        position: Option<u64>,
    },
    InProgress {
        logs: Vec<String>,
    },
    Completed {
        error: Option<String>,
        inference_secs: Option<f64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// Removed from the queue; it will not run (200).
    Removed,
    /// Running; asked to stop, it may still complete and bill (202).
    Requested,
    /// It finished before the cancel arrived (400).
    AlreadyCompleted,
    /// Unknown id or another account's request (404).
    NotFound,
}

/// One model of the search API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalModel {
    pub endpoint_id: String,
    pub name: String,
    pub category: String,
    pub description: String,
}

/// A unit price from the pricing API.
#[derive(Debug, Clone, PartialEq)]
pub struct Price {
    pub unit_price: f64,
    pub unit: String,
    pub currency: String,
}

/// A file named by a model's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFile {
    pub url: String,
    pub content_type: Option<String>,
}

/// A required input the caller left out, with what the schema says about it.
#[derive(Debug, Clone, PartialEq)]
pub struct MissingInput {
    pub name: String,
    pub description: String,
    pub options: Vec<Value>,
    pub default: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct Fal {
    queue: Http,
    platform: Option<Http>,
    /// Storage uploads (fal's own key only; the HF router has no storage route).
    rest: Option<Http>,
    /// `?_subdomain=queue` through the HF router, empty for fal itself.
    query: String,
}

fn with_base(m: &ProviderManifest, base: &str) -> ProviderManifest {
    let mut m = m.clone();
    base.clone_into(&mut m.base_url);
    m.base_url_env = None;
    m
}

/// `a=1&b=x%20y`: a percent-encoded query string (reqwest's `.query()` needs a feature we do
/// not enable; `Url` does the encoding).
pub(crate) fn query_string(pairs: &[(&str, String)]) -> String {
    let Ok(mut u) = reqwest::Url::parse("http://q/") else {
        return String::new();
    };
    u.query_pairs_mut()
        .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
    u.query().unwrap_or_default().to_owned()
}

/// `https://host/a/b?x` → `/a/b`.
pub fn url_path(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let path = &rest[rest.find('/')?..];
    let path = path.split(['?', '#']).next().unwrap_or(path);
    (!path.contains("..")).then(|| path.to_owned())
}

impl Fal {
    /// `manifest` is the `fal` provider's (auth header `Key <FAL_KEY>`).
    pub fn new(manifest: &ProviderManifest, key: SecretString) -> Result<Self, ProviderError> {
        Self::with_bases(manifest, key.clone(), QUEUE_BASE, Some(PLATFORM_BASE), "")?
            .with_storage(manifest, key, REST_BASE)
    }

    /// Enable storage uploads at `rest` (fal's own key; tests point it at a mock).
    pub fn with_storage(
        mut self,
        manifest: &ProviderManifest,
        key: SecretString,
        rest: &str,
    ) -> Result<Self, ProviderError> {
        self.rest = Some(Http::new(&with_base(manifest, rest), Some(key))?);
        Ok(self)
    }

    /// Whether [`Self::upload`] is available (fal's own key, not the HF router).
    pub fn can_upload(&self) -> bool {
        self.rest.is_some()
    }

    /// Upload a file to fal's storage and return its URL for a model input: `POST
    /// <rest>/storage/upload/initiate?storage_type=fal-cdn-v3` with `{content_type, file_name}`
    /// answers `{upload_url, file_url}`; the bytes go to `upload_url` with a keyless PUT (it is
    /// presigned), as fal-js `libs/client/src/storage.ts` does.
    pub async fn upload(
        &self,
        bytes: Vec<u8>,
        content_type: &str,
        file_name: &str,
    ) -> Result<String, ProviderError> {
        let rest = self.rest.as_ref().ok_or_else(|| {
            self.queue.err(
                ProviderErrorKind::InvalidRequest,
                "file uploads need fal's own key (FAL_KEY)",
            )
        })?;
        if bytes.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(rest.err(
                ProviderErrorKind::InvalidRequest,
                format!(
                    "the file is larger than {} MB",
                    MAX_UPLOAD_BYTES / (1024 * 1024)
                ),
            ));
        }
        let initiated = rest
            .post_plain_json(
                "storage/upload/initiate?storage_type=fal-cdn-v3",
                &json!({ "content_type": content_type, "file_name": file_name }),
            )
            .await?;
        let v = rest.read_json(initiated).await?;
        let field = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|u| {
                    u.starts_with("https://") || cfg!(test) && u.starts_with("http://127.0.0.1")
                })
                .map(str::to_owned)
        };
        let (Some(upload_url), Some(file_url)) = (field("upload_url"), field("file_url")) else {
            return Err(rest.err(ProviderErrorKind::Protocol, "malformed upload answer"));
        };
        // Presigned URL: our own client without the key header.
        let client = kor_core::net::client_builder()
            .user_agent(format!("kor/{}", kor_core::VERSION))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| rest.transport(&e))?;
        let put = client
            .put(&upload_url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| rest.transport(&e))?;
        if !put.status().is_success() {
            return Err(rest.err(
                ProviderErrorKind::Unavailable,
                format!("upload failed: HTTP {}", put.status().as_u16()),
            ));
        }
        Ok(file_url)
    }

    /// fal models billed to a Hugging Face account; `manifest` is the `huggingface` provider's.
    pub fn via_huggingface(
        manifest: &ProviderManifest,
        key: SecretString,
    ) -> Result<Self, ProviderError> {
        Self::with_bases(manifest, key, HF_ROUTER_FAL, None, "?_subdomain=queue")
    }

    /// Explicit bases (tests, proxies). `platform` is the model/pricing API, if any.
    pub fn with_bases(
        manifest: &ProviderManifest,
        key: SecretString,
        queue: &str,
        platform: Option<&str>,
        query: &str,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            queue: Http::new(&with_base(manifest, queue), Some(key.clone()))?,
            platform: platform
                .map(|p| Http::new(&with_base(manifest, p), Some(key)))
                .transpose()?,
            rest: None,
            query: query.to_owned(),
        })
    }

    fn q(&self, extra: &str) -> String {
        match (self.query.is_empty(), extra.is_empty()) {
            (true, true) => String::new(),
            (true, false) => format!("?{extra}"),
            (false, true) => self.query.clone(),
            (false, false) => format!("{}&{extra}", self.query),
        }
    }

    fn check_model(&self, model: &str) -> Result<(), ProviderError> {
        let ok = !model.is_empty()
            && model.len() <= 200
            && !model.contains("..")
            && model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
        if ok {
            Ok(())
        } else {
            Err(self.queue.err(
                ProviderErrorKind::InvalidRequest,
                format!("invalid fal model id `{model}`"),
            ))
        }
    }

    pub async fn submit(&self, model: &str, input: &Value) -> Result<Submitted, ProviderError> {
        self.check_model(model)?;
        let resp = self
            .queue
            .post_plain_json(&format!("{model}{}", self.q("")), input)
            .await?;
        let v = self.queue.read_json(resp).await?;
        parse_submit(&v).ok_or_else(|| {
            self.queue
                .err(ProviderErrorKind::Protocol, "malformed submit answer")
        })
    }

    pub async fn status(&self, s: &Submitted) -> Result<QueueStatus, ProviderError> {
        let v = self
            .queue
            .get_json(&format!("{}/status{}", s.path, self.q("logs=1")))
            .await?;
        parse_status(&v).ok_or_else(|| {
            self.queue
                .err(ProviderErrorKind::Protocol, "malformed status answer")
        })
    }

    pub async fn result(&self, s: &Submitted) -> Result<Value, ProviderError> {
        self.queue
            .get_json(&format!("{}{}", s.path, self.q("")))
            .await
    }

    pub async fn cancel(&self, s: &Submitted) -> Result<CancelOutcome, ProviderError> {
        let req = self
            .queue
            .request(Method::PUT, &format!("{}/cancel{}", s.path, self.q("")));
        match self.queue.send(req).await {
            Ok(r) if r.status().as_u16() == 202 => Ok(CancelOutcome::Requested),
            Ok(_) => Ok(CancelOutcome::Removed),
            Err(e) if e.http_status == Some(400) => Ok(CancelOutcome::AlreadyCompleted),
            Err(e) if e.http_status == Some(404) => Ok(CancelOutcome::NotFound),
            Err(e) => Err(e),
        }
    }

    /// Submit, poll every `poll` until done (reporting each status), fetch the result. On
    /// `cancel` the request is cancelled on fal's side too.
    pub async fn run(
        &self,
        model: &str,
        input: &Value,
        poll: Duration,
        deadline: Duration,
        cancel: &CancellationToken,
        on_status: &(dyn Fn(&QueueStatus) + Send + Sync),
    ) -> Result<Value, ProviderError> {
        let sub = self.submit(model, input).await?;
        self.wait(&sub, poll, deadline, cancel, on_status).await
    }

    /// Poll a submitted request until done (see [`Fal::run`]); also resumes a request submitted
    /// by an earlier process (`kor media collect`, ADR 0060).
    pub async fn wait(
        &self,
        sub: &Submitted,
        poll: Duration,
        deadline: Duration,
        cancel: &CancellationToken,
        on_status: &(dyn Fn(&QueueStatus) + Send + Sync),
    ) -> Result<Value, ProviderError> {
        on_status(&QueueStatus::InQueue {
            position: sub.queue_position,
        });
        let started = std::time::Instant::now();
        loop {
            tokio::select! {
                () = tokio::time::sleep(poll) => {}
                () = cancel.cancelled() => {
                    let outcome = self.cancel(sub).await;
                    let note = match outcome {
                        Ok(CancelOutcome::Requested) => "cancel requested; fal may still finish and bill it",
                        Ok(CancelOutcome::AlreadyCompleted) => "it had already finished (billed)",
                        _ => "removed from the queue",
                    };
                    return Err(self.queue.err(
                        ProviderErrorKind::Cancelled,
                        format!("cancelled (request {}): {note}", sub.request_id),
                    ));
                }
            }
            let st = self.status(sub).await?;
            on_status(&st);
            if let QueueStatus::Completed { error, .. } = st {
                if let Some(e) = error {
                    return Err(self.queue.err(ProviderErrorKind::InvalidRequest, e));
                }
                return self.result(sub).await;
            }
            if started.elapsed() > deadline {
                let _ = self.cancel(sub).await;
                return Err(self.queue.err(
                    ProviderErrorKind::Timeout,
                    format!(
                        "not finished after {} s (request {}); cancelled",
                        deadline.as_secs(),
                        sub.request_id
                    ),
                ));
            }
        }
    }

    fn platform(&self) -> Result<&Http, ProviderError> {
        self.platform.as_ref().ok_or_else(|| {
            self.queue.err(
                ProviderErrorKind::InvalidRequest,
                "model search needs fal's own API (FAL_KEY)",
            )
        })
    }

    /// Search fal's models: free text and/or a category (`text-to-image`, ...), active only.
    pub async fn models(
        &self,
        q: Option<&str>,
        category: Option<&str>,
        limit: u32,
    ) -> Result<Vec<FalModel>, ProviderError> {
        let p = self.platform()?;
        let mut query: Vec<(&str, String)> = vec![
            ("status", "active".into()),
            ("limit", limit.clamp(1, 100).to_string()),
        ];
        if let Some(q) = q {
            query.push(("q", q.to_owned()));
        }
        if let Some(c) = category {
            query.push(("category", c.to_owned()));
        }
        let v = p
            .get_json(&format!("models?{}", query_string(&query)))
            .await?;
        Ok(parse_models(&v))
    }

    /// The input JSON schema of one model, from its `OpenAPI` document.
    pub async fn input_schema(&self, endpoint_id: &str) -> Result<Value, ProviderError> {
        self.check_model(endpoint_id)?;
        let p = self.platform()?;
        let q = query_string(&[
            ("endpoint_id", endpoint_id.to_owned()),
            ("expand", "openapi-3.0".to_owned()),
        ]);
        let v = p.get_json(&format!("models?{q}")).await?;
        v.pointer("/models/0/openapi")
            .and_then(input_schema_from_openapi)
            .ok_or_else(|| {
                p.err(
                    ProviderErrorKind::InvalidRequest,
                    format!("no input schema for `{endpoint_id}`"),
                )
            })
    }

    /// Unit price of a model, when fal publishes one.
    pub async fn price(&self, endpoint_id: &str) -> Result<Option<Price>, ProviderError> {
        self.check_model(endpoint_id)?;
        let p = self.platform()?;
        let q = query_string(&[("endpoint_id", endpoint_id.to_owned())]);
        let v = p.get_json(&format!("models/pricing?{q}")).await?;
        Ok(v.pointer("/prices/0").and_then(parse_price))
    }

    /// Active models of a category with their input schemas in one call (`expand=openapi-3.0`),
    /// for the capability catalog (§20.1).
    pub async fn catalog(
        &self,
        category: &str,
        limit: u32,
    ) -> Result<Vec<(FalModel, Option<Value>)>, ProviderError> {
        let p = self.platform()?;
        let q = query_string(&[
            ("status", "active".into()),
            ("limit", limit.clamp(1, 100).to_string()),
            ("category", category.to_owned()),
            ("expand", "openapi-3.0".into()),
        ]);
        let v = p.get_json(&format!("models?{q}")).await?;
        let schemas = v.get("models").and_then(Value::as_array);
        Ok(parse_models(&v)
            .into_iter()
            .map(|m| {
                let schema = schemas
                    .and_then(|a| {
                        a.iter().find(|x| {
                            x.get("endpoint_id").and_then(Value::as_str) == Some(&m.endpoint_id)
                        })
                    })
                    .and_then(|x| x.get("openapi"))
                    .and_then(input_schema_from_openapi);
                (m, schema)
            })
            .collect())
    }

    /// Unit prices of many models (`endpoint_id` repeated, at most 50 per request).
    pub async fn prices(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, Price>, ProviderError> {
        let p = self.platform()?;
        let mut out = std::collections::HashMap::new();
        for chunk in ids.chunks(50) {
            let pairs: Vec<(&str, String)> =
                chunk.iter().map(|id| ("endpoint_id", id.clone())).collect();
            let v = p
                .get_json(&format!("models/pricing?{}", query_string(&pairs)))
                .await?;
            for x in v
                .get("prices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(id), Some(price)) =
                    (x.get("endpoint_id").and_then(Value::as_str), parse_price(x))
                {
                    out.insert(id.to_owned(), price);
                }
            }
        }
        Ok(out)
    }
}

fn parse_price(x: &Value) -> Option<Price> {
    Some(Price {
        unit_price: x.get("unit_price")?.as_f64()?,
        unit: x.get("unit")?.as_str()?.to_owned(),
        currency: x
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("USD")
            .to_owned(),
    })
}

pub fn parse_submit(v: &Value) -> Option<Submitted> {
    let request_id = v.get("request_id")?.as_str()?.to_owned();
    let path = url_path(v.get("response_url")?.as_str()?)?;
    Some(Submitted {
        request_id,
        path,
        queue_position: v.get("queue_position").and_then(Value::as_u64),
    })
}

pub fn parse_status(v: &Value) -> Option<QueueStatus> {
    let logs = || {
        v.get("logs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.get("message").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(match v.get("status")?.as_str()? {
        "IN_QUEUE" => QueueStatus::InQueue {
            position: v.get("queue_position").and_then(Value::as_u64),
        },
        "IN_PROGRESS" => QueueStatus::InProgress { logs: logs() },
        "COMPLETED" => QueueStatus::Completed {
            error: v.get("error").and_then(|e| match e {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            }),
            inference_secs: v.pointer("/metrics/inference_time").and_then(Value::as_f64),
        },
        _ => return None,
    })
}

pub fn parse_models(v: &Value) -> Vec<FalModel> {
    let s = |m: &Value, p: &str| {
        m.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    v.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(FalModel {
                endpoint_id: m.get("endpoint_id")?.as_str()?.to_owned(),
                name: s(m, "/metadata/display_name"),
                category: s(m, "/metadata/category"),
                description: s(m, "/metadata/description"),
            })
        })
        .collect()
}

fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => doc.pointer(r.strip_prefix('#')?),
        None => Some(schema),
    }
}

/// Request body schema of the (first) POST operation, `$ref` resolved.
pub fn input_schema_from_openapi(doc: &Value) -> Option<Value> {
    doc.get("paths")?.as_object()?.values().find_map(|p| {
        let s = p.pointer("/post/requestBody/content/application~1json/schema")?;
        resolve(doc, s).cloned()
    })
}

/// Required inputs of `schema` absent from `input`, with their options and defaults.
pub fn missing_inputs(schema: &Value, input: &Value) -> Vec<MissingInput> {
    let props = schema.get("properties");
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| input.get(*name).is_none_or(Value::is_null))
        .map(|name| field(props.and_then(|p| p.get(name)), name))
        .collect()
}

/// What the schema says about `names` (for asking about optional choices too).
pub fn describe(schema: &Value, names: &[&str]) -> Vec<MissingInput> {
    let props = schema.get("properties");
    names
        .iter()
        .filter_map(|n| props.and_then(|p| p.get(*n)).map(|p| field(Some(p), n)))
        .collect()
}

fn field(p: Option<&Value>, name: &str) -> MissingInput {
    let get = |k: &str| p.and_then(|p| p.get(k));
    MissingInput {
        name: name.to_owned(),
        description: get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(160)
            .collect(),
        options: get("enum")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        default: get("default").cloned(),
    }
}

/// Every file an output names (`images[]`, `image`, `video`, `audio`, ...): objects with an
/// `https` `url`, in document order.
pub fn output_files(v: &Value) -> Vec<OutputFile> {
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

fn walk(v: &Value, out: &mut Vec<OutputFile>) {
    match v {
        Value::Object(o) => {
            if let Some(url) = o.get("url").and_then(Value::as_str)
                && (url.starts_with("https://") || url.starts_with("data:"))
            {
                out.push(OutputFile {
                    url: url.to_owned(),
                    content_type: o
                        .get("content_type")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
                return;
            }
            o.values().for_each(|x| walk(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

/// Fetch an output file without any key (it lives on a CDN). `https` only (plain http is allowed
/// in this crate's own tests); `data:` URLs are decoded.
pub async fn download(url: &str, max: usize) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    download_with_progress(url, max, &|_, _| {}).await
}

/// [`download`], reporting `(bytes so far, Content-Length)` after each chunk.
pub async fn download_with_progress(
    url: &str,
    max: usize,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<(Vec<u8>, Option<String>), ProviderError> {
    let err = |kind, m: String| ProviderError::new("fal", kind, m);
    if let Some(rest) = url.strip_prefix("data:") {
        use base64::Engine as _;
        let (meta, data) = rest
            .split_once(',')
            .ok_or_else(|| err(ProviderErrorKind::Protocol, "bad data URL".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| err(ProviderErrorKind::Protocol, e.to_string()))?;
        return Ok((bytes, meta.split(';').next().map(str::to_owned)));
    }
    if !(url.starts_with("https://") || cfg!(test) && url.starts_with("http://127.0.0.1")) {
        return Err(err(
            ProviderErrorKind::InvalidRequest,
            "output URL is not https".into(),
        ));
    }
    let client = kor_core::net::client_builder()
        .user_agent(format!("kor/{}", kor_core::VERSION))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| err(ProviderErrorKind::Protocol, e.to_string()))?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| err(ProviderErrorKind::Unavailable, "download failed".into()))?;
    if !resp.status().is_success() {
        return Err(err(
            ProviderErrorKind::Unavailable,
            format!("download failed: HTTP {}", resp.status().as_u16()),
        ));
    }
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let total = resp.content_length();
    let mut out = Vec::new();
    while let Some(c) = resp.chunk().await.map_err(|_| {
        err(
            ProviderErrorKind::Unavailable,
            "download interrupted".into(),
        )
    })? {
        if out.len() + c.len() > max {
            return Err(err(
                ProviderErrorKind::Protocol,
                format!("output larger than {} MB", max / (1024 * 1024)),
            ));
        }
        out.extend_from_slice(&c);
        progress(out.len() as u64, total);
    }
    Ok((out, ct))
}

/// A one-line summary of a status, for progress output.
pub fn status_line(s: &QueueStatus) -> String {
    match s {
        QueueStatus::InQueue { position: Some(p) } => format!("in queue (position {p})"),
        QueueStatus::InQueue { position: None } => "in queue".into(),
        QueueStatus::InProgress { logs } => match logs.last() {
            Some(l) => format!("running: {}", l.chars().take(120).collect::<String>()),
            None => "running".into(),
        },
        QueueStatus::Completed { error: Some(e), .. } => format!("failed: {e}"),
        QueueStatus::Completed { .. } => "completed".into(),
    }
}

/// `{"prompt": ..}` plus the caller's extra inputs (extra wins), as a fal input object.
pub fn input_with_prompt(prompt: &str, extra: &Value) -> Value {
    let mut v = json!({ "prompt": prompt });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn manifest() -> ProviderManifest {
        ProviderManifest::parse(include_str!("../../../providers/fal.toml"), "fal.toml").unwrap()
    }

    fn client(server: &MockServer) -> Fal {
        Fal::with_bases(
            &manifest(),
            SecretString::from("k-123".to_owned()),
            &server.uri(),
            Some(&server.uri()),
            "",
        )
        .unwrap()
    }

    #[test]
    fn paths_and_statuses() {
        assert_eq!(
            url_path("https://queue.fal.run/fal-ai/flux/requests/abc?x=1").as_deref(),
            Some("/fal-ai/flux/requests/abc")
        );
        assert_eq!(url_path("https://evil/../x"), None);
        assert_eq!(
            parse_status(&json!({"status": "IN_QUEUE", "queue_position": 3})),
            Some(QueueStatus::InQueue { position: Some(3) })
        );
        assert_eq!(
            parse_status(&json!({"status": "IN_PROGRESS", "logs": [{"message": "step 1"}]})),
            Some(QueueStatus::InProgress {
                logs: vec!["step 1".into()]
            })
        );
        assert_eq!(
            parse_status(&json!({"status": "COMPLETED", "metrics": {"inference_time": 1.5}})),
            Some(QueueStatus::Completed {
                error: None,
                inference_secs: Some(1.5)
            })
        );
    }

    #[test]
    fn schema_and_missing_inputs() {
        let doc = json!({
            "paths": {"/fal-ai/x": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/In"}}}}}}},
            "components": {"schemas": {"In": {
                "required": ["prompt", "duration"],
                "properties": {
                    "prompt": {"type": "string"},
                    "duration": {"enum": ["5", "10"], "default": "5", "description": "Seconds"}
                }}}}
        });
        let schema = input_schema_from_openapi(&doc).unwrap();
        let miss = missing_inputs(&schema, &json!({"prompt": "cat"}));
        assert_eq!(miss.len(), 1);
        assert_eq!(miss[0].name, "duration");
        assert_eq!(miss[0].options, vec![json!("5"), json!("10")]);
        assert_eq!(miss[0].default, Some(json!("5")));
    }

    #[test]
    fn output_files_are_found_anywhere() {
        let v = json!({"images": [{"url": "https://v3.fal.media/a.png", "content_type": "image/png"}],
            "video": {"url": "https://v3.fal.media/b.mp4"}, "seed": 1, "x": {"url": "http://insecure"}});
        let f = output_files(&v);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].content_type.as_deref(), Some("image/png"));
    }

    #[tokio::test]
    async fn submit_poll_result() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("POST"))
            .and(path("/fal-ai/flux/schnell"))
            .and(header("authorization", "Key k-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "r1",
                "response_url": format!("{base}/fal-ai/flux/requests/r1"),
                "status_url": format!("{base}/fal-ai/flux/requests/r1/status"),
                "queue_position": 0
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fal-ai/flux/requests/r1/status"))
            .and(query_param("logs", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "COMPLETED"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fal-ai/flux/requests/r1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "images": [{"url": format!("{base}/img.png"), "content_type": "image/png"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/img.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PNG".to_vec()))
            .mount(&server)
            .await;
        let fal = client(&server);
        let seen = std::sync::Mutex::new(Vec::new());
        let out = fal
            .run(
                "fal-ai/flux/schnell",
                &json!({"prompt": "cat"}),
                Duration::from_millis(10),
                Duration::from_secs(5),
                &CancellationToken::new(),
                &|s| seen.lock().unwrap().push(status_line(s)),
            )
            .await
            .unwrap();
        assert!(seen.lock().unwrap().contains(&"completed".to_owned()));
        let files = output_files(&out);
        // A plain-http test URL is not an output file; fetch it directly.
        assert!(files.is_empty());
        let (bytes, _) = download(&format!("{base}/img.png"), 1024).await.unwrap();
        assert_eq!(bytes, b"PNG");
    }

    #[tokio::test]
    async fn failed_request_and_cancel() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("POST"))
            .and(path("/fal-ai/slow"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "r2", "response_url": format!("{base}/fal-ai/slow/requests/r2")
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fal-ai/slow/requests/r2/status"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"status": "IN_PROGRESS"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/fal-ai/slow/requests/r2/cancel"))
            .respond_with(
                ResponseTemplate::new(202)
                    .set_body_json(json!({"status": "CANCELLATION_REQUESTED"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let fal = client(&server);
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            c2.cancel();
        });
        let e = fal
            .run(
                "fal-ai/slow",
                &json!({}),
                Duration::from_millis(20),
                Duration::from_secs(5),
                &cancel,
                &|_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, ProviderErrorKind::Cancelled);
        assert!(e.message.contains("may still finish"));

        // Errors: 401 from submit, a completed-with-error status, a 400 on cancel.
        Mock::given(method("POST"))
            .and(path("/fal-ai/denied"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"detail": "bad key"})))
            .mount(&server)
            .await;
        let e = fal.submit("fal-ai/denied", &json!({})).await.unwrap_err();
        assert_eq!(e.kind, ProviderErrorKind::AuthFailed);
        assert!(fal.submit("../etc", &json!({})).await.is_err());
        let sub = Submitted {
            request_id: "r3".into(),
            path: "/fal-ai/done/requests/r3".into(),
            queue_position: None,
        };
        Mock::given(method("PUT"))
            .and(path("/fal-ai/done/requests/r3/cancel"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"status": "ALREADY_COMPLETED"})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            fal.cancel(&sub).await.unwrap(),
            CancelOutcome::AlreadyCompleted
        );
    }

    #[tokio::test]
    async fn storage_upload_initiates_then_puts_without_the_key() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("POST"))
            .and(path("/storage/upload/initiate"))
            .and(query_param("storage_type", "fal-cdn-v3"))
            .and(header("authorization", "Key k-123"))
            .and(wiremock::matchers::body_json(
                json!({"content_type": "video/mp4", "file_name": "clip.mp4"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "upload_url": format!("{base}/put/abc?sig=1"),
                "file_url": "https://v3b.fal.media/files/b/abc/clip.mp4"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/put/abc"))
            .and(header("content-type", "video/mp4"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let fal = client(&server)
            .with_storage(&manifest(), SecretString::from("k-123".to_owned()), &base)
            .unwrap();
        assert!(fal.can_upload());
        let url = fal
            .upload(b"MP4DATA".to_vec(), "video/mp4", "clip.mp4")
            .await
            .unwrap();
        assert_eq!(url, "https://v3b.fal.media/files/b/abc/clip.mp4");
        let reqs = server.received_requests().await.unwrap();
        let put = reqs.iter().find(|r| r.method.as_str() == "PUT").unwrap();
        assert_eq!(put.body, b"MP4DATA");
        assert!(!put.headers.contains_key("authorization"));
        // Without fal's own key (HF router) there is no storage.
        assert!(!client(&server).can_upload());
        assert!(
            client(&server)
                .upload(vec![1], "image/png", "a.png")
                .await
                .is_err()
        );
    }

    /// Live: one call to the cheapest image model (paid, about $0.003). Runs only with
    /// `FAL_KEY` set and `cargo test -- --ignored live_fal`.
    #[tokio::test]
    #[ignore = "paid live call; needs FAL_KEY"]
    async fn live_fal_one_cheap_image() {
        let Ok(key) = std::env::var("FAL_KEY") else {
            return;
        };
        let fal = Fal::new(&manifest(), SecretString::from(key)).unwrap();
        let out = fal
            .run(
                DEFAULT_IMAGE_MODEL,
                &json!({"prompt": "a red dot", "image_size": "square", "num_inference_steps": 1}),
                Duration::from_secs(1),
                Duration::from_secs(120),
                &CancellationToken::new(),
                &|_| {},
            )
            .await
            .unwrap();
        assert!(!output_files(&out).is_empty(), "{out}");
    }

    #[tokio::test]
    async fn discovery_schema_and_price() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("expand", "openapi-3.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
                "endpoint_id": "fal-ai/x",
                "openapi": {"paths": {"/fal-ai/x": {"post": {"requestBody": {"content": {
                    "application/json": {"schema": {"required": ["prompt"], "properties": {"prompt": {}}}}}}}}}}
            }]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("category", "text-to-image"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"endpoint_id": "fal-ai/flux/schnell", "metadata": {"display_name": "FLUX.1 [schnell]", "category": "text-to-image"}}
            ], "has_more": false})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models/pricing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prices": [
                {"endpoint_id": "fal-ai/x", "unit_price": 0.003, "unit": "megapixels", "currency": "USD"}
            ]})))
            .mount(&server)
            .await;
        let fal = client(&server);
        let s = fal.input_schema("fal-ai/x").await.unwrap();
        assert_eq!(s["required"][0], "prompt");
        let m = fal.models(None, Some("text-to-image"), 10).await.unwrap();
        assert_eq!(m[0].endpoint_id, "fal-ai/flux/schnell");
        assert_eq!(m[0].category, "text-to-image");
        let p = fal.price("fal-ai/x").await.unwrap().unwrap();
        assert_eq!(p.unit, "megapixels");
    }
}
