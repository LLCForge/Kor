//! `kor mount`: the `~/Kor/` tree as a WebDAV drive on `127.0.0.1` (§27.5, ADR 0052).
//!
//! The operating system's own WebDAV client mounts it (`kor_platform::mount`), so no kernel
//! driver is needed. Read-mostly: everything can be listed and read; only `Gelen/` and
//! `Görevler/` accept new files and folders (the drop boxes). Nothing can be deleted or moved.
//! Requests need the random path token and a loopback `Host` header (DNS rebinding), and the
//! listener binds to `127.0.0.1` only.

use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::watch;

use super::{INBOX, TASKS};

/// Largest file accepted by `PUT`.
const MAX_PUT: usize = 512 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 32;

type Resp = Response<Full<Bytes>>;

/// A served tree.
#[derive(Debug, Clone)]
pub struct Tree {
    pub root: PathBuf,
    /// First path segment every request must carry.
    pub token: String,
}

/// Why a URL path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    NotFound,
    Forbidden,
}

fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Tree {
    /// URL prefix (`/<token>/`).
    pub fn prefix(&self) -> String {
        format!("/{}/", self.token)
    }

    /// File system path and relative segments for a request path.
    ///
    /// # Errors
    /// Wrong token, `..`, a drive or root component.
    pub fn resolve(&self, url_path: &str) -> Result<(PathBuf, Vec<String>), Refused> {
        let rest = url_path
            .strip_prefix(&format!("/{}", self.token))
            .filter(|r| r.is_empty() || r.starts_with('/'))
            .ok_or(Refused::NotFound)?;
        let decoded = decode(rest).ok_or(Refused::Forbidden)?;
        let mut segs = Vec::new();
        for s in decoded.split('/').filter(|s| !s.is_empty()) {
            let ok = matches!(Path::new(s).components().next(), Some(Component::Normal(_)))
                && Path::new(s).components().count() == 1
                && !s.contains(['\\', ':', '\0']);
            if !ok {
                return Err(Refused::Forbidden);
            }
            segs.push(s.to_owned());
        }
        let mut p = self.root.clone();
        for s in &segs {
            p.push(s);
        }
        Ok((p, segs))
    }

    /// Writes are allowed only inside the drop boxes.
    pub fn writable(segs: &[String]) -> bool {
        segs.len() >= 2 && (segs[0] == INBOX || segs[0] == TASKS)
    }

    /// `PROPFIND` body for `path` (depth 0: itself; 1: and its children).
    pub fn propfind(&self, path: &Path, segs: &[String], depth: u8) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let href = format!(
            "{}{}",
            self.prefix().trim_end_matches('/'),
            encode(&format!("/{}", segs.join("/")))
        );
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n",
        );
        out.push_str(&entry(
            &href,
            &meta,
            segs.last().map_or("Kor", String::as_str),
        ));
        if depth > 0 && meta.is_dir() {
            let mut kids: Vec<_> = std::fs::read_dir(path).ok()?.flatten().collect();
            kids.sort_by_key(std::fs::DirEntry::file_name);
            for k in kids {
                let name = k.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(m) = std::fs::metadata(k.path()) else {
                    continue;
                };
                let child = format!("{}/{}", href.trim_end_matches('/'), encode(&name));
                out.push_str(&entry(&child, &m, &name));
            }
        }
        out.push_str("</D:multistatus>\n");
        Some(out)
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn http_date(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

fn entry(href: &str, m: &std::fs::Metadata, name: &str) -> String {
    let dir = m.is_dir();
    let href = if dir && !href.ends_with('/') {
        format!("{href}/")
    } else {
        href.to_owned()
    };
    let modified = m.modified().map(http_date).unwrap_or_default();
    let kind = if dir {
        "<D:resourcetype><D:collection/></D:resourcetype>"
    } else {
        "<D:resourcetype/>"
    };
    let len = if dir {
        String::new()
    } else {
        format!("<D:getcontentlength>{}</D:getcontentlength>", m.len())
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname>{kind}{len}\
         <D:getlastmodified>{modified}</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status>\
         </D:propstat></D:response>\n",
        xml(&href),
        xml(name)
    )
}

fn status(code: StatusCode) -> Resp {
    let mut r = Response::new(Full::new(Bytes::new()));
    *r.status_mut() = code;
    r
}

fn content_type(p: &Path) -> &'static str {
    match p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("md") => "text/markdown; charset=utf-8",
        Some("diff" | "txt" | "jsonl" | "cast") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        Some("html") => "text/html; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// Handle one request against `tree`; `host_ok` already checked the `Host` header.
pub async fn handle(tree: &Tree, req: Request<Incoming>) -> Resp {
    let method = req.method().clone();
    let depth = match req.headers().get("depth").and_then(|v| v.to_str().ok()) {
        Some("0") => 0,
        _ => 1,
    };
    let (path, segs) = match tree.resolve(req.uri().path()) {
        Ok(x) => x,
        Err(Refused::NotFound) => return status(StatusCode::NOT_FOUND),
        Err(Refused::Forbidden) => return status(StatusCode::FORBIDDEN),
    };
    match method.as_str() {
        "OPTIONS" => {
            let mut r = status(StatusCode::OK);
            let h = r.headers_mut();
            h.insert("DAV", "1, 2".parse().expect("static header"));
            h.insert(
                "Allow",
                "OPTIONS, GET, HEAD, PROPFIND, PUT, MKCOL, LOCK, UNLOCK, PROPPATCH"
                    .parse()
                    .expect("static header"),
            );
            h.insert("MS-Author-Via", "DAV".parse().expect("static header"));
            r
        }
        "PROPFIND" => match tree.propfind(&path, &segs, depth) {
            Some(body) => {
                let mut r = Response::new(Full::new(Bytes::from(body)));
                *r.status_mut() = StatusCode::MULTI_STATUS;
                r.headers_mut().insert(
                    "Content-Type",
                    "application/xml; charset=utf-8"
                        .parse()
                        .expect("static header"),
                );
                r
            }
            None => status(StatusCode::NOT_FOUND),
        },
        "GET" | "HEAD" => {
            if path.is_dir() {
                return status(StatusCode::METHOD_NOT_ALLOWED);
            }
            match tokio::fs::read(&path).await {
                Ok(b) => {
                    let len = b.len();
                    let body = if method == Method::HEAD {
                        Bytes::new()
                    } else {
                        Bytes::from(b)
                    };
                    let mut r = Response::new(Full::new(body));
                    let h = r.headers_mut();
                    h.insert(
                        "Content-Type",
                        content_type(&path).parse().expect("static header"),
                    );
                    h.insert("Content-Length", len.into());
                    r
                }
                Err(_) => status(StatusCode::NOT_FOUND),
            }
        }
        "PUT" => {
            if !Tree::writable(&segs) || path.is_dir() {
                return status(StatusCode::FORBIDDEN);
            }
            // Never replaces an existing file the client did not just create (no overwrites).
            let existed = path.exists();
            if existed && std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
                return status(StatusCode::FORBIDDEN);
            }
            match Limited::new(req.into_body(), MAX_PUT).collect().await {
                Ok(c) => match tokio::fs::write(&path, c.to_bytes()).await {
                    Ok(()) => status(if existed {
                        StatusCode::NO_CONTENT
                    } else {
                        StatusCode::CREATED
                    }),
                    Err(_) => status(StatusCode::CONFLICT),
                },
                Err(_) => status(StatusCode::PAYLOAD_TOO_LARGE),
            }
        }
        "MKCOL" => {
            if !Tree::writable(&segs) {
                return status(StatusCode::FORBIDDEN);
            }
            match std::fs::create_dir(&path) {
                Ok(()) => status(StatusCode::CREATED),
                Err(_) => status(StatusCode::METHOD_NOT_ALLOWED),
            }
        }
        // Windows and macOS lock before writing; a lock is only a token here (one user, one host).
        "LOCK" => {
            let token = format!(
                "opaquelocktoken:kor-{}",
                crate::korfolder::hash(req.uri().path().as_bytes())
            );
            let body = format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock>\
                 <D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:depth>0</D:depth>\
                 <D:timeout>Second-3600</D:timeout><D:locktoken><D:href>{token}</D:href></D:locktoken>\
                 </D:activelock></D:lockdiscovery></D:prop>\n"
            );
            let mut r = Response::new(Full::new(Bytes::from(body)));
            r.headers_mut().insert(
                "Lock-Token",
                format!("<{token}>").parse().expect("ascii token"),
            );
            r.headers_mut().insert(
                "Content-Type",
                "application/xml; charset=utf-8"
                    .parse()
                    .expect("static header"),
            );
            r
        }
        "UNLOCK" => status(StatusCode::NO_CONTENT),
        "PROPPATCH" => {
            let body = format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\"><D:response><D:href>{}</D:href>\
                 <D:propstat><D:prop/><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>\n",
                xml(req.uri().path())
            );
            let mut r = Response::new(Full::new(Bytes::from(body)));
            *r.status_mut() = StatusCode::MULTI_STATUS;
            r
        }
        _ => status(StatusCode::FORBIDDEN),
    }
}

/// A running server; dropping it stops it.
#[derive(Debug)]
pub struct Server {
    pub url: String,
    stop: watch::Sender<bool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

fn random_token() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Serve `root` on `127.0.0.1:<free port>/<random token>/`.
///
/// # Errors
/// If no loopback port can be bound.
pub async fn serve(root: PathBuf) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let tree = Arc::new(Tree {
        root,
        token: random_token()?,
    });
    let url = format!("http://127.0.0.1:{port}{}", tree.prefix());
    let (stop, mut rx) = watch::channel(false);
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    tokio::spawn(async move {
        let limit = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        loop {
            tokio::select! {
                _ = rx.changed() => return,
                acc = listener.accept() => {
                    let Ok((stream, _)) = acc else { continue };
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                    let tree = tree.clone();
                    let hosts = hosts.clone();
                    tokio::spawn(async move {
                        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                            let tree = tree.clone();
                            let ok = req.headers().get("host").and_then(|h| h.to_str().ok()).is_some_and(|h| hosts.iter().any(|x| x == h));
                            async move {
                                Ok::<_, Infallible>(if ok { handle(&tree, req).await } else { status(StatusCode::FORBIDDEN) })
                            }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(Duration::from_secs(15))
                            .serve_connection(TokioIo::new(stream), svc)
                            .await;
                        drop(permit);
                    });
                }
            }
        }
    });
    Ok(Server { url, stop })
}
