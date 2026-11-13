//! Domain allowlist (§6.4): a local HTTP proxy that confined commands are pointed at through
//! `HTTP_PROXY` / `HTTPS_PROXY` (and the lowercase and `ALL_PROXY` forms).
//!
//! It understands `CONNECT host:port` (HTTPS, git, anything tunnelled) and plain `http://` requests
//! in absolute form. The host name is checked against `[sandbox] network` before anything is
//! resolved; DNS resolution and the outgoing connection happen here, outside the sandbox, so the
//! confined process needs no DNS of its own. Anything else is answered `403` and closed.
//!
//! Where the backend can restrict sockets (Seatbelt), the only address a command may reach is this
//! proxy, so the list is enforced. Where it cannot (bubblewrap shares the network once it is on),
//! programs that honour the proxy variables are filtered and the others are not; see
//! [`crate::Filter`].

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// Longest request head the proxy reads before giving up.
const MAX_HEAD: usize = 16 * 1024;

/// Whether `host:port` matches an allowlist entry (`host:port`, `*.domain:port`, `*:port`,
/// `host:*`, `*`). Host names compare case-insensitively; `*.example.com` matches subdomains
/// only.
pub fn allowed(allow: &[String], host: &str, port: u16) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allow.iter().any(|entry| {
        if entry == "*" {
            return true;
        }
        let Some((h, p)) = entry.rsplit_once(':') else {
            return false;
        };
        let port_ok = p == "*" || p.parse::<u16>().is_ok_and(|p| p == port);
        let h = h.trim_end_matches('.').to_ascii_lowercase();
        let host_ok = match h.strip_prefix("*.") {
            _ if h == "*" => true,
            Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
            None => h.trim_matches(['[', ']']) == host.trim_matches(['[', ']']),
        };
        port_ok && host_ok
    })
}

/// A running proxy; stops when dropped.
#[derive(Debug)]
pub struct Proxy {
    pub port: u16,
    blocked: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    /// Listen on `127.0.0.1` (a free port) for one command. Needs a Tokio runtime.
    pub fn start(allow: Vec<String>) -> std::io::Result<Self> {
        let std = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        std.set_nonblocking(true)?;
        let port = std.local_addr()?.port();
        let listener = TcpListener::from_std(std)?;
        let allow = Arc::new(allow);
        let blocked: Arc<Mutex<Vec<String>>> = Arc::default();
        let b2 = blocked.clone();
        let task = tokio::spawn(async move {
            while let Ok((conn, _)) = listener.accept().await {
                let (allow, blocked) = (allow.clone(), b2.clone());
                tokio::spawn(async move {
                    let _ = serve(conn, &allow, &blocked).await;
                });
            }
        });
        Ok(Self {
            port,
            blocked,
            task,
        })
    }

    /// `host:port` the command asked for and was refused, oldest first.
    pub fn blocked(&self) -> Vec<String> {
        self.blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Environment that points a command at this proxy.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let url = format!("http://127.0.0.1:{}", self.port);
        [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
        .into_iter()
        .map(|k| (k, url.clone()))
        .collect()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// `host:port` from `authority` (`example.com:443`, `[::1]:8080`, `example.com`).
fn split_authority(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if tail.is_empty() => default_port,
            None => return None,
        };
        return Some((h.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) => Some((h.to_owned(), p.parse().ok()?)),
        None => Some((authority.to_owned(), default_port)),
    }
    .filter(|(h, _)| !h.is_empty())
}

/// What the client asked for.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    host: String,
    port: u16,
    /// `None` for `CONNECT`; else the head to send upstream (origin-form, `Connection: close`).
    forward: Option<Vec<u8>>,
}

fn parse(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_authority(target, 443)?;
        return Some(Request {
            host,
            port,
            forward: None,
        });
    }
    let rest = target.strip_prefix("http://")?;
    let (authority, path) = rest.find('/').map_or((rest, "/"), |i| rest.split_at(i));
    let (host, port) = split_authority(authority, 80)?;
    let mut out = format!("{method} {path} {version}\r\n");
    for l in lines.filter(|l| !l.is_empty()) {
        let name = l.split(':').next().unwrap_or_default().to_ascii_lowercase();
        // One request per connection: a kept-alive connection could otherwise reach another host.
        if name.starts_with("proxy-") || name == "connection" || name == "keep-alive" {
            continue;
        }
        out.push_str(l);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    Some(Request {
        host,
        port,
        forward: Some(out.into_bytes()),
    })
}

async fn serve(
    mut conn: TcpStream,
    allow: &[String],
    blocked: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let end = loop {
        let mut chunk = [0u8; 2048];
        let n = conn.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return reply(&mut conn, "431 Request Header Fields Too Large", "").await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let Some(req) = parse(&head) else {
        return reply(
            &mut conn,
            "400 Bad Request",
            "kor sandbox proxy: unsupported request\n",
        )
        .await;
    };
    if !allowed(allow, &req.host, req.port) {
        blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{}:{}", req.host, req.port));
        let msg = format!(
            "kor sandbox: {}:{} is not in sandbox.network\n",
            req.host, req.port
        );
        return reply(&mut conn, "403 Forbidden", &msg).await;
    }
    // Resolved here, outside the sandbox.
    let Ok(mut up) = TcpStream::connect((req.host.as_str(), req.port)).await else {
        return reply(
            &mut conn,
            "502 Bad Gateway",
            "kor sandbox proxy: cannot connect\n",
        )
        .await;
    };
    match req.forward {
        None => {
            conn.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await?;
            // Bytes the client sent after the head (rare, e.g. TLS hello in the same packet).
            up.write_all(&buf[end..]).await?;
        }
        Some(head) => {
            up.write_all(&head).await?;
            up.write_all(&buf[end..]).await?;
        }
    }
    tokio::io::copy_bidirectional(&mut conn, &mut up).await?;
    Ok(())
}

async fn reply(conn: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let msg = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    conn.write_all(msg.as_bytes()).await?;
    conn.shutdown().await
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;

    fn addr(p: &Proxy) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, p.port))
    }

    fn list(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn allowlist_matching() {
        let a = list(&[
            "github.com:443",
            "*.npmjs.org:443",
            "localhost:*",
            "[::1]:8080",
        ]);
        assert!(allowed(&a, "github.com", 443));
        assert!(allowed(&a, "GitHub.com.", 443));
        assert!(!allowed(&a, "github.com", 80));
        assert!(!allowed(&a, "evil-github.com", 443));
        assert!(allowed(&a, "registry.npmjs.org", 443));
        assert!(
            !allowed(&a, "npmjs.org", 443),
            "wildcard is subdomains only"
        );
        assert!(!allowed(&a, "npmjs.org.evil.com", 443));
        assert!(allowed(&a, "localhost", 5432));
        assert!(allowed(&a, "::1", 8080));
        assert!(allowed(&list(&["*:443"]), "any.host", 443));
        assert!(allowed(&list(&["*"]), "any.host", 1));
        assert!(!allowed(&[], "github.com", 443));
    }

    #[test]
    fn requests_are_parsed_and_rewritten() {
        let r = parse("CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n").unwrap();
        assert_eq!(
            (r.host.as_str(), r.port, r.forward),
            ("github.com", 443, None)
        );
        let r = parse("CONNECT [::1]:8443 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!((r.host.as_str(), r.port), ("::1", 8443));
        let r = parse(
            "GET http://example.com/a?b HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\n\r\n",
        )
        .unwrap();
        assert_eq!((r.host.as_str(), r.port), ("example.com", 80));
        let fwd = String::from_utf8(r.forward.unwrap()).unwrap();
        assert_eq!(
            fwd,
            "GET /a?b HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(
            parse("GET http://h:8080 HTTP/1.1\r\n\r\n").unwrap().port,
            8080
        );
        assert!(parse("GET /relative HTTP/1.1\r\n\r\n").is_none());
        assert!(parse("CONNECT :443 HTTP/1.1\r\n\r\n").is_none());
    }

    async fn roundtrip(proxy: SocketAddr, request: String) -> String {
        let mut c = TcpStream::connect(proxy).await.unwrap();
        c.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = c.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn allowed_hosts_pass_and_others_get_403() {
        // A tiny upstream that answers one HTTP request.
        let up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let up_port = up.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = up.accept().await {
                tokio::spawn(async move {
                    let mut b = [0u8; 1024];
                    let n = s.read(&mut b).await.unwrap_or(0);
                    let first = String::from_utf8_lossy(&b[..n])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned();
                    let body = format!("upstream saw: {first}");
                    let _ = s
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                    let _ = s.shutdown().await;
                });
            }
        });
        let p = Proxy::start(list(&["localhost:*"])).unwrap();
        assert!(
            p.env()
                .iter()
                .any(|(k, v)| *k == "HTTPS_PROXY" && v.ends_with(&p.port.to_string()))
        );

        let ok = roundtrip(
            addr(&p),
            format!("GET http://localhost:{up_port}/x HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
        .await;
        assert!(ok.contains("upstream saw: GET /x HTTP/1.1"), "{ok}");

        let tunnel = roundtrip(
            addr(&p),
            format!("CONNECT localhost:{up_port} HTTP/1.1\r\n\r\nGET /t HTTP/1.1\r\n\r\n"),
        )
        .await;
        assert!(
            tunnel.starts_with("HTTP/1.1 200 Connection established"),
            "{tunnel}"
        );
        assert!(tunnel.contains("upstream saw: GET /t"), "{tunnel}");

        let denied = roundtrip(
            addr(&p),
            "CONNECT example.com:443 HTTP/1.1\r\n\r\n".to_owned(),
        )
        .await;
        assert!(denied.starts_with("HTTP/1.1 403"), "{denied}");
        assert_eq!(p.blocked(), ["example.com:443"]);
    }
}
