//! Hugging Face beyond chat (chat itself is the `huggingface` manifest, OpenAI-compatible at
//! `router.huggingface.co/v1`):
//!
//! - Task inference on the HF Inference provider, `POST https://router.huggingface.co/hf-inference/models/<model>`
//!   (feature extraction at `.../models/<model>/pipeline/feature-extraction`); raw image/audio
//!   bodies for vision and speech tasks. Routes as in huggingface.js
//!   `packages/inference/src/providers/hf-inference.ts`; tasks listed at
//!   <https://huggingface.co/docs/inference-providers/providers/hf-inference>; text-to-image body
//!   at <https://huggingface.co/docs/inference-providers/tasks/text-to-image>.
//! - Other providers' media models (text-to-video, image-to-image, TTS) through the router's
//!   fal-ai route: see [`crate::fal::Fal::via_huggingface`]; the provider model id comes from
//!   `GET /api/models/<repo>?expand[]=inferenceProviderMapping`. Replicate, Together, WaveSpeed
//!   and Novita run through `router.huggingface.co/<provider>/...`: see [`Hf::run_routed`].
//! - Inference Endpoints (dedicated): the same task bodies posted to the user's endpoint URL
//!   (<https://huggingface.co/docs/inference-endpoints/index>).
//! - Hub API: `GET /api/models` (`search`, `author`, `filter`, `pipeline_tag`, `sort`, `limit`),
//!   `GET /api/models/<repo>[/revision/<rev>]`, `GET /api/models/<repo>/tree/<rev>?recursive=true`
//!   (LFS files carry `lfs.oid` = SHA-256). Reference: <https://huggingface.co/docs/hub/api> and
//!   <https://huggingface.co/.well-known/openapi.json>.
//! - Downloads: `GET https://huggingface.co/<repo>/resolve/<rev>/<file>` with `Range` for resume;
//!   the Hub answers with a redirect to its CDN, followed here without the token.
//!
//! The token is optional for the Hub (public repos) and required for inference.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};
use sha2::Digest as _;
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::ProviderManifest;
use crate::fal::Fal;
use crate::http::Http;

pub const HUB: &str = "https://huggingface.co";
pub const HF_INFERENCE: &str = "https://router.huggingface.co/hf-inference";
/// `HF_ROUTER_URL` of huggingface.js; a provider's route is `<router>/<provider>/<route>`.
pub const ROUTER: &str = "https://router.huggingface.co";
/// Task answers (images, JSON) beyond this are refused.
const MAX_TASK_BYTES: usize = 64 * 1024 * 1024;
const MAX_REDIRECTS: u8 = 5;

/// A task the HF Inference provider (or an Inference Endpoint) serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    TextToImage,
    SpeechToText,
    FeatureExtraction,
    ImageClassification,
    ObjectDetection,
}

/// Request body of a task: JSON (`{"inputs": ...}`) or raw bytes with their media type.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Json(Value),
    Bytes(Vec<u8>, String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub search: Option<String>,
    pub author: Option<String>,
    /// Tags such as `gguf`, `text-generation`, a library or a licence.
    pub filter: Vec<String>,
    pub pipeline_tag: Option<String>,
    /// `downloads` (default), `likes`, `trending_score`, `lastModified`.
    pub sort: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubModel {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    pub pipeline_tag: Option<String>,
    pub library: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 of LFS files; `None` for small git files (their oid is a git SHA-1).
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub path: PathBuf,
    pub verified: bool,
    /// Bytes that were already on disk from an earlier attempt.
    pub resumed_from: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRoute {
    pub provider: String,
    pub provider_id: String,
    pub task: String,
    pub live: bool,
}

#[derive(Debug, Clone)]
pub struct Hf {
    manifest: ProviderManifest,
    key: Option<SecretString>,
    hub: Http,
    infer: Http,
    /// `router.huggingface.co`: other providers' routes (`/<provider>/<their route>`).
    router: Http,
    /// `infer` is a dedicated Inference Endpoint (no `models/<id>` route).
    endpoint: bool,
}

/// What a routed provider answered: output file URLs, inline bytes, or text (speech-to-text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    Urls(Vec<String>),
    Bytes(Vec<u8>, String),
    Text(String),
}

/// Providers (other than `hf-inference` and `fal-ai`) Kor drives through the router, with the
/// Hub task names it supports for each. Request shapes follow huggingface.js
/// `packages/inference/src/providers/{replicate,together,wavespeed,novita}.ts`.
pub fn router_supports(provider: &str, task: &str) -> bool {
    matches!(
        (provider, task),
        (
            "replicate",
            "text-to-image"
                | "text-to-video"
                | "image-to-image"
                | "text-to-speech"
                | "automatic-speech-recognition"
        ) | ("together", "text-to-image" | "text-to-video")
            | (
                "wavespeed",
                "text-to-image" | "text-to-video" | "image-to-image"
            )
            | ("novita", "text-to-video")
    )
}

/// `https://host/a/b?x=1` → `/a/b?x=1` (no `..`), for polling through our own base.
fn path_and_query(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    let p = u.path();
    if p.contains("..") {
        return None;
    }
    Some(match u.query() {
        Some(q) => format!("{p}?{q}"),
        None => p.to_owned(),
    })
}

/// The `rel="next"` target of a `Link` header, as path and query.
fn next_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|p| p.trim().trim_start_matches("rel=").trim_matches('"') == "next")
            .then(|| path_and_query(url.trim().trim_start_matches('<').trim_end_matches('>')))?
    })
}

/// First string of `v` (a string or an array of strings).
fn first_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.iter().find_map(|x| x.as_str().map(str::to_owned)),
        _ => None,
    }
}

const ROUTER_POLL: Duration = Duration::from_secs(2);
const ROUTER_DEADLINE: Duration = Duration::from_secs(30 * 60);
/// Tree pages followed at most (1000 entries each).
const MAX_TREE_PAGES: usize = 200;

fn with_base(m: &ProviderManifest, base: &str) -> ProviderManifest {
    let mut m = m.clone();
    base.clone_into(&mut m.base_url);
    m.base_url_env = None;
    m
}

/// `owner/name` with the characters the Hub allows.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 96
            && !p.starts_with('.')
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

/// A relative file path inside a repo: no `..`, no absolute or backslash paths.
pub fn valid_file(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

fn valid_rev(rev: &str) -> bool {
    !rev.is_empty()
        && rev.len() <= 100
        && !rev.contains("..")
        && rev
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// `*.gguf`-style matching (only `*` and `?`), on the whole path.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some(b'*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some(b'?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => rec(&p[1..], &t[1..]),
            _ => false,
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

impl Hf {
    /// `manifest` is the `huggingface` provider's (Bearer `HF_TOKEN`); the token is optional.
    pub fn new(
        manifest: &ProviderManifest,
        key: Option<SecretString>,
    ) -> Result<Self, ProviderError> {
        Self::with_bases(manifest, key, HUB, HF_INFERENCE, false)
    }

    /// Task inference on a dedicated Inference Endpoint at `url` instead of the router.
    pub fn endpoint(
        manifest: &ProviderManifest,
        key: Option<SecretString>,
        url: &str,
    ) -> Result<Self, ProviderError> {
        if !url.starts_with("https://") {
            return Err(ProviderError::new(
                &manifest.id,
                ProviderErrorKind::InvalidRequest,
                "the endpoint URL must start with https://",
            ));
        }
        Self::with_bases(manifest, key, HUB, url, true)
    }

    pub fn with_bases(
        manifest: &ProviderManifest,
        key: Option<SecretString>,
        hub: &str,
        infer: &str,
        endpoint: bool,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            manifest: manifest.clone(),
            hub: Http::new(&with_base(manifest, hub), key.clone())?,
            infer: Http::new(&with_base(manifest, infer), key.clone())?,
            router: Http::new(&with_base(manifest, ROUTER), key.clone())?,
            key,
            endpoint,
        })
    }

    /// Point the provider routes at `base` (tests, proxies).
    pub fn with_router(mut self, base: &str) -> Result<Self, ProviderError> {
        self.router = Http::new(&with_base(&self.manifest, base), self.key.clone())?;
        Ok(self)
    }

    /// Run `route` (a live entry of [`Self::provider_mapping`] with [`router_supports`]) through
    /// the router, billed to this token's account. `file` is a `data:` URL (image or audio);
    /// `extra` holds the model's own inputs. `on_status` gets a line per poll.
    #[allow(clippy::too_many_lines)] // one match arm per provider shape, read side by side
    pub async fn run_routed(
        &self,
        route: &ProviderRoute,
        prompt: &str,
        file: Option<&str>,
        extra: &Value,
        cancel: &CancellationToken,
        on_status: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Routed, ProviderError> {
        let r = &self.router;
        if self.key.is_none() {
            return Err(r.err(
                ProviderErrorKind::AuthFailed,
                "inference needs a Hugging Face token (set HF_TOKEN or run `kor auth login huggingface`)",
            ));
        }
        let (p, task, id) = (
            route.provider.as_str(),
            route.task.as_str(),
            route.provider_id.as_str(),
        );
        if !router_supports(p, task) || id.is_empty() || id.contains("..") || id.contains('?') {
            return Err(r.err(
                ProviderErrorKind::InvalidRequest,
                format!("{p} ({task}) is not supported through the Hugging Face router"),
            ));
        }
        let mut body = extra.as_object().cloned().unwrap_or_default();
        let malformed = || {
            r.err(
                ProviderErrorKind::Protocol,
                format!("malformed answer from {p}"),
            )
        };
        match p {
            "replicate" => {
                let mut input = body;
                match task {
                    "automatic-speech-recognition" => {
                        input.insert("audio".into(), json!(file.unwrap_or_default()));
                    }
                    "text-to-speech" => {
                        input.insert("text".into(), json!(prompt));
                    }
                    _ => {
                        input.insert("prompt".into(), json!(prompt));
                    }
                }
                if task == "image-to-image"
                    && let Some(f) = file
                {
                    for (k, many) in [
                        ("image", false),
                        ("images", true),
                        ("input_image", false),
                        ("input_images", true),
                    ] {
                        input.insert(k.into(), if many { json!([f]) } else { json!(f) });
                    }
                }
                let (path, version) = match id.split_once(':') {
                    Some((_, v)) => ("replicate/v1/predictions".to_owned(), Some(v)),
                    None => (format!("replicate/v1/models/{id}/predictions"), None),
                };
                let mut payload = json!({ "input": input });
                if let Some(v) = version {
                    payload["version"] = json!(v);
                }
                crate::http::mask_body(&mut payload);
                on_status(&format!("running on {p}"));
                let req = r
                    .request(Method::POST, &path)
                    .header("content-type", "application/json")
                    .header("prefer", "wait")
                    .body(
                        serde_json::to_vec(&payload)
                            .map_err(|e| r.err(ProviderErrorKind::Protocol, e.to_string()))?,
                    );
                let v = tokio::select! {
                    v = async { r.read_json(r.send(req).await?).await } => v?,
                    () = cancel.cancelled() => return Err(r.err(ProviderErrorKind::Cancelled, "cancelled")),
                };
                if let Some(e) = v.get("error").and_then(Value::as_str) {
                    return Err(r.err(ProviderErrorKind::InvalidRequest, e));
                }
                let out = v.get("output");
                if task == "automatic-speech-recognition" {
                    let text = first_str(out)
                        .or_else(|| out.and_then(|o| first_str(o.get("transcription"))))
                        .or_else(|| out.and_then(|o| first_str(o.get("translation"))));
                    return text.map(Routed::Text).ok_or_else(malformed);
                }
                match first_str(out) {
                    Some(u) => Ok(Routed::Urls(vec![u])),
                    None => Err(r.err(
                        ProviderErrorKind::Timeout,
                        format!(
                            "{p} did not finish within its wait window (prediction {})",
                            v.get("id").and_then(Value::as_str).unwrap_or("?")
                        ),
                    )),
                }
            }
            "together" if task == "text-to-image" => {
                body.insert("model".into(), json!(id));
                body.insert("prompt".into(), json!(prompt));
                body.insert("response_format".into(), json!("base64"));
                on_status(&format!("running on {p}"));
                let v = tokio::select! {
                    v = async { r.read_json(r.post_plain_json("together/v1/images/generations", &Value::Object(body)).await?).await } => v?,
                    () = cancel.cancelled() => return Err(r.err(ProviderErrorKind::Cancelled, "cancelled")),
                };
                if let Some(b) = v.pointer("/data/0/b64_json").and_then(Value::as_str) {
                    use base64::Engine as _;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b)
                        .map_err(|e| r.err(ProviderErrorKind::Protocol, e.to_string()))?;
                    return Ok(Routed::Bytes(bytes, "image/jpeg".into()));
                }
                first_str(v.pointer("/data/0/url"))
                    .map(|u| Routed::Urls(vec![u]))
                    .ok_or_else(malformed)
            }
            _ => {
                // Asynchronous jobs: submit, then poll a result route until done.
                if file.is_some() && task == "image-to-image" {
                    let f = file.unwrap_or_default();
                    body.insert("image".into(), json!(f));
                    body.entry("images").or_insert_with(|| json!([f]));
                }
                body.insert("prompt".into(), json!(prompt));
                let submit = match p {
                    "together" => {
                        body.insert("model".into(), json!(id));
                        "together/v2/videos".to_owned()
                    }
                    "wavespeed" => format!("wavespeed/api/v3/{id}"),
                    _ => format!("novita/v3/async/{id}"),
                };
                let v = r
                    .read_json(r.post_plain_json(&submit, &Value::Object(body)).await?)
                    .await?;
                let poll = match p {
                    "together" => v
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|j| format!("together/v2/videos/{j}")),
                    "wavespeed" => v
                        .pointer("/data/urls/get")
                        .and_then(Value::as_str)
                        .and_then(path_and_query)
                        .map(|q| format!("wavespeed{q}")),
                    _ => v.get("task_id").and_then(Value::as_str).map(|t| {
                        format!(
                            "novita/v3/async/task-result?{}",
                            crate::fal::query_string(&[("task_id", t.to_owned())])
                        )
                    }),
                }
                .ok_or_else(malformed)?;
                let started = std::time::Instant::now();
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(ROUTER_POLL) => {}
                        () = cancel.cancelled() => return Err(r.err(ProviderErrorKind::Cancelled,
                            format!("cancelled; {p} may still finish and bill the job"))),
                    }
                    let v = r.get_json(&poll).await?;
                    let (status, url) = match p {
                        "together" => (v.get("status"), first_str(v.pointer("/outputs/video_url"))),
                        "wavespeed" => (
                            v.pointer("/data/status"),
                            first_str(v.pointer("/data/outputs")),
                        ),
                        _ => (
                            v.pointer("/task/status"),
                            first_str(v.pointer("/videos/0/video_url")),
                        ),
                    };
                    let status = status.and_then(Value::as_str).unwrap_or("unknown");
                    on_status(&format!("{p}: {}", status.to_ascii_lowercase()));
                    match status {
                        "completed" | "TASK_STATUS_SUCCEED" => {
                            return url.map(|u| Routed::Urls(vec![u])).ok_or_else(malformed);
                        }
                        "failed" | "TASK_STATUS_FAILED" => {
                            let why = first_str(v.pointer("/data/error"))
                                .or_else(|| first_str(v.pointer("/error/message")))
                                .or_else(|| first_str(v.pointer("/task/reason")))
                                .unwrap_or_else(|| "the job failed".into());
                            return Err(r.err(ProviderErrorKind::InvalidRequest, why));
                        }
                        _ if started.elapsed() > ROUTER_DEADLINE => {
                            return Err(r.err(
                                ProviderErrorKind::Timeout,
                                format!("{p} did not finish in 30 minutes"),
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// fal models billed to this HF account (router fal-ai route); needs the token.
    pub fn fal(&self) -> Result<Fal, ProviderError> {
        let key = self.key.clone().ok_or_else(|| {
            self.hub.err(
                ProviderErrorKind::AuthFailed,
                "inference needs a Hugging Face token (set HF_TOKEN or run `kor auth login huggingface`)",
            )
        })?;
        Fal::via_huggingface(&self.manifest, key)
    }

    fn repo(&self, repo: &str) -> Result<(), ProviderError> {
        if valid_repo(repo) {
            Ok(())
        } else {
            Err(self.hub.err(
                ProviderErrorKind::InvalidRequest,
                format!("`{repo}` is not a repo id (owner/name)"),
            ))
        }
    }

    pub async fn search(&self, q: &SearchQuery) -> Result<Vec<HubModel>, ProviderError> {
        let mut query: Vec<(&str, String)> = vec![
            ("limit", q.limit.clamp(1, 100).to_string()),
            ("sort", q.sort.clone().unwrap_or_else(|| "downloads".into())),
            ("direction", "-1".into()),
        ];
        for (k, v) in [
            ("search", &q.search),
            ("author", &q.author),
            ("pipeline_tag", &q.pipeline_tag),
        ] {
            if let Some(v) = v {
                query.push((k, v.clone()));
            }
        }
        query.extend(q.filter.iter().map(|f| ("filter", f.clone())));
        let v = self
            .hub
            .get_json(&format!("api/models?{}", crate::fal::query_string(&query)))
            .await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_model)
            .collect())
    }

    /// Model card metadata (`gated`, `pipeline_tag`, `cardData`, `siblings`, ...).
    pub async fn info(&self, repo: &str, rev: Option<&str>) -> Result<Value, ProviderError> {
        self.repo(repo)?;
        let path = match rev {
            Some(r) if valid_rev(r) => format!("api/models/{repo}/revision/{r}"),
            Some(r) => {
                return Err(self.hub.err(
                    ProviderErrorKind::InvalidRequest,
                    format!("bad revision `{r}`"),
                ));
            }
            None => format!("api/models/{repo}"),
        };
        self.hub
            .get_json(&path)
            .await
            .map_err(|e| gated_hint(repo, e))
    }

    /// Which inference providers serve `repo`, with their own model ids.
    pub async fn provider_mapping(&self, repo: &str) -> Result<Vec<ProviderRoute>, ProviderError> {
        self.repo(repo)?;
        let v = self
            .hub
            .get_json(&format!(
                "api/models/{repo}?expand[]=inferenceProviderMapping"
            ))
            .await?;
        Ok(parse_mapping(&v))
    }

    /// Every file of the repo at `rev` (default `main`), with size and SHA-256 for LFS files. The
    /// Hub pages the tree (1000 entries) with a `Link: <...>; rel="next"` header, as
    /// `huggingface_hub/utils/_pagination.py` follows it; only its path is joined to our base.
    pub async fn files(
        &self,
        repo: &str,
        rev: Option<&str>,
    ) -> Result<Vec<HubFile>, ProviderError> {
        self.repo(repo)?;
        let rev = rev.unwrap_or("main");
        if !valid_rev(rev) {
            return Err(self.hub.err(
                ProviderErrorKind::InvalidRequest,
                format!("bad revision `{rev}`"),
            ));
        }
        let mut page = format!("api/models/{repo}/tree/{rev}?recursive=true");
        let mut out = Vec::new();
        for _ in 0..MAX_TREE_PAGES {
            let resp = self
                .hub
                .send(self.hub.request(Method::GET, &page))
                .await
                .map_err(|e| gated_hint(repo, e))?;
            let next = next_link(resp.headers());
            out.extend(parse_tree(&self.hub.read_json(resp).await?));
            match next {
                Some(n) if n.starts_with("/api/") => page = n,
                _ => return Ok(out),
            }
        }
        Err(self.hub.err(
            ProviderErrorKind::Protocol,
            format!("more than {MAX_TREE_PAGES} pages of files"),
        ))
    }

    /// Download `file` into `dest_dir/<owner>--<name>/<path>`, resuming a `.part` left by an earlier
    /// attempt and checking the SHA-256 the Hub lists. `progress(done, total)`.
    #[allow(clippy::too_many_lines)] // resume, stream, verify, rename: one flow in order
    pub async fn download(
        &self,
        repo: &str,
        rev: Option<&str>,
        file: &HubFile,
        dest_dir: &Path,
        cancel: &CancellationToken,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<Downloaded, ProviderError> {
        self.repo(repo)?;
        let rev = rev.unwrap_or("main");
        if !valid_file(&file.path) || !valid_rev(rev) {
            return Err(self.hub.err(
                ProviderErrorKind::InvalidRequest,
                format!("bad file path `{}`", file.path),
            ));
        }
        let io = |e: std::io::Error| self.hub.err(ProviderErrorKind::Protocol, e.to_string());
        let dest = dest_dir.join(repo.replace('/', "--")).join(&file.path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let part = dest.with_extension(format!(
            "{}part",
            dest.extension()
                .map(|e| format!("{}.", e.to_string_lossy()))
                .unwrap_or_default()
        ));
        let mut have = tokio::fs::metadata(&part).await.map_or(0, |m| m.len());
        if have > file.size {
            tokio::fs::remove_file(&part).await.map_err(io)?;
            have = 0;
        }
        let resumed_from = have;
        if have < file.size || file.size == 0 {
            let resp = self.fetch(repo, rev, &file.path, have).await?;
            let append = match resp.status() {
                StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE => true,
                _ => {
                    have = 0;
                    false
                }
            };
            if resp.status() != StatusCode::RANGE_NOT_SATISFIABLE {
                let mut out = tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(append)
                    .truncate(!append)
                    .open(&part)
                    .await
                    .map_err(io)?;
                let mut resp = resp;
                loop {
                    let chunk = tokio::select! {
                        c = resp.chunk() => c.map_err(|e| self.hub.transport(&e))?,
                        () = cancel.cancelled() => {
                            out.flush().await.map_err(io)?;
                            return Err(self.hub.err(ProviderErrorKind::Cancelled,
                                "cancelled; run the same command again to resume"));
                        }
                    };
                    let Some(chunk) = chunk else { break };
                    have += chunk.len() as u64;
                    if file.size > 0 && have > file.size {
                        return Err(self.hub.err(
                            ProviderErrorKind::Protocol,
                            "server sent more bytes than listed",
                        ));
                    }
                    out.write_all(&chunk).await.map_err(io)?;
                    progress(have, file.size);
                }
                out.flush().await.map_err(io)?;
            }
        }
        let len = tokio::fs::metadata(&part).await.map_err(io)?.len();
        if file.size > 0 && len != file.size {
            return Err(self.hub.err(
                ProviderErrorKind::Unavailable,
                format!(
                    "incomplete download ({len} of {} bytes); run again to resume",
                    file.size
                ),
            ));
        }
        let verified = match &file.sha256 {
            Some(expected) => {
                let p = part.clone();
                let got = tokio::task::spawn_blocking(move || sha256_file(&p))
                    .await
                    .map_err(|e| self.hub.err(ProviderErrorKind::Protocol, e.to_string()))?
                    .map_err(io)?;
                if !got.eq_ignore_ascii_case(expected) {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(self.hub.err(
                        ProviderErrorKind::Protocol,
                        format!("checksum mismatch for {} (expected {expected}, got {got}); the partial file was removed", file.path),
                    ));
                }
                true
            }
            None => false,
        };
        tokio::fs::rename(&part, &dest).await.map_err(io)?;
        Ok(Downloaded {
            path: dest,
            verified,
            resumed_from,
        })
    }

    /// GET `resolve/<rev>/<file>` from `offset`, following the Hub's redirects: same-host ones with
    /// the token, other hosts (the CDN, presigned) without it.
    async fn fetch(
        &self,
        repo: &str,
        rev: &str,
        file: &str,
        offset: u64,
    ) -> Result<Response, ProviderError> {
        let range = format!("bytes={offset}-");
        let mut resp = self
            .hub
            .request(Method::GET, &format!("{repo}/resolve/{rev}/{file}"))
            .header(reqwest::header::RANGE, &range)
            .send()
            .await
            .map_err(|e| self.hub.transport(&e))?;
        let plain = kor_core::net::client_builder()
            .user_agent(format!("kor/{}", kor_core::VERSION))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS.into()))
            .build()
            .map_err(|e| self.hub.transport(&e))?;
        for _ in 0..MAX_REDIRECTS {
            if !resp.status().is_redirection() {
                break;
            }
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            resp = if loc.starts_with('/') {
                self.hub
                    .request(Method::GET, &loc)
                    .header(reqwest::header::RANGE, &range)
                    .send()
                    .await
                    .map_err(|e| self.hub.transport(&e))?
            } else if loc.starts_with("https://")
                || cfg!(test) && loc.starts_with("http://127.0.0.1")
            {
                plain
                    .get(&loc)
                    .header(reqwest::header::RANGE, &range)
                    .send()
                    .await
                    .map_err(|e| self.hub.transport(&e))?
            } else {
                return Err(self
                    .hub
                    .err(ProviderErrorKind::Protocol, "unexpected redirect target"));
            };
        }
        let st = resp.status();
        if st.is_success() || st == StatusCode::RANGE_NOT_SATISFIABLE {
            return Ok(resp);
        }
        let mut e = self.hub.err(
            ProviderError::kind_from_status(st.as_u16()),
            format!("download failed: HTTP {}", st.as_u16()),
        );
        e.http_status = Some(st.as_u16());
        Err(gated_hint(repo, e))
    }

    fn task_path(&self, model: &str, task: Task) -> Result<String, ProviderError> {
        if self.endpoint {
            return Ok(String::new());
        }
        self.repo(model)?;
        Ok(match task {
            Task::FeatureExtraction => format!("models/{model}/pipeline/feature-extraction"),
            _ => format!("models/{model}"),
        })
    }

    /// Run a task; returns the raw answer and its media type (an image for text-to-image, JSON
    /// otherwise).
    pub async fn infer(
        &self,
        model: &str,
        task: Task,
        body: Body,
    ) -> Result<(Vec<u8>, Option<String>), ProviderError> {
        let path = self.task_path(model, task)?;
        let req = self.infer.request(Method::POST, &path);
        let req = match body {
            Body::Json(mut v) => req.header("content-type", "application/json").body({
                crate::http::mask_body(&mut v);
                serde_json::to_vec(&v)
                    .map_err(|e| self.infer.err(ProviderErrorKind::Protocol, e.to_string()))?
            }),
            Body::Bytes(b, ct) => req.header("content-type", ct).body(b),
        };
        let mut resp = self.infer.send(req).await?;
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut out = Vec::new();
        while let Some(c) = resp.chunk().await.map_err(|e| self.infer.transport(&e))? {
            if out.len() + c.len() > MAX_TASK_BYTES {
                return Err(self
                    .infer
                    .err(ProviderErrorKind::Protocol, "answer larger than 64 MB"));
            }
            out.extend_from_slice(&c);
        }
        Ok((out, ct))
    }

    async fn infer_json(
        &self,
        model: &str,
        task: Task,
        body: Body,
    ) -> Result<Value, ProviderError> {
        let (b, _) = self.infer(model, task, body).await?;
        serde_json::from_slice(&b)
            .map_err(|e| self.infer.err(ProviderErrorKind::Protocol, e.to_string()))
    }

    /// Text-to-image: the image bytes and their media type.
    pub async fn text_to_image(
        &self,
        model: &str,
        prompt: &str,
        parameters: Value,
    ) -> Result<(Vec<u8>, Option<String>), ProviderError> {
        let mut body = json!({ "inputs": prompt });
        if parameters.as_object().is_some_and(|o| !o.is_empty()) {
            body["parameters"] = parameters;
        }
        self.infer(model, Task::TextToImage, Body::Json(body)).await
    }

    /// Speech-to-text of raw audio bytes (`audio/wav`, `audio/mpeg`, `audio/flac`, ...).
    pub async fn speech_to_text(
        &self,
        model: &str,
        audio: Vec<u8>,
        media_type: &str,
    ) -> Result<String, ProviderError> {
        let v = self
            .infer_json(
                model,
                Task::SpeechToText,
                Body::Bytes(audio, media_type.to_owned()),
            )
            .await?;
        v.get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                self.infer
                    .err(ProviderErrorKind::Protocol, "no text in the answer")
            })
    }

    /// Image classification or object detection of raw image bytes: the JSON answer
    /// (`[{label, score}]`, plus `box` for detection).
    pub async fn vision(
        &self,
        model: &str,
        task: Task,
        image: Vec<u8>,
        media_type: &str,
    ) -> Result<Value, ProviderError> {
        self.infer_json(model, task, Body::Bytes(image, media_type.to_owned()))
            .await
    }

    /// Embedding vectors for `texts`, in order (feature extraction; token-level answers are
    /// mean-pooled).
    pub async fn embed(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        let v = self
            .infer_json(
                model,
                Task::FeatureExtraction,
                Body::Json(json!({ "inputs": texts })),
            )
            .await?;
        parse_features(&v, texts.len()).ok_or_else(|| {
            self.infer
                .err(ProviderErrorKind::Protocol, "malformed embeddings answer")
        })
    }
}

/// 401/403 on a repo: say how to get access to a gated model.
fn gated_hint(repo: &str, mut e: ProviderError) -> ProviderError {
    if matches!(e.http_status, Some(401 | 403)) {
        e.message = format!(
            "{}; if the model is gated, accept its terms at {HUB}/{repo} with your account, \
             then set HF_TOKEN or run `kor auth login huggingface`",
            e.message
        );
    }
    e
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn parse_model(m: &Value) -> Option<HubModel> {
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(HubModel {
        id: s("id").or_else(|| s("modelId"))?,
        downloads: m.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        likes: m.get("likes").and_then(Value::as_u64).unwrap_or(0),
        pipeline_tag: s("pipeline_tag"),
        library: s("library_name"),
        tags: m
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub fn parse_tree(v: &Value) -> Vec<HubFile> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("file"))
        .filter_map(|e| {
            Some(HubFile {
                path: e.get("path")?.as_str()?.to_owned(),
                size: e
                    .pointer("/lfs/size")
                    .or_else(|| e.get("size"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                sha256: e
                    .pointer("/lfs/oid")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() == 64)
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub fn parse_mapping(v: &Value) -> Vec<ProviderRoute> {
    v.get("inferenceProviderMapping")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(provider, e)| {
            Some(ProviderRoute {
                provider: provider.clone(),
                provider_id: e.get("providerId")?.as_str()?.to_owned(),
                task: e
                    .get("task")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                live: e.get("status").and_then(Value::as_str) == Some("live"),
            })
        })
        .collect()
}

/// `gated` of a model: `Some("manual" | "auto")` when the user must accept terms.
pub fn gated(info: &Value) -> Option<String> {
    match info.get("gated") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Bool(true)) => Some("auto".into()),
        _ => None,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn as_vec(v: &Value) -> Option<Vec<f32>> {
    v.as_array()?
        .iter()
        .map(|x| x.as_f64().map(|f| f as f32))
        .collect()
}

/// 2-D (one vector per input), 3-D (per token: mean-pooled) or 1-D (a single input).
#[allow(clippy::cast_precision_loss)]
pub fn parse_features(v: &Value, count: usize) -> Option<Vec<Vec<f32>>> {
    let rows = v.as_array()?;
    if count == 1 && rows.first().is_some_and(Value::is_number) {
        return Some(vec![as_vec(v)?]);
    }
    let out: Vec<Vec<f32>> = rows
        .iter()
        .map(|r| {
            if r.as_array()?.first().is_some_and(Value::is_array) {
                let toks: Vec<Vec<f32>> =
                    r.as_array()?.iter().map(as_vec).collect::<Option<_>>()?;
                let n = toks.len().max(1) as f32;
                let dim = toks.first()?.len();
                Some(
                    (0..dim)
                        .map(|i| {
                            toks.iter()
                                .map(|t| t.get(i).copied().unwrap_or(0.0))
                                .sum::<f32>()
                                / n
                        })
                        .collect(),
                )
            } else {
                as_vec(r)
            }
        })
        .collect::<Option<_>>()?;
    (out.len() == count).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn manifest() -> ProviderManifest {
        ProviderManifest::parse(
            include_str!("../../../providers/huggingface.toml"),
            "huggingface.toml",
        )
        .unwrap()
    }

    fn client(server: &MockServer, key: bool) -> Hf {
        Hf::with_bases(
            &manifest(),
            key.then(|| SecretString::from("hf_test".to_owned())),
            &server.uri(),
            &server.uri(),
            false,
        )
        .unwrap()
    }

    #[test]
    fn ids_paths_and_globs() {
        assert!(valid_repo("TheBloke/Llama-2-7B-GGUF"));
        assert!(!valid_repo("a/b/c") && !valid_repo("../x") && !valid_repo("a"));
        assert!(valid_file("sub/model.Q4_K_M.gguf"));
        assert!(
            !valid_file("../x")
                && !valid_file("/etc/passwd")
                && !valid_file("a\\b")
                && !valid_file("C:x")
        );
        assert!(glob_match("*.gguf", "x/model.Q4.GGUF"));
        assert!(glob_match("*Q4_K_M*", "m.Q4_K_M.gguf"));
        assert!(!glob_match("*.gguf", "m.safetensors"));
    }

    #[test]
    fn features_shapes() {
        assert_eq!(
            parse_features(&json!([[1.0, 2.0], [3.0, 4.0]]), 2).unwrap()[1],
            vec![3.0, 4.0]
        );
        assert_eq!(
            parse_features(&json!([1.0, 2.0]), 1).unwrap(),
            vec![vec![1.0, 2.0]]
        );
        assert_eq!(
            parse_features(&json!([[[1.0, 0.0], [3.0, 2.0]]]), 1).unwrap(),
            vec![vec![2.0, 1.0]]
        );
        assert!(parse_features(&json!([[1.0]]), 2).is_none());
    }

    #[test]
    fn mapping_and_gating() {
        let v = json!({"inferenceProviderMapping": {"fal-ai": {"status": "live", "providerId": "fal-ai/flux/dev", "task": "text-to-image"}}});
        let m = parse_mapping(&v);
        assert_eq!(m[0].provider_id, "fal-ai/flux/dev");
        assert!(m[0].live);
        assert_eq!(
            gated(&json!({"gated": "manual"})).as_deref(),
            Some("manual")
        );
        assert_eq!(gated(&json!({"gated": false})), None);
    }

    #[tokio::test]
    async fn search_tasks_and_embeddings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("filter", "gguf"))
            .and(query_param("search", "qwen"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": "Qwen/Qwen3-GGUF", "downloads": 10, "likes": 2, "pipeline_tag": "text-generation", "tags": ["gguf"]}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/models/BAAI/bge-small-en-v1.5/pipeline/feature-extraction",
            ))
            .and(header("authorization", "Bearer hf_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([[0.1, 0.2], [0.3, 0.4]])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/openai/whisper-large-v3"))
            .and(header("content-type", "audio/wav"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "merhaba"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/stabilityai/sd"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/jpeg")
                    .set_body_bytes(b"JPG".to_vec()),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/bad/model"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(json!({"error": "Model is loading"})),
            )
            .mount(&server)
            .await;
        let hf = client(&server, true);
        let found = hf
            .search(&SearchQuery {
                search: Some("qwen".into()),
                filter: vec!["gguf".into()],
                limit: 5,
                ..SearchQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(found[0].id, "Qwen/Qwen3-GGUF");
        let e = hf
            .embed("BAAI/bge-small-en-v1.5", &["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(
            hf.speech_to_text("openai/whisper-large-v3", b"RIFF".to_vec(), "audio/wav")
                .await
                .unwrap(),
            "merhaba"
        );
        let (img, ct) = hf
            .text_to_image("stabilityai/sd", "a cat", json!({}))
            .await
            .unwrap();
        assert_eq!(
            (img.as_slice(), ct.as_deref()),
            (&b"JPG"[..], Some("image/jpeg"))
        );
        let err = hf
            .text_to_image("bad/model", "x", json!({}))
            .await
            .unwrap_err();
        assert!(err.message.contains("loading"));
    }

    /// Serves `body` honouring `Range: bytes=N-`.
    struct Ranged(Vec<u8>);
    impl Respond for Ranged {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let from = req
                .headers
                .get("range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes="))
                .and_then(|v| v.trim_end_matches('-').parse::<usize>().ok())
                .unwrap_or(0);
            if from >= self.0.len() {
                return ResponseTemplate::new(416);
            }
            ResponseTemplate::new(if from > 0 { 206 } else { 200 })
                .set_body_bytes(self.0[from..].to_vec())
        }
    }

    #[tokio::test]
    async fn download_follows_redirect_resumes_and_checks_sha() {
        let server = MockServer::start().await;
        let data = b"GGUF-model-bytes-0123456789".to_vec();
        let sha = {
            let mut h = sha2::Sha256::new();
            h.update(&data);
            hex(&h.finalize())
        };
        // The Hub redirects to a CDN host; the CDN must not receive the token.
        Mock::given(method("GET"))
            .and(path("/o/m/resolve/main/m.gguf"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/cdn/m.gguf", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cdn/m.gguf"))
            .respond_with(Ranged(data.clone()))
            .mount(&server)
            .await;
        let hf = client(&server, true);
        let dir = tempfile::tempdir().unwrap();
        let file = HubFile {
            path: "m.gguf".into(),
            size: data.len() as u64,
            sha256: Some(sha.clone()),
        };
        // A previous attempt left the first 10 bytes.
        let part = dir.path().join("o--m").join("m.gguf.part");
        std::fs::create_dir_all(part.parent().unwrap()).unwrap();
        std::fs::write(&part, &data[..10]).unwrap();
        let got = hf
            .download(
                "o/m",
                None,
                &file,
                dir.path(),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await
            .unwrap();
        assert!(got.verified);
        assert_eq!(got.resumed_from, 10);
        assert_eq!(std::fs::read(&got.path).unwrap(), data);
        let cdn_calls = server.received_requests().await.unwrap();
        assert!(
            cdn_calls
                .iter()
                .filter(|r| r.url.path().starts_with("/cdn/"))
                .all(|r| !r.headers.contains_key("authorization"))
        );

        // A wrong checksum removes the partial file.
        let bad = HubFile {
            path: "m.gguf".into(),
            size: data.len() as u64,
            sha256: Some("0".repeat(64)),
        };
        let dir2 = tempfile::tempdir().unwrap();
        let e = hf
            .download(
                "o/m",
                None,
                &bad,
                dir2.path(),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await
            .unwrap_err();
        assert!(e.message.contains("checksum"));
        assert!(!dir2.path().join("o--m").join("m.gguf.part").exists());

        // Gated repo: 403 explains how to get access.
        Mock::given(method("GET"))
            .and(path("/o/gated/resolve/main/m.gguf"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let e = hf
            .download(
                "o/gated",
                None,
                &file,
                dir2.path(),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await
            .unwrap_err();
        assert!(e.message.contains("accept its terms"));
    }

    #[tokio::test]
    async fn tree_lists_lfs_checksums() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models/o/m/tree/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"type": "directory", "path": "sub", "oid": "x", "size": 0},
                {"type": "file", "path": "config.json", "oid": "abc", "size": 12},
                {"type": "file", "path": "m.gguf", "oid": "def", "size": 5, "lfs": {"oid": "a".repeat(64), "size": 5, "pointerSize": 134}}
            ])))
            .mount(&server)
            .await;
        let files = client(&server, false).files("o/m", None).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].sha256, None);
        assert_eq!(files[1].sha256.as_deref(), Some("a".repeat(64).as_str()));
    }

    #[tokio::test]
    async fn tree_follows_link_pagination_on_our_own_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models/o/big/tree/main"))
            .and(query_param("cursor", "p2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"type": "file", "path": "b.bin", "size": 2}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/models/o/big/tree/main"))
            .respond_with(
                ResponseTemplate::new(200)
                    // A foreign host in the link: only its path is used.
                    .insert_header(
                        "link",
                        "<https://elsewhere.example/api/models/o/big/tree/main?recursive=true&cursor=p2>; rel=\"next\"",
                    )
                    .set_body_json(json!([{"type": "file", "path": "a.bin", "size": 1}])),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let files = client(&server, false).files("o/big", None).await.unwrap();
        let names: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["a.bin", "b.bin"]);
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "link",
            "<https://x/api/a?c=1>; rel=\"prev\", <https://x/api/b?c=2>; rel=\"next\""
                .parse()
                .unwrap(),
        );
        assert_eq!(next_link(&h).as_deref(), Some("/api/b?c=2"));
    }

    fn route(provider: &str, id: &str, task: &str) -> ProviderRoute {
        ProviderRoute {
            provider: provider.into(),
            provider_id: id.into(),
            task: task.into(),
            live: true,
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one mock set per provider
    async fn router_runs_other_providers_per_route() {
        let server = MockServer::start().await;
        let base = server.uri();
        // Replicate: synchronous prediction with `Prefer: wait`, prompt under `input`.
        Mock::given(method("POST"))
            .and(path("/replicate/v1/models/black-forest-labs/flux-dev/predictions"))
            .and(header("authorization", "Bearer hf_test"))
            .and(header("prefer", "wait"))
            .and(wiremock::matchers::body_partial_json(json!({"input": {"prompt": "cat", "seed": 1}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "p1", "status": "succeeded", "output": ["https://replicate.delivery/x.webp"]})))
            .mount(&server)
            .await;
        // Together video: submit, then poll `v2/videos/<id>` on the router.
        Mock::given(method("POST"))
            .and(path("/together/v2/videos"))
            .and(wiremock::matchers::body_partial_json(
                json!({"model": "Wan-AI/Wan2.2", "prompt": "waves"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "j1"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/together/v2/videos/j1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "completed", "outputs": {"video_url": "https://cdn.together/v.mp4"}})))
            .mount(&server)
            .await;
        // WaveSpeed: the poll URL's host is replaced by the router's `/wavespeed`.
        Mock::given(method("POST"))
            .and(path("/wavespeed/api/v3/wavespeed-ai/flux-dev"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {
                "urls": {"get": "https://api.wavespeed.ai/api/v3/predictions/w1/result"}}})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/wavespeed/api/v3/predictions/w1/result"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {
                "status": "failed", "error": "nsfw"}})))
            .mount(&server)
            .await;
        let hf = client(&server, true).with_router(&base).unwrap();
        let none = CancellationToken::new();
        let seen = std::sync::Mutex::new(Vec::new());
        let on = |s: &str| seen.lock().unwrap().push(s.to_owned());
        let r = hf
            .run_routed(
                &route("replicate", "black-forest-labs/flux-dev", "text-to-image"),
                "cat",
                None,
                &json!({"seed": 1}),
                &none,
                &on,
            )
            .await
            .unwrap();
        assert_eq!(
            r,
            Routed::Urls(vec!["https://replicate.delivery/x.webp".into()])
        );
        let r = hf
            .run_routed(
                &route("together", "Wan-AI/Wan2.2", "text-to-video"),
                "waves",
                None,
                &json!({}),
                &none,
                &on,
            )
            .await
            .unwrap();
        assert_eq!(r, Routed::Urls(vec!["https://cdn.together/v.mp4".into()]));
        assert!(
            seen.lock()
                .unwrap()
                .contains(&"together: completed".to_owned())
        );
        let e = hf
            .run_routed(
                &route("wavespeed", "wavespeed-ai/flux-dev", "text-to-image"),
                "x",
                None,
                &json!({}),
                &none,
                &on,
            )
            .await
            .unwrap_err();
        assert_eq!(e.message, "nsfw");
        // Unsupported pairs and a missing token are refused before any request.
        assert!(!router_supports("cerebras", "text-to-image"));
        assert!(
            hf.run_routed(
                &route("novita", "x/y", "text-to-image"),
                "x",
                None,
                &json!({}),
                &none,
                &on
            )
            .await
            .is_err()
        );
        let anon = client(&server, false).with_router(&base).unwrap();
        let e = anon
            .run_routed(
                &route("replicate", "a/b", "text-to-image"),
                "x",
                None,
                &json!({}),
                &none,
                &on,
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, ProviderErrorKind::AuthFailed);
    }

    /// Live and free: Hub search, model info, provider mapping and a paged tree listing. Runs
    /// with `cargo test -- --ignored live_hub` (`HF_TOKEN` optional; public repos only).
    #[tokio::test]
    #[ignore = "live network"]
    async fn live_hub_search_info_and_mapping() {
        let token = std::env::var("HF_TOKEN").ok().map(SecretString::from);
        let hf = Hf::new(&manifest(), token).unwrap();
        let found = hf
            .search(&SearchQuery {
                search: Some("whisper".into()),
                limit: 3,
                ..SearchQuery::default()
            })
            .await
            .unwrap();
        assert!(!found.is_empty());
        let info = hf.info("openai/whisper-large-v3", None).await.unwrap();
        assert_eq!(info["id"], "openai/whisper-large-v3");
        let routes = hf
            .provider_mapping("black-forest-labs/FLUX.1-dev")
            .await
            .unwrap();
        assert!(
            routes.iter().any(|r| r.task == "text-to-image"),
            "{routes:?}"
        );
        let files = hf.files("openai/whisper-large-v3", None).await.unwrap();
        assert!(files.iter().any(|f| f.path == "config.json"));
    }
}
