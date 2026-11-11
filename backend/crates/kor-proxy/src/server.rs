//! HTTP layer (port of CLIProxyAPI `internal/api`): routes, client and management auth, body
//! limits, and the read-only `/management.html` page. hyper is used directly; it is already in
//! the dependency tree through reqwest.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use kor_secrets::ExposeSecret;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;

use crate::translate::{self, Collected, Format};
use crate::{Fail, Gateway};

type Body = BoxBody<Bytes, Infallible>;
type Resp = Response<Body>;

const MAX_CONNECTIONS: usize = 256;

/// A server started by [`start`]. Dropping it does not stop the server; call [`Running::stop`].
#[derive(Debug)]
pub struct Running {
    pub addr: SocketAddr,
    pub gateway: Arc<Gateway>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

fn is_loopback_host(h: &str) -> bool {
    let h = h.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.to_ascii_lowercase().ends_with(".localhost")
        || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Refuses binds that would expose the gateway without a key: a non-loopback address needs an
/// access key and never honours `--no-auth`.
pub fn check_bind(addr: &SocketAddr, gw: &Gateway) -> Result<(), String> {
    if addr.ip().is_loopback() {
        return Ok(());
    }
    if gw.access.no_auth {
        return Err("--no-auth is only allowed on a loopback address".into());
    }
    if gw.access.key.is_none() && gw.settings().client_keys.is_empty() {
        return Err("a non-loopback bind needs an access key".into());
    }
    Ok(())
}

/// Bind `addr` and serve until [`Running::stop`].
pub async fn start(gw: Arc<Gateway>, addr: SocketAddr) -> Result<Running, String> {
    check_bind(&addr, &gw)?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("{addr}: {e}"))?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let limit = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let g = gw.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.changed() => break,
                acc = listener.accept() => {
                    let Ok((stream, peer)) = acc else { continue };
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                    let g = g.clone();
                    tokio::spawn(async move {
                        let svc = hyper::service::service_fn(move |req| {
                            let g = g.clone();
                            async move { Ok::<_, Infallible>(handle(&g, req, peer, local).await) }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(Duration::from_secs(30))
                            .serve_connection(TokioIo::new(stream), svc)
                            .await;
                        drop(permit);
                    });
                }
            }
        }
    });
    Ok(Running {
        addr: local,
        gateway: gw,
        stop: tx,
        task,
    })
}

// ---------------------------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------------------------

fn with_headers(mut r: Resp) -> Resp {
    let h = r.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    r
}

fn full(status: u16, ctype: &'static str, body: impl Into<Bytes>) -> Resp {
    let mut r = Response::new(Full::new(body.into()).boxed());
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    r.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(ctype));
    with_headers(r)
}

fn json_resp(status: u16, v: &Value) -> Resp {
    full(status, "application/json", v.to_string())
}

fn fail(format: Format, f: &Fail) -> Resp {
    let mut r = json_resp(
        f.status,
        &translate::error_body(format, f.status, &f.message),
    );
    if let Some(s) = f.retry_after
        && let Ok(v) = HeaderValue::from_str(&s.to_string())
    {
        r.headers_mut().insert("retry-after", v);
    }
    r
}

fn streamed(
    ctype: &'static str,
    frames: impl futures::Stream<Item = String> + Send + Sync + 'static,
) -> Resp {
    let body = StreamBody::new(frames.map(|s| Ok::<_, Infallible>(Frame::data(Bytes::from(s)))));
    let mut r = Response::new(BodyExt::boxed(body));
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert("cache-control", HeaderValue::from_static("no-cache"));
    with_headers(r)
}

// ---------------------------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------------------------

fn header<'a>(req: &'a Request<Incoming>, name: &str) -> Option<&'a str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

fn query_param(req: &Request<Incoming>, name: &str) -> Option<String> {
    req.uri().query()?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| percent_decode(v))
    })
}

/// The key a client presented: `Authorization: Bearer`, `x-api-key` (Anthropic),
/// `x-goog-api-key` or `?key=` (Gemini).
fn presented(req: &Request<Incoming>) -> Option<String> {
    header(req, "authorization")
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(|s| s.trim().to_owned())
        .or_else(|| header(req, "x-api-key").map(str::to_owned))
        .or_else(|| header(req, "x-goog-api-key").map(str::to_owned))
        .or_else(|| query_param(req, "key"))
}

fn eq(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

fn client_ok(gw: &Gateway, req: &Request<Incoming>, local_bind: bool) -> bool {
    if gw.access.no_auth && local_bind {
        return true;
    }
    let Some(k) = presented(req) else {
        return false;
    };
    let st = gw.settings();
    // Every candidate is compared, so timing does not reveal which one matched.
    let mut ok = false;
    if let Some(own) = &gw.access.key {
        ok |= eq(&k, own.expose_secret());
    }
    for c in &st.client_keys {
        ok |= eq(&k, c);
    }
    ok
}

/// `Authorization: Bearer` or `X-Management-Key`, against `management.secret-key` or Kor's key.
fn management_ok(
    gw: &Gateway,
    req: &Request<Incoming>,
    peer: SocketAddr,
    local_bind: bool,
) -> Result<(), u16> {
    let st = gw.settings();
    if !peer.ip().is_loopback() && !st.allow_remote_management {
        return Err(403);
    }
    if gw.access.no_auth && local_bind {
        return Ok(());
    }
    let Some(k) = header(req, "x-management-key")
        .map(str::to_owned)
        .or_else(|| presented(req))
    else {
        return Err(401);
    };
    let mut ok = false;
    if let Some(m) = &st.management_key {
        ok |= eq(&k, m);
    }
    if let Some(own) = &gw.access.key {
        ok |= eq(&k, own.expose_secret());
    }
    if ok { Ok(()) } else { Err(401) }
}

/// DNS-rebinding and cross-site guard for loopback binds: the `Host` must be a loopback name and
/// a browser `Origin`, when sent, must be a loopback origin too.
fn same_machine(req: &Request<Incoming>) -> bool {
    let host_ok = header(req, "host").is_none_or(|h| {
        let name = if h.starts_with('[') {
            h.split_once(']').map_or(h, |(a, _)| a)
        } else {
            h.rsplit_once(':').map_or(h, |(a, _)| a)
        };
        is_loopback_host(name)
    });
    let origin_ok = header(req, "origin").is_none_or(|o| {
        o.split_once("://")
            .map(|(_, rest)| rest)
            .map(|rest| {
                if rest.starts_with('[') {
                    rest.split_once(']').map_or(rest, |(a, _)| a)
                } else {
                    rest.rsplit_once(':').map_or(rest, |(a, _)| a)
                }
            })
            .is_some_and(is_loopback_host)
    });
    host_ok && origin_ok
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_lines)] // the route table; one match arm per endpoint
async fn handle(
    gw: &Arc<Gateway>,
    req: Request<Incoming>,
    peer: SocketAddr,
    local: SocketAddr,
) -> Resp {
    let local_bind = local.ip().is_loopback();
    if local_bind && !same_machine(&req) {
        return json_resp(
            403,
            &json!({"error": "requests must come from this machine"}),
        );
    }
    let path = req.uri().path().to_owned();
    let method = req.method().clone();
    match (&method, path.as_str()) {
        (&Method::GET, "/healthz") => return json_resp(200, &json!({"status": "ok"})),
        (&Method::GET, "/management.html") => {
            let mut r = full(200, "text/html; charset=utf-8", gw.page.clone());
            r.headers_mut().insert(
                "content-security-policy",
                HeaderValue::from_static(
                    "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'",
                ),
            );
            return r;
        }
        (&Method::GET, "/management.js") => {
            return full(200, "text/javascript; charset=utf-8", PAGE_JS);
        }
        _ => {}
    }
    if let Some(rest) = path.strip_prefix("/v0/management/") {
        if let Err(code) = management_ok(gw, &req, peer, local_bind) {
            return json_resp(
                code,
                &json!({"error": if code == 403 { "remote management is disabled" } else { "invalid management key" }}),
            );
        }
        return management(gw, &method, rest, req).await;
    }
    let format = if path.starts_with("/v1beta") {
        Format::Gemini
    } else if path.starts_with("/v1/messages") {
        Format::Claude
    } else {
        Format::OpenAi
    };
    if !client_ok(gw, &req, local_bind) {
        return fail(format, &Fail::new(401, "missing or invalid API key"));
    }
    match (&method, path.as_str()) {
        (&Method::GET, "/" | "/v1") => json_resp(
            200,
            &json!({
                "message": "Kor proxy (CLIProxyAPI-compatible)",
                "endpoints": ["POST /v1/chat/completions", "POST /v1/responses", "POST /v1/messages",
                              "POST /v1/messages/count_tokens", "GET /v1/models",
                              "GET /v1beta/models", "POST /v1beta/models/{model}:generateContent",
                              "POST /v1beta/models/{model}:streamGenerateContent"],
            }),
        ),
        (&Method::GET, "/v1/models") => json_resp(200, &openai_models(&gw.models().await)),
        (&Method::GET, "/v1beta/models") => json_resp(200, &gemini_models(&gw.models().await)),
        (&Method::POST, "/v1/chat/completions" | "/chat/completions") => {
            chat(gw, req, Format::OpenAi, None, false).await
        }
        (&Method::POST, "/v1/responses") => chat(gw, req, Format::Responses, None, false).await,
        (&Method::POST, "/v1/messages") => chat(gw, req, Format::Claude, None, false).await,
        (&Method::POST, "/v1/messages/count_tokens") => count_tokens(gw, req).await,
        (&Method::GET, p) if p.starts_with("/v1/models/") => {
            let id = percent_decode(&p["/v1/models/".len()..]);
            let models = gw.models().await;
            match models.iter().find(|(m, _)| *m == id) {
                Some((m, owner)) => json_resp(200, &model_json(m, owner)),
                None => fail(
                    Format::OpenAi,
                    &Fail::new(404, format!("model `{id}` not found")),
                ),
            }
        }
        (&Method::POST, p) if p.starts_with("/v1beta/models/") => {
            let rest = percent_decode(&p["/v1beta/models/".len()..]);
            let Some((model, action)) = rest.rsplit_once(':') else {
                return fail(
                    Format::Gemini,
                    &Fail::new(404, "expected /v1beta/models/{model}:{action}"),
                );
            };
            let sse = query_param(&req, "alt").as_deref() == Some("sse");
            match action {
                "generateContent" => {
                    chat(gw, req, Format::Gemini, Some(model.to_owned()), false).await
                }
                "streamGenerateContent" => {
                    chat(gw, req, Format::Gemini, Some(model.to_owned()), !sse).await
                }
                "countTokens" => count_tokens(gw, req).await,
                other => fail(
                    Format::Gemini,
                    &Fail::new(404, format!("action `{other}` is not supported")),
                ),
            }
        }
        _ => fail(
            format,
            &Fail::new(404, format!("{method} {path} is not supported")),
        ),
    }
}

async fn body_json(gw: &Gateway, req: Request<Incoming>) -> Result<Value, Fail> {
    let max = gw.settings().max_body;
    let bytes = Limited::new(req.into_body(), max)
        .collect()
        .await
        .map_err(|_| Fail::new(413, format!("request body is larger than {max} bytes")))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|e| Fail::new(400, format!("invalid JSON: {e}")))
}

/// One chat request in any client format. `gemini_array` selects Gemini's JSON-array streaming.
async fn chat(
    gw: &Arc<Gateway>,
    req: Request<Incoming>,
    format: Format,
    model: Option<String>,
    gemini_array: bool,
) -> Resp {
    let streaming_path = req.uri().path().ends_with(":streamGenerateContent");
    let body = match body_json(gw, req).await {
        Ok(b) => b,
        Err(f) => return fail(format, &f),
    };
    let mut inbound = match translate::parse(format, &body, model.as_deref()) {
        Ok(i) => i,
        Err(e) => return fail(format, &Fail::new(400, e)),
    };
    if format == Format::Gemini {
        inbound.stream = streaming_path;
    }
    let events = match gw.complete(inbound.req, &inbound.model).await {
        Ok(s) => s,
        Err(f) => return fail(format, &f),
    };
    if inbound.stream {
        let enc = translate::encoder(format, &inbound.model, !gemini_array);
        let ctype = if gemini_array {
            "application/json"
        } else {
            "text/event-stream"
        };
        let frames = futures::stream::unfold(Some((events, enc)), |st| async move {
            let (mut events, mut enc) = st?;
            Some(match events.next().await {
                Some(Ok(ev)) => (enc.event(&ev), Some((events, enc))),
                Some(Err(e)) => (enc.error(&e), None),
                None => (enc.finish(), None),
            })
        })
        .flat_map(futures::stream::iter);
        return streamed(ctype, SyncStream::new(frames));
    }
    let mut c = Collected::default();
    let mut events = events;
    while let Some(ev) = events.next().await {
        match ev {
            Ok(ev) => c.push(&ev),
            Err(e) => return fail(format, &Fail::from(&e)),
        }
    }
    json_resp(200, &translate::complete(format, &inbound.model, &c))
}

/// Rough count (4 characters per token) for `count_tokens` / `countTokens`; Kor's request form
/// does not carry the provider-specific counting call through the pool.
async fn count_tokens(gw: &Gateway, req: Request<Incoming>) -> Resp {
    let body = match body_json(gw, req).await {
        Ok(b) => b,
        Err(f) => return fail(Format::OpenAi, &f),
    };
    let n = (body.to_string().chars().count() / 4).max(1);
    json_resp(200, &json!({"input_tokens": n, "totalTokens": n}))
}

fn model_json(id: &str, owner: &str) -> Value {
    json!({"id": id, "object": "model", "type": "model", "created": 0, "owned_by": owner,
           "display_name": id, "created_at": "1970-01-01T00:00:00Z"})
}

/// One list answers both OpenAI (`object`, `data`) and Anthropic (`type`, `has_more`) clients.
fn openai_models(models: &[(String, String)]) -> Value {
    let data: Vec<Value> = models.iter().map(|(m, o)| model_json(m, o)).collect();
    json!({"object": "list", "data": data, "has_more": false,
           "first_id": models.first().map(|m| &m.0), "last_id": models.last().map(|m| &m.0)})
}

fn gemini_models(models: &[(String, String)]) -> Value {
    let data: Vec<Value> = models
        .iter()
        .map(|(m, _)| json!({"name": format!("models/{m}"), "displayName": m,
                             "supportedGenerationMethods": ["generateContent", "streamGenerateContent", "countTokens"]}))
        .collect();
    json!({"models": data})
}

async fn management(
    gw: &Arc<Gateway>,
    method: &Method,
    rest: &str,
    req: Request<Incoming>,
) -> Resp {
    let st = gw.settings();
    match (method, rest) {
        (&Method::GET, "usage") => json_resp(200, &gw.usage_json()),
        (&Method::GET, "api-key-usage") => {
            // CLIProxyAPI's shape, but keyed by masked keys: raw keys never leave the process.
            let mut out = serde_json::Map::new();
            for c in gw.credentials() {
                let entry = out
                    .entry(c.provider.to_owned())
                    .or_insert_with(|| json!({}));
                entry[format!("{}|{}", c.base_url, c.key)] =
                    json!({"success": c.success, "failed": c.failed, "recent_requests": []});
            }
            json_resp(200, &Value::Object(out))
        }
        (&Method::GET, "config") => json_resp(200, &gw.config_json()),
        (&Method::GET, "config.yaml") => match gw.config_path().map(std::fs::read_to_string) {
            Some(Ok(text)) => full(200, "application/yaml; charset=utf-8", text),
            _ => json_resp(404, &json!({"error": "no config file"})),
        },
        (&Method::PUT, "config.yaml") => {
            let Some(path) = gw.config_path().map(std::path::Path::to_path_buf) else {
                return json_resp(404, &json!({"error": "no config file"}));
            };
            let max = st.max_body;
            let Ok(bytes) = Limited::new(req.into_body(), max)
                .collect()
                .await
                .map(http_body_util::Collected::to_bytes)
            else {
                return json_resp(413, &json!({"error": "body too large"}));
            };
            let Ok(text) = String::from_utf8(bytes.to_vec()) else {
                return json_resp(400, &json!({"error": "config must be UTF-8"}));
            };
            match crate::config::save(&path, &text) {
                Ok(settings) => {
                    let warnings = gw.reload(settings);
                    json_resp(200, &json!({"status": "ok", "warnings": warnings}))
                }
                Err(e) => json_resp(422, &json!({"error": e})),
            }
        }
        (&Method::GET, "auth-files") => {
            let files: Vec<Value> = gw
                .credentials()
                .into_iter()
                .map(|c| json!({
                    "id": c.id, "name": c.id, "provider": c.provider, "type": "api_key",
                    "label": c.key, "status": if c.cooling_for_secs > 0 { "cooling" } else { "active" },
                    "disabled": false, "unavailable": c.cooling_for_secs > 0,
                    "next_retry_after_secs": c.cooling_for_secs, "weight": c.weight, "priority": c.priority,
                }))
                .collect();
            json_resp(200, &json!({"files": files}))
        }
        (&Method::GET, "api-keys") => json_resp(
            200,
            &json!({"api-keys": st.client_keys.iter().map(|k| kor_secrets::mask(k)).collect::<Vec<_>>()}),
        ),
        (&Method::GET, "request-retry") => {
            json_resp(200, &json!({"request-retry": st.request_retry}))
        }
        (&Method::GET, "max-retry-interval") => json_resp(
            200,
            &json!({"max-retry-interval": st.max_retry_interval.as_secs()}),
        ),
        (&Method::GET, "force-model-prefix") => {
            json_resp(200, &json!({"force-model-prefix": st.force_model_prefix}))
        }
        (&Method::GET, "usage-statistics-enabled") => json_resp(
            200,
            &json!({"usage-statistics-enabled": st.usage_statistics}),
        ),
        _ => json_resp(404, &json!({"error": "not found"})),
    }
}

/// hyper needs a `Sync` body stream; the frames stream is only ever polled by one task.
struct SyncStream<S>(std::sync::Mutex<std::pin::Pin<Box<S>>>);

impl<S> SyncStream<S> {
    fn new(s: S) -> Self {
        Self(std::sync::Mutex::new(Box::pin(s)))
    }
}

impl<S: futures::Stream + Send> futures::Stream for SyncStream<S> {
    type Item = S::Item;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<S::Item>> {
        let this = self.get_mut();
        let inner = this
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.as_mut().poll_next(cx)
    }
}

// ---------------------------------------------------------------------------------------------
// Management page
// ---------------------------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `/management.html`: asks for the key, then shows `/v0/management/usage`. Static markup with
/// localized labels; the data is rendered with `textContent` only.
pub fn page(l: &kor_i18n::Localizer) -> String {
    let t = |k: &str| esc(&l.t(k));
    let cols = [
        "proxy-col-credential",
        "proxy-col-model",
        "proxy-col-requests",
        "proxy-col-failed",
        "proxy-col-limited",
        "proxy-col-input",
        "proxy-col-output",
    ];
    let head = cols.iter().fold(String::new(), |mut s, c| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("<th>{}</th>", t(c)));
        s
    });
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
<style>body{{font:15px/1.5 system-ui,sans-serif;margin:16px;color:#1b1b1b;background:#fafafa}}\
table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #ddd;padding:6px;text-align:left}}\
input{{padding:6px;width:min(420px,100%)}}button{{padding:6px 12px}}#err{{color:#b00020}}\
@media (prefers-color-scheme:dark){{body{{background:#161616;color:#eee}}td,th{{border-color:#333}}}}</style></head>\
<body><main><h1>{title}</h1><p><label for=\"key\">{key}</label><br><input id=\"key\" type=\"password\" autocomplete=\"off\"> \
<button id=\"load\">{load}</button></p><p id=\"err\" role=\"alert\" data-msg=\"{err}\"></p><p id=\"sum\"></p>\
<table><thead><tr>{head}</tr></thead><tbody id=\"rows\"></tbody></table></main>\
<script src=\"/management.js\"></script></body></html>",
        lang = l.lang().code(),
        title = t("proxy-page-title"),
        key = t("proxy-page-key"),
        load = t("proxy-page-load"),
        err = t("proxy-page-error"),
    )
}

const PAGE_JS: &str = r"(function(){
var $=function(i){return document.getElementById(i)};
function load(){
  var h={};var k=$('key').value.trim();if(k){h['Authorization']='Bearer '+k}
  fetch('/v0/management/usage',{headers:h,cache:'no-store'}).then(function(r){
    if(!r.ok){throw new Error(String(r.status))}return r.json()}).then(function(d){
    $('err').textContent='';var u=d.usage||{};
    $('sum').textContent=(u.total_requests||0)+' / '+(u.total_tokens||0);
    var tb=$('rows');while(tb.firstChild){tb.removeChild(tb.firstChild)}
    var apis=u.apis||{};Object.keys(apis).forEach(function(c){var ms=apis[c].models||{};
      Object.keys(ms).forEach(function(m){var x=ms[m];var tr=document.createElement('tr');
        [c,m,x.total_requests,x.failure_count,x.rate_limited,x.input_tokens,x.output_tokens].forEach(function(v){
          var td=document.createElement('td');td.textContent=String(v);tr.appendChild(td)});tb.appendChild(tr)})});
  }).catch(function(e){$('err').textContent=$('err').getAttribute('data-msg')+' ('+e.message+')'});
}
$('load').addEventListener('click',load);load();
})();";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_names_and_percent_decoding() {
        for h in ["localhost", "127.0.0.1", "[::1]", "::1", "app.localhost"] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in ["evil.com", "192.168.1.2", "localhost.evil.com"] {
            assert!(!is_loopback_host(h), "{h}");
        }
        assert_eq!(percent_decode("openai%2Fgpt-4o"), "openai/gpt-4o");
        assert_eq!(percent_decode("a%zzb%2"), "a%zzb%2");
    }

    #[test]
    fn page_escapes_labels() {
        let p = page(&kor_i18n::Localizer::new(kor_i18n::Lang::En));
        assert!(p.contains("/management.js") && !p.contains("<script>"));
    }
}
