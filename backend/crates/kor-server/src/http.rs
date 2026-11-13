//! The HTTPS API. One TLS listener (rustls, TLS 1.2+); client certificates from the team CA are
//! requested and, with `require_client_cert`, demanded on every member route. Routes:
//!
//! | Route | Role |
//! | --- | --- |
//! | `GET /healthz` | none |
//! | `GET /api/v1/auth/providers`, `POST /api/v1/auth/token` | none (SSO exchange) |
//! | `GET /api/v1/me`, `GET /api/v1/members` | viewer |
//! | `GET /api/v1/docs/<ns>[/<key>][?version=n]` | viewer |
//! | `PUT /api/v1/docs/<ns>/<key>` `{body, version?}` | admin: rules/templates/hooks/tools; member: memory/graph/reports |
//! | `PUT/GET /team_memory?repo=`, `PUT/GET /team_tools` | the existing `kor` clients' paths |
//! | `GET /api/v1/vault`, `GET /api/v1/vault/<name>` | member (each opening audited) |
//! | `PUT/DELETE /api/v1/vault/<name>` | admin |
//! | `POST /api/v1/shares` `{ns:"reports", key, hours}`; `GET /share/<token>` | member; link holder |
//! | `POST/GET /api/v1/queue`, `GET /api/v1/queue/<id>`, `POST /api/v1/queue/claim`, `POST /api/v1/queue/<id>/finish` | member (viewer reads) |
//! | `GET /api/v1/audit?after=&limit=` | admin |
//! | `GET /api/v1/relay/listen/<name>`, `GET /api/v1/relay/dial/<name>` (WebSocket) | member |

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{Method, Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::store::{Error, Result, Role, Store, valid_key, writer_of};
use crate::{Layout, ServerConfig, auth, pki, relay};

type Resp = Response<Full<Bytes>>;

const MAX_CONNECTIONS: usize = 512;
const MAX_DOC: usize = 4 * 1024 * 1024;
const MAX_PROMPT: usize = 64 * 1024;
const MAX_RESULT: usize = 4 * 1024 * 1024;
const MAX_SECRET: usize = 64 * 1024;
/// Requests per second per address, and the burst.
const RATE: f64 = 50.0;
const BURST: f64 = 200.0;

/// The running server's shared state.
pub struct App {
    /// Server configuration.
    pub cfg: ServerConfig,
    /// Storage.
    pub store: Store,
    /// Files of the server directory.
    pub layout: Layout,
    verifier: auth::Verifier,
    hub: relay::Hub,
    buckets: Mutex<HashMap<IpAddr, (f64, Instant)>>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("team", &self.cfg.team)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Load the server directory.
    ///
    /// # Errors
    /// No server there, or the database/vault key cannot be opened.
    pub async fn open(dir: &std::path::Path) -> std::result::Result<Arc<Self>, String> {
        let layout = Layout::new(dir);
        let cfg = ServerConfig::load(&layout.config())?;
        Self::with(cfg, layout).await
    }

    /// # Errors
    /// The database/vault key cannot be opened.
    pub async fn with(cfg: ServerConfig, layout: Layout) -> std::result::Result<Arc<Self>, String> {
        let store = Store::open(&cfg, &layout.dir).await?;
        Ok(Arc::new(Self {
            cfg,
            store,
            layout,
            verifier: auth::Verifier::new()?,
            hub: relay::Hub::default(),
            buckets: Mutex::new(HashMap::new()),
        }))
    }

    fn allow(&self, ip: IpAddr) -> bool {
        let mut b = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if b.len() > 50_000 {
            b.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(60));
        }
        let now = Instant::now();
        let (tokens, last) = b.entry(ip).or_insert((BURST, now));
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * RATE).min(BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// A started server.
#[derive(Debug)]
pub struct Running {
    /// Address the server listens on.
    pub addr: SocketAddr,
    stop: tokio::sync::watch::Sender<bool>,
    /// The server task.
    pub task: tokio::task::JoinHandle<()>,
}

impl Running {
    /// Ask the server to stop.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// rustls server config: the server certificate, client certificates from the team CA
/// requested (enforced per route, so SSO exchange and share links work without one).
///
/// # Errors
/// Missing or damaged certificate files.
pub fn tls_config(layout: &Layout) -> std::result::Result<rustls::ServerConfig, String> {
    let read =
        |p: std::path::PathBuf| std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()));
    let chain = pki::certs_from_pem(&read(layout.server_cert())?)?;
    let key = pki::key_from_pem(&read(layout.server_key())?)?;
    let mut roots = rustls::RootCertStore::empty();
    for c in pki::certs_from_pem(&read(layout.ca_cert())?)? {
        roots.add(c).map_err(|e| e.to_string())?;
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .allow_unauthenticated()
    .build()
    .map_err(|e| e.to_string())?;
    let mut c = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|e| e.to_string())?;
    c.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(c)
}

/// Bind `addr` and serve until [`Running::stop`].
///
/// # Errors
/// TLS files or the address.
pub async fn start(app: Arc<App>, addr: SocketAddr) -> std::result::Result<Running, String> {
    let tls = tokio_rustls::TlsAcceptor::from(Arc::new(tls_config(&app.layout)?));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("{addr}: {e}"))?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let limit = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.changed() => break,
                acc = listener.accept() => {
                    let Ok((tcp, peer)) = acc else { continue };
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                    let (app, tls) = (app.clone(), tls.clone());
                    tokio::spawn(async move {
                        let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_secs(10), tls.accept(tcp)).await else {
                            return;
                        };
                        let cert = stream
                            .get_ref()
                            .1
                            .peer_certificates()
                            .and_then(<[_]>::first)
                            .map(|c| pki::fingerprint(c));
                        let svc = hyper::service::service_fn(move |req| {
                            let (app, cert) = (app.clone(), cert.clone());
                            async move { Ok::<_, Infallible>(handle(&app, req, peer.ip(), cert.as_deref()).await) }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(Duration::from_secs(20))
                            .serve_connection(TokioIo::new(stream), svc)
                            .with_upgrades()
                            .await;
                        drop(permit);
                    });
                }
            }
        }
    });
    Ok(Running {
        addr: local,
        stop: tx,
        task,
    })
}

// -- responses -------------------------------------------------------------------------------

fn secure(mut r: Resp) -> Resp {
    let h = r.headers_mut();
    for (k, v) in [
        (
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        ),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
        ("referrer-policy", "no-referrer"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
        ("cache-control", "no-store"),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    r
}

fn reply(status: u16, ctype: &'static str, body: impl Into<Bytes>) -> Resp {
    let mut r = Response::new(Full::new(body.into()));
    *r.status_mut() =
        hyper::StatusCode::from_u16(status).unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
    r.headers_mut()
        .insert("content-type", HeaderValue::from_static(ctype));
    secure(r)
}

fn ok(v: &Value) -> Resp {
    reply(200, "application/json", v.to_string())
}

fn fail(e: &Error) -> Resp {
    // Internal errors are logged here and never sent with their details.
    let msg = if e.status >= 500 && e.status != 502 {
        eprintln!(
            "{}",
            json!({ "level": "error", "event": "server.error", "error": e.message })
        );
        "internal error".to_owned()
    } else {
        e.message.clone()
    };
    reply(
        e.status,
        "application/json",
        json!({ "error": msg }).to_string(),
    )
}

// -- request helpers -------------------------------------------------------------------------

async fn body_json(app: &App, req: Request<Incoming>) -> Result<Value> {
    let bytes = Limited::new(req.into_body(), app.cfg.max_body)
        .collect()
        .await
        .map_err(|_| {
            Error::new(
                413,
                format!("request body is larger than {} bytes", app.cfg.max_body),
            )
        })?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|_| Error::new(400, "invalid JSON"))
}

fn query(req: &Request<Incoming>, key: &str) -> Option<String> {
    req.uri().query()?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(x) => {
                        out.push(x);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn bearer(req: &Request<Incoming>) -> Option<String> {
    if let Some(t) = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(t.trim().to_owned());
    }
    // Browsers cannot set headers on a WebSocket: `new WebSocket(url, ["kor-relay", "bearer.<token>"])`.
    req.headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(',')
                .map(str::trim)
                .find_map(|p| p.strip_prefix("bearer."))
        })
        .map(str::to_owned)
}

/// Who is calling: client certificate and/or bearer token, both resolving to the same member.
async fn who(app: &App, req: &Request<Incoming>, cert: Option<&str>) -> Result<(String, Role)> {
    let by_cert = match cert {
        Some(fp) => app.store.cert_owner(fp).await?,
        None => None,
    };
    if app.cfg.require_client_cert && by_cert.is_none() {
        return Err(Error::new(401, "a team client certificate is required"));
    }
    let by_token = match bearer(req) {
        Some(t) => Some(
            app.store
                .token_owner(&t)
                .await?
                .ok_or_else(|| Error::new(401, "unknown or expired token"))?,
        ),
        None => None,
    };
    let email = match (by_cert, by_token) {
        (Some(c), Some(t)) if c != t => {
            return Err(Error::new(
                403,
                "the token and the client certificate belong to different members",
            ));
        }
        (_, Some(e)) | (Some(e), None) => e,
        (None, None) => return Err(Error::new(401, "sign in first")),
    };
    let role = app
        .store
        .role(&email)
        .await?
        .ok_or_else(|| Error::new(403, "not a member of this team"))?;
    Ok((email, role))
}

fn need(role: Role, min: Role) -> Result<()> {
    if role >= min {
        Ok(())
    } else {
        Err(Error::new(403, format!("needs the {} role", min.as_str())))
    }
}

fn str_field<'a>(v: &'a Value, k: &str, max: usize) -> Result<&'a str> {
    let s = v[k]
        .as_str()
        .ok_or_else(|| Error::new(400, format!("`{k}` is required")))?;
    if s.len() > max {
        return Err(Error::new(413, format!("`{k}` is longer than {max} bytes")));
    }
    Ok(s)
}

// -- routing ---------------------------------------------------------------------------------

async fn handle(app: &Arc<App>, req: Request<Incoming>, ip: IpAddr, cert: Option<&str>) -> Resp {
    if !app.allow(ip) {
        return fail(&Error::new(429, "too many requests"));
    }
    match route(app, req, cert).await {
        Ok(r) => r,
        Err(e) => fail(&e),
    }
}

async fn route(app: &Arc<App>, req: Request<Incoming>, cert: Option<&str>) -> Result<Resp> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (&method, segs.as_slice()) {
        (&Method::GET, ["healthz"]) => Ok(ok(&json!({ "ok": true, "team": app.cfg.team }))),
        (&Method::GET, ["api", "v1", "auth", "providers"]) => Ok(ok(&providers(app).await)),
        (&Method::POST, ["api", "v1", "auth", "token"]) => token(app, req, cert).await,
        (&Method::GET, ["share", t]) => share_open(app, t).await,
        (_, ["api", "v1", "relay", mode @ ("listen" | "dial"), name]) => {
            let (email, role) = who(app, &req, cert).await?;
            need(role, Role::Member)?;
            relay_upgrade(app, req, mode, name, &email)
        }
        _ => {
            let (email, role) = who(app, &req, cert).await?;
            member_route(app, req, &method, &segs, &email, role).await
        }
    }
}

#[allow(clippy::too_many_lines)] // one dispatch arm per route; splitting it would scatter the table
async fn member_route(
    app: &Arc<App>,
    req: Request<Incoming>,
    method: &Method,
    segs: &[&str],
    email: &str,
    role: Role,
) -> Result<Resp> {
    let st = &app.store;
    match (method, segs) {
        (&Method::GET, ["api", "v1", "me"]) => Ok(ok(
            &json!({ "email": email, "role": role, "team": app.cfg.team }),
        )),
        (&Method::GET, ["api", "v1", "members"]) => {
            let m: Vec<Value> = st
                .members()
                .await?
                .into_iter()
                .map(|(e, r)| json!({ "email": e, "role": r }))
                .collect();
            Ok(ok(&json!({ "members": m })))
        }
        // Shared documents.
        (&Method::GET, ["api", "v1", "docs", ns]) => {
            writer_of(ns).ok_or_else(|| Error::new(404, "unknown namespace"))?;
            Ok(ok(&json!({ "docs": st.docs(ns).await? })))
        }
        (&Method::GET, ["api", "v1", "docs", ns, key @ ..]) => {
            let key = key.join("/");
            if writer_of(ns).is_none() || !valid_key(&key) {
                return Err(Error::new(404, "unknown document"));
            }
            let version = query(&req, "version").and_then(|v| v.parse().ok());
            let d = st
                .doc(ns, &key, version)
                .await?
                .ok_or_else(|| Error::new(404, "no such document"))?;
            Ok(ok(&json!(d)))
        }
        (&Method::PUT, ["api", "v1", "docs", ns, key @ ..]) => {
            let key = key.join("/");
            let writer = writer_of(ns).ok_or_else(|| Error::new(404, "unknown namespace"))?;
            if !valid_key(&key) {
                return Err(Error::new(400, "bad document key"));
            }
            need(role, writer)?;
            let ns = (*ns).to_owned();
            let v = body_json(app, req).await?;
            let body = str_field(&v, "body", MAX_DOC)?;
            let version = st
                .put_doc(&ns, &key, body, v["version"].as_i64(), email)
                .await?;
            Ok(ok(&json!({ "version": version })))
        }
        // The paths `kor` already uses (team_sync, `kor tool share/pull`).
        (&Method::PUT, ["team_memory"]) => {
            need(role, Role::Member)?;
            let repo = query(&req, "repo").unwrap_or_default();
            if !valid_key(&repo) {
                return Err(Error::new(400, "bad repo"));
            }
            let if_match = req
                .headers()
                .get("if-match")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let v = body_json(app, req).await?;
            if !v["entries"].is_object() {
                return Err(Error::new(400, "`entries` must be an object"));
            }
            let body = v.to_string();
            if body.len() > MAX_DOC {
                return Err(Error::new(413, "memory too large"));
            }
            let cur = st.doc("memory", &repo, None).await?;
            if let Some(tag) = if_match {
                let have = cur
                    .as_ref()
                    .map(|d| crate::sha256_hex(&d.body))
                    .unwrap_or_default();
                if tag.trim_matches('"') != have {
                    return Err(Error::new(412, "ETag mismatch"));
                }
            }
            st.put_doc("memory", &repo, &body, None, email).await?;
            Ok(ok(&json!({ "checksum": crate::sha256_hex(&body) })))
        }
        (&Method::GET, ["team_memory"]) => {
            let repo = query(&req, "repo").unwrap_or_default();
            let d = st
                .doc("memory", &repo, None)
                .await?
                .ok_or_else(|| Error::new(404, "no team memory"))?;
            let v: Value = serde_json::from_str(&d.body).unwrap_or(Value::Null);
            Ok(ok(
                &json!({ "entries": v["entries"], "checksum": crate::sha256_hex(&d.body) }),
            ))
        }
        (&Method::PUT, ["team_tools"]) => {
            need(role, Role::Admin)?;
            let v = body_json(app, req).await?;
            if !v["tools"].is_array() {
                return Err(Error::new(400, "`tools` must be an array"));
            }
            st.put_doc("tools", "team", &v.to_string(), None, email)
                .await?;
            Ok(ok(&json!({ "ok": true })))
        }
        (&Method::GET, ["team_tools"]) => {
            let tools = match st.doc("tools", "team", None).await? {
                Some(d) => serde_json::from_str::<Value>(&d.body)
                    .map(|v| v["tools"].clone())
                    .unwrap_or(json!([])),
                None => json!([]),
            };
            Ok(ok(&json!({ "tools": tools })))
        }
        // Vault.
        (&Method::GET, ["api", "v1", "vault"]) => {
            need(role, Role::Member)?;
            Ok(ok(&json!({ "names": st.vault_names().await? })))
        }
        (&Method::GET, ["api", "v1", "vault", name]) => {
            need(role, Role::Member)?;
            let v = st
                .vault_get(name, email)
                .await?
                .ok_or_else(|| Error::new(404, "no such secret"))?;
            Ok(ok(&json!({ "name": name, "value": v })))
        }
        (&Method::PUT, ["api", "v1", "vault", name]) => {
            need(role, Role::Admin)?;
            if !valid_secret_name(name) {
                return Err(Error::new(400, "secret names: A-Z, 0-9 and _"));
            }
            let name = (*name).to_owned();
            let v = body_json(app, req).await?;
            st.vault_put(&name, str_field(&v, "value", MAX_SECRET)?, email)
                .await?;
            Ok(ok(&json!({ "ok": true })))
        }
        (&Method::DELETE, ["api", "v1", "vault", name]) => {
            need(role, Role::Admin)?;
            st.vault_delete(name, email).await?;
            Ok(ok(&json!({ "ok": true })))
        }
        // Share links for reports.
        (&Method::POST, ["api", "v1", "shares"]) => {
            need(role, Role::Member)?;
            let v = body_json(app, req).await?;
            if v["ns"].as_str().unwrap_or("reports") != "reports" {
                return Err(Error::new(
                    400,
                    "only reports can be shared outside the team",
                ));
            }
            let key = str_field(&v, "key", 200)?;
            let (token, expires) = st
                .share("reports", key, v["hours"].as_i64().unwrap_or(72), email)
                .await?;
            Ok(ok(
                &json!({ "path": format!("/share/{token}"), "expires": expires }),
            ))
        }
        // Task queue.
        (&Method::POST, ["api", "v1", "queue"]) => {
            need(role, Role::Member)?;
            let v = body_json(app, req).await?;
            let prompt = str_field(&v, "prompt", MAX_PROMPT)?;
            let repo = v["repo"].as_str().unwrap_or_default();
            let callback = v["callback"].as_str().unwrap_or_default();
            if repo.len() > 300 {
                return Err(Error::new(413, "`repo` is too long"));
            }
            if !callback.is_empty() {
                check_callback_url(callback, app.cfg.allow_private_callbacks)?;
            }
            let t = st.task_create(prompt, repo, callback, email).await?;
            Ok(reply(
                201,
                "application/json",
                json!({ "id": t.id, "state": t.state, "secret": t.secret }).to_string(),
            ))
        }
        (&Method::GET, ["api", "v1", "queue"]) => {
            let state = query(&req, "state");
            Ok(ok(
                &json!({ "tasks": st.tasks(state.as_deref(), 100).await? }),
            ))
        }
        (&Method::POST, ["api", "v1", "queue", "claim"]) => {
            need(role, Role::Member)?;
            Ok(match st.task_claim(email).await? {
                Some(t) => ok(&json!(t)),
                None => reply(204, "application/json", ""),
            })
        }
        (&Method::GET, ["api", "v1", "queue", id]) => {
            let t = st
                .task(id)
                .await?
                .ok_or_else(|| Error::new(404, "no such task"))?;
            Ok(ok(&json!(t)))
        }
        (&Method::POST, ["api", "v1", "queue", id, "finish"]) => {
            need(role, Role::Member)?;
            let id = (*id).to_owned();
            let v = body_json(app, req).await?;
            let result = v["result"].as_str().unwrap_or_default();
            if result.len() > MAX_RESULT {
                return Err(Error::new(413, "result too large"));
            }
            let t = st
                .task_finish(&id, email, v["ok"].as_bool().unwrap_or(false), result)
                .await?;
            if !t.callback.is_empty() {
                let app = app.clone();
                let t2 = t.clone();
                tokio::spawn(async move { deliver_callback(&app, &t2).await });
            }
            Ok(ok(&json!(t)))
        }
        // Audit.
        (&Method::GET, ["api", "v1", "audit"]) => {
            need(role, Role::Admin)?;
            let after = query(&req, "after")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let limit = query(&req, "limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(200);
            let chain = match st.audit_verify().await {
                Ok(n) => json!({ "ok": true, "entries": n }),
                Err(seq) => json!({ "ok": false, "brokenAt": seq }),
            };
            Ok(ok(
                &json!({ "entries": st.audit_list(after, limit).await?, "chain": chain }),
            ))
        }
        _ => Err(Error::new(404, "no such route")),
    }
}

fn valid_secret_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 128
        && n.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

async fn providers(app: &App) -> Value {
    let mut out = Vec::new();
    for p in &app.cfg.providers {
        let (device, token) =
            if p.kind == "oidc" && (p.device_endpoint.is_empty() || p.token_endpoint.is_empty()) {
                match app.verifier.discovery(p).await {
                    Ok(d) => (
                        d["device_authorization_endpoint"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        d["token_endpoint"].as_str().unwrap_or_default().to_owned(),
                    ),
                    Err(_) => (String::new(), String::new()),
                }
            } else if p.kind == "github" && p.device_endpoint.is_empty() {
                (
                    "https://github.com/login/device/code".to_owned(),
                    "https://github.com/login/oauth/access_token".to_owned(),
                )
            } else {
                (p.device_endpoint.clone(), p.token_endpoint.clone())
            };
        out.push(json!({
            "name": p.name, "kind": p.kind, "issuer": p.issuer, "clientId": p.client_id,
            "deviceEndpoint": device, "tokenEndpoint": token,
        }));
    }
    json!({ "providers": out, "requireClientCert": app.cfg.require_client_cert })
}

/// `POST /api/v1/auth/token {provider, id_token | access_token}` → a session token.
async fn token(app: &Arc<App>, req: Request<Incoming>, cert: Option<&str>) -> Result<Resp> {
    let by_cert = match cert {
        Some(fp) => app.store.cert_owner(fp).await?,
        None => None,
    };
    if app.cfg.require_client_cert && by_cert.is_none() {
        return Err(Error::new(401, "a team client certificate is required"));
    }
    let v = body_json(app, req).await?;
    let name = str_field(&v, "provider", 64)?;
    let p = app
        .cfg
        .provider(name)
        .ok_or_else(|| Error::new(404, "unknown identity provider"))?;
    let email = match p.kind.as_str() {
        "github" => {
            app.verifier
                .github_email(p, str_field(&v, "access_token", 512)?)
                .await
        }
        _ => {
            app.verifier
                .verify_id_token(p, str_field(&v, "id_token", 16 * 1024)?)
                .await
        }
    };
    let email = match email {
        Ok(e) => e,
        Err(e) => {
            app.store
                .audit(
                    "anonymous",
                    "auth.refused",
                    name,
                    &json!({ "reason": e.message }),
                )
                .await?;
            return Err(e);
        }
    };
    if by_cert.as_deref().is_some_and(|c| c != email) {
        return Err(Error::new(
            403,
            "the client certificate belongs to another member",
        ));
    }
    let role = app.store.role(&email).await?.ok_or_else(|| {
        Error::new(
            403,
            "not a member of this team (ask an admin: kor server add-member)",
        )
    })?;
    let (token, expires) = app
        .store
        .issue_session(&email, app.cfg.session_hours, name)
        .await?;
    Ok(ok(
        &json!({ "token": token, "expires": expires, "email": email, "role": role }),
    ))
}

async fn share_open(app: &App, token: &str) -> Result<Resp> {
    let d = app
        .store
        .shared(token)
        .await?
        .ok_or_else(|| Error::new(404, "this link has expired or does not exist"))?;
    // Plain text with `default-src 'none'`: a shared report can never run script in the reader's browser.
    let mut r = reply(200, "text/plain; charset=utf-8", d.body);
    r.headers_mut()
        .insert("content-disposition", HeaderValue::from_static("inline"));
    Ok(r)
}

fn relay_upgrade(
    app: &Arc<App>,
    mut req: Request<Incoming>,
    mode: &str,
    name: &str,
    email: &str,
) -> Result<Resp> {
    if !relay::valid_name(name) {
        return Err(Error::new(400, "bad relay name"));
    }
    let h = req.headers();
    let is_ws = h
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let key = h
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (true, Some(key)) = (is_ws, key) else {
        return Err(Error::new(400, "a WebSocket upgrade is required"));
    };
    let wants_proto = h
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|p| p.trim() == "kor-relay"));
    if mode == "listen" {
        app.hub.claim(name, email).map_err(|e| Error::new(409, e))?;
    } else if !app.hub.listening(name) {
        return Err(Error::new(404, "nobody is listening on that name"));
    }
    let on = hyper::upgrade::on(&mut req);
    let (app2, name2, mode2, email2) = (
        app.clone(),
        name.to_owned(),
        mode.to_owned(),
        email.to_owned(),
    );
    tokio::spawn(async move {
        let Ok(up) = on.await else { return };
        let ws = relay::accept(up).await;
        let _ = app2
            .store
            .audit(&email2, &format!("relay.{mode2}"), &name2, &json!({}))
            .await;
        if mode2 == "listen" {
            app2.hub.listen(name2, ws).await;
        } else {
            let _ = app2.hub.dial(&name2, ws).await;
        }
    });
    let mut r = Response::new(Full::new(Bytes::new()));
    *r.status_mut() = hyper::StatusCode::SWITCHING_PROTOCOLS;
    let hm = r.headers_mut();
    hm.insert("upgrade", HeaderValue::from_static("websocket"));
    hm.insert("connection", HeaderValue::from_static("Upgrade"));
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    hm.insert(
        "sec-websocket-accept",
        HeaderValue::from_str(&accept).map_err(|e| e.to_string())?,
    );
    if wants_proto {
        hm.insert(
            "sec-websocket-protocol",
            HeaderValue::from_static("kor-relay"),
        );
    }
    Ok(r)
}

// -- callbacks -------------------------------------------------------------------------------

/// `https://` (or `http://` with `allow_private_callbacks`), no credentials, ≤ 2048 bytes.
///
/// # Errors
/// 400 naming the problem.
pub fn check_callback_url(raw: &str, allow_private: bool) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(raw).map_err(|_| Error::new(400, "callback is not a URL"))?;
    let scheme_ok = u.scheme() == "https" || (allow_private && u.scheme() == "http");
    if raw.len() > 2048
        || !scheme_ok
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(Error::new(
            400,
            "callback must be an https:// URL without credentials",
        ));
    }
    Ok(u)
}

/// Loopback, private, link-local, CGNAT, multicast, unspecified, documentation: not the public
/// internet.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || o[0] == 0
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || v.to_ipv4_mapped()
                    .is_some_and(|m| is_private(IpAddr::V4(m)))
        }
    }
}

/// HMAC-SHA256 of the body with the task's secret, hex.
pub fn sign(secret: &str, body: &[u8]) -> String {
    let k = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, secret.as_bytes());
    crate::hex(aws_lc_rs::hmac::sign(&k, body).as_ref())
}

/// POST the finished task to its callback, signed (`x-kor-signature: sha256=<hmac>`), up to three
/// tries. The host is resolved once and the request pinned to that address, so a DNS answer cannot
/// switch to an internal address between the check and the request.
async fn deliver_callback(app: &App, t: &crate::store::Task) {
    let outcome = send_callback(app, t).await;
    let detail = match &outcome {
        Ok(status) => json!({ "status": status }),
        Err(e) => json!({ "error": e }),
    };
    let _ = app
        .store
        .audit("server", "task.callback", &t.id, &detail)
        .await;
}

async fn send_callback(app: &App, t: &crate::store::Task) -> std::result::Result<u16, String> {
    let u =
        check_callback_url(&t.callback, app.cfg.allow_private_callbacks).map_err(|e| e.message)?;
    let host = u
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| e.to_string())?
        .collect();
    let addr = addrs
        .into_iter()
        .find(|a| app.cfg.allow_private_callbacks || !is_private(a.ip()))
        .ok_or("callback host resolves only to private addresses")?;
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&host, addr)
        .build()
        .map_err(|e| e.to_string())?;
    let body = json!({ "id": t.id, "state": t.state, "repo": t.repo, "result": t.result, "finishedAt": t.updated_ms }).to_string();
    let sig = format!("sha256={}", sign(&t.secret, body.as_bytes()));
    let mut last = String::new();
    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
        }
        match client
            .post(u.clone())
            .header("content-type", "application/json")
            .header("x-kor-task", &t.id)
            .header("x-kor-signature", &sig)
            .body(body.clone())
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => return Ok(r.status().as_u16()),
            Ok(r) => last = format!("answered {}", r.status()),
            Err(e) => last = kor_core::net::describe(&e),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_and_callback_urls() {
        for p in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "::ffff:10.0.0.1",
            "0.0.0.0",
        ] {
            assert!(is_private(p.parse().unwrap()), "{p}");
        }
        assert!(!is_private("1.1.1.1".parse().unwrap()));
        assert!(check_callback_url("https://ci.example.com/hook", false).is_ok());
        assert!(check_callback_url("http://ci.example.com/hook", false).is_err());
        assert!(check_callback_url("https://u:p@ci.example.com/", false).is_err());
        assert!(check_callback_url("http://127.0.0.1:9/", true).is_ok());
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(percent_decode("acme%2Frepo"), "acme/repo");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
