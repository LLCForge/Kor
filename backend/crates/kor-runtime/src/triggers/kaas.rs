//! `KaaS` API (spec §11.2) on the `kor daemon` listener: `POST /api/v1/tasks` queues an agent task
//! on a project (the daemon's queue, runner, budget and approval policy), `GET …/tasks/<id>`,
//! `POST …/tasks/<id>/cancel`, a WebSocket event stream at `GET …/tasks/<id>/events`, and an
//! HMAC-signed callback when the task ends. `POST /v1/chat/completions` (`OpenAI`-compatible,
//! streaming and not) serves Kor itself as the model `kor`, read-only in a project.
//!
//! Every route needs `Authorization: Bearer <key>`; keys (`kor daemon key --create`) are stored
//! as SHA-256 hashes in `<daemon dir>/api-keys.json` and rate-limited per key. Bodies are capped by
//! the daemon (1 MiB); the listener stays on loopback unless `--allow-remote`.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::{SinkExt as _, StreamExt as _};
use http_body_util::{BodyExt as _, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest};
use kor_config::Approval;
use kor_core::{Message, Role};
use kor_providers::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::{Body, Fire, MAX_BODY, Resp, Shared, enqueue_job, err, jresp, log, project_repos};
use crate::triggers::{Defined, Job, Status, notify, runner, webhook};

/// Requests per key: burst, then this many per second.
const BURST: f64 = 60.0;
const REFILL_PER_SEC: f64 = 1.0;
/// Chat completions running at once (each is a full agent turn).
const MAX_CHATS: usize = 4;
const MAX_MESSAGES: usize = 500;
const MAX_CALLBACK: usize = 2048;
const MAX_BUDGET_USD: f64 = 1000.0;
/// `model` of the chat endpoint (`kor`, or `kor/<provider>/<model>` to pick the model behind it).
pub const MODEL: &str = "kor";

// ---------------------------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------------------------

/// A stored key: never the key itself, only its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// Hex SHA-256 of the full key.
    pub hash: String,
    /// Project the key's tasks and chats run in when the request names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    pub created: u64,
}

pub fn keys_path(dir: &Path) -> PathBuf {
    dir.join("api-keys.json")
}

pub fn list_keys(dir: &Path) -> Vec<ApiKey> {
    std::fs::read_to_string(keys_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A new key named `name`; the stored entry and the key, shown to the user once.
///
/// # Errors
/// A bad name or project, no random source, or the file cannot be written.
pub fn create_key(
    dir: &Path,
    name: &str,
    project: Option<&Path>,
) -> Result<(ApiKey, String), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("name: 1-80 printable characters".into());
    }
    let project = match project {
        Some(p) => {
            Some(crate::triggers::canon(p).ok_or_else(|| format!("{}: not found", p.display()))?)
        }
        None => None,
    };
    let secret = format!("kor_{}", notify::random_hex(24)?);
    let key = ApiKey {
        id: notify::random_hex(4)?,
        name: name.to_owned(),
        hash: hash(&secret),
        project,
        created: crate::triggers::now_ms(),
    };
    let mut all = list_keys(dir);
    all.push(key.clone());
    crate::triggers::write_json(&keys_path(dir), &all)?;
    Ok((key, secret))
}

/// Remove key `id`; `false` when there is none.
///
/// # Errors
/// The file cannot be written.
pub fn revoke_key(dir: &Path, id: &str) -> Result<bool, String> {
    let mut all = list_keys(dir);
    let before = all.len();
    all.retain(|k| k.id != id);
    if all.len() == before {
        return Ok(false);
    }
    crate::triggers::write_json(&keys_path(dir), &all).map(|()| true)
}

/// The key a bearer token belongs to (read from disk each time: a revoke takes effect at once).
fn authenticate(dir: &Path, token: &str) -> Option<ApiKey> {
    if token.is_empty() {
        return None;
    }
    let h = hash(token);
    list_keys(dir)
        .into_iter()
        .find(|k| bool::from(k.hash.as_bytes().ct_eq(h.as_bytes())))
}

// ---------------------------------------------------------------------------------------------
// Daemon-side state
// ---------------------------------------------------------------------------------------------

/// Token buckets by key id.
#[derive(Debug, Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Limiter {
    /// Take one request from `key`'s bucket; `false` when it is empty.
    pub fn allow(&self, key: &str, now: Instant) -> bool {
        let mut b = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let (tokens, last) = b.entry(key.to_owned()).or_insert((BURST, now));
        let refill = now.saturating_duration_since(*last).as_secs_f64() * REFILL_PER_SEC;
        *tokens = (*tokens + refill).min(BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct Kaas {
    limiter: Limiter,
    /// Callback URL and signing secret by job id (in memory: a restart fails the task anyway).
    callbacks: Mutex<HashMap<String, (String, String)>>,
    chats: Arc<tokio::sync::Semaphore>,
}

impl Default for Kaas {
    fn default() -> Self {
        Self {
            limiter: Limiter::default(),
            callbacks: Mutex::default(),
            chats: Arc::new(tokio::sync::Semaphore::new(MAX_CHATS)),
        }
    }
}

/// What a client sees of a task (never the callback secret, the worktree or the transcript).
pub fn view(j: &Job) -> Value {
    json!({
        "id": j.id,
        "status": j.status,
        "created": j.created,
        "started": j.started,
        "finished": j.finished,
        "summary": j.summary,
        "error": j.error,
        "cost_usd": j.cost_usd,
        "cost_estimated": j.cost_estimated,
        "approvals": j.approvals,
        "branch": j.branch,
        "pr_url": j.pr_url,
    })
}

/// After a job ends: its signed callback, sent in the background (3 tries).
pub(super) fn finished(s: &Shared, job: &Job) {
    let cb = s
        .kaas
        .callbacks
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&job.id);
    let Some((url, secret)) = cb else { return };
    let body = json!({ "event": "task.finished", "task": view(job) }).to_string();
    let id = job.id.clone();
    tokio::spawn(async move {
        if let Err(e) = send_callback(&url, &secret, &body).await {
            log(&format!("callback of task {id} failed: {e}"));
        }
    });
}

/// POST `body` to `url` with `X-Kor-Signature: sha256=<hmac>` under `secret`.
///
/// # Errors
/// Every attempt failed.
pub async fn send_callback(url: &str, secret: &str, body: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let sig = webhook::sign(secret.as_bytes(), body.as_bytes());
    let mut last = String::new();
    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
        }
        match client
            .post(url)
            .header("content-type", "application/json")
            .header("x-kor-event", "task.finished")
            .header("x-kor-signature", &sig)
            .body(body.to_owned())
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last = format!("HTTP {}", r.status()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

// ---------------------------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------------------------

/// `OpenAI`-style error body for `/v1/*`.
fn oai_err(status: u16, kind: &str, msg: &str) -> Resp {
    jresp(
        status,
        &json!({ "error": { "message": msg, "type": kind, "code": Value::Null } }),
    )
}

pub(super) async fn handle(s: &Arc<Shared>, req: Request<Incoming>) -> Resp {
    let path = req.uri().path().to_owned();
    let openai = path.starts_with("/v1/");
    let fail = |status: u16, kind: &str, msg: &str| {
        if openai {
            oai_err(status, kind, msg)
        } else {
            err(status, msg)
        }
    };
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|a| a.strip_prefix("Bearer "))
        .unwrap_or_default()
        .trim()
        .to_owned();
    let Some(key) = authenticate(&s.env.dir, &token) else {
        let mut r = fail(401, "invalid_request_error", "invalid or missing API key");
        r.headers_mut()
            .insert("www-authenticate", HeaderValue::from_static("Bearer"));
        return r;
    };
    if !s.kaas.limiter.allow(&key.id, Instant::now()) {
        let mut r = fail(429, "rate_limit_error", "rate limit exceeded");
        r.headers_mut()
            .insert("retry-after", HeaderValue::from_static("1"));
        return r;
    }
    let method = req.method().clone();
    let project_header = req
        .headers()
        .get("x-kor-project")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (&method, segs.as_slice()) {
        (&Method::GET, ["v1", "models"]) => jresp(
            200,
            &json!({ "object": "list", "data": [{ "id": MODEL, "object": "model", "owned_by": "kor" }] }),
        ),
        (&Method::GET, ["api", "v1", "tasks", id, "events"]) => {
            let id = (*id).to_owned();
            events(s, &key, &id, req)
        }
        (&Method::GET, ["api", "v1", "tasks", id]) => match owned(s, &key, id) {
            Some(j) => jresp(200, &view(&j)),
            None => err(404, "no such task"),
        },
        (&Method::POST, rest) => {
            let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return fail(413, "invalid_request_error", "body too large"),
            };
            match rest {
                ["v1", "chat", "completions"] => {
                    chat(s, &key, project_header.as_deref(), &body).await
                }
                ["api", "v1", "tasks"] => create(s, &key, &body),
                ["api", "v1", "tasks", id, "cancel"] => cancel(s, &key, id),
                _ => fail(404, "invalid_request_error", "not found"),
            }
        }
        _ => fail(404, "invalid_request_error", "not found"),
    }
}

/// Job `id` when `key` created it.
fn owned(s: &Shared, key: &ApiKey, id: &str) -> Option<Job> {
    crate::triggers::load_job(&s.env.dir, id).filter(|j| j.api_key.as_deref() == Some(&key.id))
}

/// The project a request may run in: the key's own, or a watched project (`kor triggers`)
/// named by path or remote URL. Anything else is refused.
fn project(s: &Shared, key: &ApiKey, repo: &str) -> Result<PathBuf, String> {
    let repo = repo.trim();
    if repo.is_empty() {
        return key
            .project
            .clone()
            .ok_or_else(|| "no project: create the key with --project or name one".into());
    }
    let mut allowed = crate::triggers::projects(&s.env.dir);
    allowed.extend(key.project.clone());
    let found = match crate::triggers::canon(Path::new(repo)) {
        Some(c) => allowed.into_iter().find(|p| *p == c),
        None => allowed
            .into_iter()
            .find(|p| project_repos(p).iter().any(|r| webhook::same_repo(r, repo))),
    };
    found.ok_or_else(|| format!("{repo}: not a project of this key or the daemon"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTask {
    task: String,
    /// Remote URL (`github.com/acme/app`) or path of a watched project; default: the key's.
    #[serde(default)]
    repo: String,
    #[serde(default)]
    budget_usd: f64,
    #[serde(default)]
    model: String,
    #[serde(default)]
    callback: String,
    #[serde(default)]
    approval: Approval,
    #[serde(default)]
    pr: bool,
}

fn create(s: &Shared, key: &ApiKey, body: &[u8]) -> Resp {
    let t: NewTask = match serde_json::from_slice(body) {
        Ok(t) => t,
        Err(e) => return err(400, &format!("bad request: {e}")),
    };
    if t.task.trim().is_empty() || t.task.chars().count() > crate::triggers::MAX_TASK {
        return err(400, "task: required, at most 10000 characters");
    }
    if !t.budget_usd.is_finite() || !(0.0..=MAX_BUDGET_USD).contains(&t.budget_usd) {
        return err(400, "budget_usd: 0-1000");
    }
    if t.model.chars().count() > 200 || t.model.chars().any(char::is_control) {
        return err(400, "model: at most 200 characters");
    }
    let callback = t.callback.trim();
    if !callback.is_empty() {
        let ok = callback.len() <= MAX_CALLBACK
            && reqwest::Url::parse(callback)
                .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some());
        if !ok {
            return err(400, "callback: an http(s) URL");
        }
    }
    let root = match project(s, key, &t.repo) {
        Ok(r) => r,
        Err(e) => return err(404, &e),
    };
    let mut trigger = runner::adhoc("api", &t.task, &t.model, t.approval);
    trigger.budget_usd = t.budget_usd;
    trigger.pr = t.pr;
    let fire = Fire {
        def: Defined {
            name: "api".into(),
            root,
            origin: PathBuf::new(),
            trigger,
            error: None,
        },
        detail: format!("POST /api/v1/tasks ({})", key.name),
        base: None,
        sha: None,
    };
    let mut job = match runner::new_job(&fire) {
        Ok(j) => j,
        Err(e) => return err(500, &e),
    };
    job.event = "api".into();
    job.api_key = Some(key.id.clone());
    let secret = if callback.is_empty() {
        None
    } else {
        match notify::random_hex(32) {
            Ok(x) => {
                s.kaas
                    .callbacks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(job.id.clone(), (callback.to_owned(), x.clone()));
                Some(x)
            }
            Err(e) => return err(500, &e),
        }
    };
    match enqueue_job(s, fire, job) {
        Ok(j) => {
            kor_agent::audit::record(
                "kaas.task",
                json!({ "job": j.id, "key": key.id, "root": j.root }),
            );
            let mut v = view(&j);
            v["events"] = json!(format!("/api/v1/tasks/{}/events", j.id));
            if let Some(x) = secret {
                v["callback_secret"] = json!(x);
            }
            jresp(202, &v)
        }
        Err(e) => {
            s.kaas
                .callbacks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|_, (u, _)| u != callback);
            err(429, &e)
        }
    }
}

fn cancel(s: &Shared, key: &ApiKey, id: &str) -> Resp {
    let Some(j) = owned(s, key, id) else {
        return err(404, "no such task");
    };
    if matches!(j.status, Status::Done | Status::Failed) {
        return err(409, "task already finished");
    }
    if let Some(run) = s.env.live.get(id) {
        run.cancel.cancel();
    }
    kor_agent::audit::record("kaas.cancel", json!({ "job": id, "key": key.id }));
    jresp(202, &json!({ "id": id, "cancelling": true }))
}

// ---------------------------------------------------------------------------------------------
// WebSocket event stream
// ---------------------------------------------------------------------------------------------

fn events(s: &Arc<Shared>, key: &ApiKey, id: &str, req: Request<Incoming>) -> Resp {
    let Some(job) = owned(s, key, id) else {
        return err(404, "no such task");
    };
    let h = |n: &str| {
        req.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase)
    };
    let ws_key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (Some(ws_key), true, true) = (
        ws_key,
        h("upgrade").is_some_and(|u| u == "websocket"),
        h("sec-websocket-version").as_deref() == Some("13"),
    ) else {
        return err(426, "WebSocket upgrade required");
    };
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(ws_key.as_bytes());
    // Subscribe before the upgrade so no event between now and the handshake is lost.
    let rx = s.env.live.get(id).map(|r| r.events.subscribe());
    let (s2, id) = (s.clone(), id.to_owned());
    tokio::spawn(async move {
        let Ok(up) = hyper::upgrade::on(req).await else {
            return;
        };
        let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            TokioIo::new(up),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        stream_events(&s2, &id, job, rx, ws).await;
    });
    let mut r = Response::new(http_body_util::Empty::<Bytes>::new().boxed_unsync());
    *r.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let hs = r.headers_mut();
    hs.insert("upgrade", HeaderValue::from_static("websocket"));
    hs.insert("connection", HeaderValue::from_static("Upgrade"));
    if let Ok(v) = HeaderValue::from_str(&accept) {
        hs.insert("sec-websocket-accept", v);
    }
    r
}

async fn stream_events<S>(
    s: &Shared,
    id: &str,
    job: Job,
    rx: Option<tokio::sync::broadcast::Receiver<Value>>,
    mut ws: tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let send = |v: Value| WsMessage::text(v.to_string());
    let done = |s: &Shared| {
        let j = crate::triggers::load_job(&s.env.dir, id).unwrap_or_default();
        json!({ "type": "done", "task": view(&j) })
    };
    if ws
        .send(send(json!({ "type": "task", "task": view(&job) })))
        .await
        .is_err()
    {
        return;
    }
    let Some(mut rx) = rx.filter(|_| !matches!(job.status, Status::Done | Status::Failed)) else {
        let _ = ws.send(send(done(s))).await;
        let _ = ws.close(None).await;
        return;
    };
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(v) => {
                    let end = v["type"] == "status" && matches!(v["status"].as_str(), Some("done" | "failed"));
                    if ws.send(send(v)).await.is_err() {
                        return;
                    }
                    if end {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let _ = ws.send(send(json!({ "type": "lagged", "missed": n }))).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            msg = ws.next() => match msg {
                Some(Ok(WsMessage::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = ws.send(send(done(s))).await;
    let _ = ws.close(None).await;
}

// ---------------------------------------------------------------------------------------------
// OpenAI-compatible chat completions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChatReq {
    model: String,
    messages: Vec<ChatMsg>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatMsg {
    role: String,
    #[serde(default)]
    content: Value,
}

/// Text of an `OpenAI` content field: a string or `[{type: text, text}]` parts.
fn content_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Earlier turns, client system text and the prompt (the last user message).
fn split_messages(msgs: &[ChatMsg]) -> Result<(Vec<Message>, String, String), String> {
    let Some((last, before)) = msgs.split_last() else {
        return Err("messages: at least one".into());
    };
    if last.role != "user" {
        return Err("messages: the last one must be from the user".into());
    }
    let mut history = Vec::new();
    let mut system = Vec::new();
    for m in before {
        let text = content_text(&m.content);
        match m.role.as_str() {
            "system" | "developer" => system.push(text),
            "user" => history.push(Message::text(Role::User, text)),
            "assistant" if !text.is_empty() => history.push(Message::text(Role::Assistant, text)),
            _ => {}
        }
    }
    Ok((history, system.join("\n\n"), content_text(&last.content)))
}

/// Answers every permission request with no: the chat endpoint is read-only.
#[derive(Debug)]
struct ReadOnly;

#[async_trait::async_trait]
impl Approver for ReadOnly {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
    fn surface(&self) -> &'static str {
        "api"
    }
}

fn chunk(id: &str, created: u64, delta: &Value, finish: Option<&str>) -> Bytes {
    let v = json!({
        "id": id, "object": "chat.completion.chunk", "created": created, "model": MODEL,
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
    });
    Bytes::from(format!("data: {v}\n\n"))
}

#[allow(clippy::too_many_lines)] // plain and streaming answers share the setup
async fn chat(s: &Arc<Shared>, key: &ApiKey, project_header: Option<&str>, body: &[u8]) -> Resp {
    let r: ChatReq = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return oai_err(400, "invalid_request_error", &format!("bad request: {e}")),
    };
    let inner = match r.model.strip_prefix(MODEL) {
        Some("") => String::new(),
        Some(rest) if rest.starts_with('/') && rest.len() <= 201 => rest[1..].to_owned(),
        _ => {
            return oai_err(
                404,
                "invalid_request_error",
                "model: `kor` or `kor/<provider>/<model>`",
            );
        }
    };
    if r.messages.len() > MAX_MESSAGES {
        return oai_err(400, "invalid_request_error", "too many messages");
    }
    let (history, system, prompt) = match split_messages(&r.messages) {
        Ok(x) => x,
        Err(e) => return oai_err(400, "invalid_request_error", &e),
    };
    let root = match project(s, key, project_header.unwrap_or_default()) {
        Ok(p) => p,
        Err(e) => return oai_err(400, "invalid_request_error", &e),
    };
    let Ok(permit) = s.kaas.chats.clone().try_acquire_owned() else {
        return oai_err(429, "rate_limit_error", "too many chats at once");
    };
    let id = match notify::random_hex(12) {
        Ok(x) => format!("chatcmpl-{x}"),
        Err(e) => return oai_err(500, "server_error", &e),
    };
    let created = crate::triggers::now_ms() / 1000;
    let t = runner::adhoc("chat", "", &inner, Approval::Deny);
    let cancel = CancellationToken::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let s2 = s.clone();
    let id2 = id.clone();
    let c2 = cancel.clone();
    let run = tokio::spawn(async move {
        let res = runner::one_shot(
            &s2.env.l,
            s2.env.factory.as_ref(),
            &root,
            &t,
            &id2,
            history,
            &system,
            prompt,
            &ReadOnly,
            &tx,
            c2,
        )
        .await;
        drop(permit);
        res
    });
    if !r.stream {
        while rx.recv().await.is_some() {}
        return match run.await {
            Ok(Ok((text, u))) => jresp(
                200,
                &json!({
                    "id": id, "object": "chat.completion", "created": created, "model": MODEL,
                    "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
                    "usage": { "prompt_tokens": u.input, "completion_tokens": u.output, "total_tokens": u.input + u.output },
                }),
            ),
            Ok(Err(e)) => oai_err(502, "server_error", &e),
            Err(_) => oai_err(500, "server_error", "chat task failed"),
        };
    }
    let (btx, brx) = tokio::sync::mpsc::channel::<Bytes>(64);
    tokio::spawn(async move {
        let mut out = vec![chunk(
            &id,
            created,
            &json!({ "role": "assistant", "content": "" }),
            None,
        )];
        loop {
            for b in out.drain(..) {
                if btx.send(b).await.is_err() {
                    // The client went away: stop the agent.
                    cancel.cancel();
                    return;
                }
            }
            match rx.recv().await {
                Some(AgentEvent::Text(t)) => {
                    out.push(chunk(&id, created, &json!({ "content": t }), None));
                }
                Some(_) => {}
                None => break,
            }
        }
        let end = match run.await {
            Ok(Ok(_)) => chunk(&id, created, &json!({}), Some("stop")),
            Ok(Err(e)) => Bytes::from(format!(
                "data: {}\n\n",
                json!({ "error": { "message": e, "type": "server_error" } })
            )),
            Err(_) => Bytes::new(),
        };
        let _ = btx.send(end).await;
        let _ = btx.send(Bytes::from_static(b"data: [DONE]\n\n")).await;
    });
    let stream = futures::stream::unfold(brx, |mut rx| async move {
        rx.recv()
            .await
            .map(|b| (Ok::<_, Infallible>(Frame::data(b)), rx))
    });
    let body: Body = StreamBody::new(stream).boxed_unsync();
    let mut out = Response::new(body);
    let h = out.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    h.insert("cache-control", HeaderValue::from_static("no-cache"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_hashed_listed_and_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let (k, secret) = create_key(dir.path(), "ci", Some(dir.path())).unwrap();
        assert!(secret.starts_with("kor_") && secret.len() == 52);
        let text = std::fs::read_to_string(keys_path(dir.path())).unwrap();
        assert!(!text.contains(&secret), "the key itself is never stored");
        assert_eq!(authenticate(dir.path(), &secret).unwrap().id, k.id);
        assert!(authenticate(dir.path(), "kor_wrong").is_none());
        assert!(authenticate(dir.path(), "").is_none());
        assert!(create_key(dir.path(), "", None).is_err());
        assert!(create_key(dir.path(), "x", Some(&dir.path().join("missing"))).is_err());
        assert!(revoke_key(dir.path(), &k.id).unwrap());
        assert!(!revoke_key(dir.path(), &k.id).unwrap());
        assert!(authenticate(dir.path(), &secret).is_none());
    }

    #[test]
    fn limiter_bursts_then_refills() {
        let l = Limiter::default();
        let t0 = Instant::now();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for _ in 0..BURST as usize {
            assert!(l.allow("a", t0));
        }
        assert!(!l.allow("a", t0));
        assert!(l.allow("b", t0), "buckets are per key");
        assert!(l.allow("a", t0 + Duration::from_secs(1)));
    }

    #[test]
    fn chat_messages_split_into_history_system_and_prompt() {
        let m = |role: &str, content: Value| ChatMsg {
            role: role.into(),
            content,
        };
        let (h, sys, p) = split_messages(&[
            m("system", json!("be brief")),
            m("user", json!("hi")),
            m("assistant", json!("hello")),
            m(
                "user",
                json!([{ "type": "text", "text": "what is lib.rs?" }]),
            ),
        ])
        .unwrap();
        assert_eq!(
            (h.len(), sys.as_str(), p.as_str()),
            (2, "be brief", "what is lib.rs?")
        );
        assert!(split_messages(&[]).is_err());
        assert!(split_messages(&[m("assistant", json!("x"))]).is_err());
    }
}
