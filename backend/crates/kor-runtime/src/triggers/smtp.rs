//! Minimal SMTP submission for `[notify]` e-mail: implicit TLS (465) or STARTTLS, AUTH PLAIN,
//! one plain-text UTF-8 message. A password is never sent over a cleartext connection; plain
//! SMTP is only used for a loopback relay without a password (and the test servers).

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use kor_secrets::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls;

#[derive(Debug, Clone)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<SecretString>,
}

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE: usize = 4096;

async fn reply<S: AsyncRead + Unpin>(r: &mut BufReader<S>) -> Result<(u16, String), String> {
    let mut all = String::new();
    loop {
        let mut line = String::new();
        let n = tokio::time::timeout(TIMEOUT, r.read_line(&mut line))
            .await
            .map_err(|_| "SMTP: timeout".to_owned())?
            .map_err(|e| format!("SMTP: {e}"))?;
        if n == 0 || line.len() > MAX_LINE || all.len() > 16 * MAX_LINE {
            return Err("SMTP: connection closed".into());
        }
        let code = line
            .get(..3)
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or("SMTP: bad reply")?;
        all.push_str(line.get(4..).unwrap_or_default());
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, all));
        }
    }
}

async fn cmd<S: AsyncRead + AsyncWrite + Unpin>(
    r: &mut BufReader<S>,
    line: &str,
    want: u16,
) -> Result<String, String> {
    r.get_mut()
        .write_all(format!("{line}\r\n").as_bytes())
        .await
        .map_err(|e| format!("SMTP: {e}"))?;
    let (code, text) = reply(r).await?;
    if code == want {
        Ok(text)
    } else {
        // The line may hold credentials (AUTH); name only the verb.
        let verb = line.split_whitespace().next().unwrap_or_default();
        Err(format!("SMTP {verb}: {code} {}", text.trim()))
    }
}

fn header_safe(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\r' | '\n')).collect()
}

fn address_ok(a: &str) -> bool {
    a.len() <= 254
        && a.contains('@')
        && !a
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | ',' | '\r' | '\n'))
}

/// The DATA payload: headers, base64 body, dot-stuffing not needed (base64 has no leading dots).
pub fn message(from: &str, to: &[String], subject: &str, body: &str, date: &str) -> String {
    let subject = format!("=?UTF-8?B?{}?=", B64.encode(header_safe(subject)));
    let encoded = B64.encode(body.replace("\r\n", "\n").replace('\n', "\r\n"));
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    format!(
        "From: {}\r\nTo: {}\r\nSubject: {subject}\r\nDate: {date}\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n.",
        header_safe(from),
        header_safe(&to.join(", ")),
        lines.join("\r\n")
    )
}

async fn deliver<S: AsyncRead + AsyncWrite + Unpin>(
    r: &mut BufReader<S>,
    s: &Server,
    secure: bool,
    from: &str,
    to: &[String],
    data: &str,
) -> Result<(), String> {
    if let Some(p) = &s.password {
        if !secure {
            return Err("SMTP: refusing to send a password without TLS".into());
        }
        let plain = B64.encode(format!("\0{}\0{}", s.user, p.expose_secret()));
        cmd(r, &format!("AUTH PLAIN {plain}"), 235).await?;
    }
    cmd(r, &format!("MAIL FROM:<{from}>"), 250).await?;
    for t in to {
        cmd(r, &format!("RCPT TO:<{t}>"), 250).await?;
    }
    cmd(r, "DATA", 354).await?;
    cmd(r, data, 250).await?;
    let _ = cmd(r, "QUIT", 221).await;
    Ok(())
}

fn tls() -> Result<tokio_rustls::TlsConnector, String> {
    use rustls_platform_verifier::ConfigVerifierExt as _;
    let config = rustls::ClientConfig::with_platform_verifier().map_err(|e| format!("TLS: {e}"))?;
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

async fn upgrade(
    plain: TcpStream,
    host: &str,
) -> Result<BufReader<tokio_rustls::client::TlsStream<TcpStream>>, String> {
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| format!("TLS: {e}"))?;
    let stream = tls()?
        .connect(name, plain)
        .await
        .map_err(|e| format!("TLS: {e}"))?;
    Ok(BufReader::new(stream))
}

/// Send one message.
///
/// # Errors
/// Blocked by privacy/offline mode, connection, TLS or SMTP failures (no credentials in them).
pub async fn send(
    s: &Server,
    from: &str,
    to: &[String],
    subject: &str,
    body: &str,
) -> Result<(), String> {
    kor_core::net::check_host(&s.host).map_err(|b| b.to_string())?;
    if !address_ok(from) || to.is_empty() || !to.iter().all(|t| address_ok(t)) {
        return Err("SMTP: invalid address".into());
    }
    let date = chrono::Local::now().to_rfc2822();
    let payload = message(from, to, subject, body, &date);
    let tcp = tokio::time::timeout(TIMEOUT, TcpStream::connect((s.host.as_str(), s.port)))
        .await
        .map_err(|_| "SMTP: timeout".to_owned())?
        .map_err(|e| format!("SMTP {}: {e}", s.host))?;
    if s.port == 465 {
        let mut r = upgrade(tcp, &s.host).await?;
        reply(&mut r).await?;
        cmd(&mut r, "EHLO kor", 250).await?;
        return deliver(&mut r, s, true, from, to, &payload).await;
    }
    let mut r = BufReader::new(tcp);
    reply(&mut r).await?;
    let caps = cmd(&mut r, "EHLO kor", 250).await?;
    if caps
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("STARTTLS"))
    {
        cmd(&mut r, "STARTTLS", 220).await?;
        let mut t = upgrade(r.into_inner(), &s.host).await?;
        cmd(&mut t, "EHLO kor", 250).await?;
        return deliver(&mut t, s, true, from, to, &payload).await;
    }
    // Plain SMTP: only to this computer.
    if !kor_core::net::is_loopback_host(&s.host) {
        return Err("SMTP: server offers no TLS".into());
    }
    deliver(&mut r, s, false, from, to, &payload).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::AsyncReadExt as _;

    /// A scripted SMTP server on loopback; returns its port and the received DATA.
    pub(crate) async fn mock_server(starttls: bool) -> (u16, tokio::task::JoinHandle<String>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let h = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            s.write_all(b"220 mock\r\n").await.unwrap();
            let mut buf = Vec::new();
            let mut data = String::new();
            let mut in_data = false;
            loop {
                let mut chunk = [0u8; 4096];
                let n = s.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
                    let line = String::from_utf8_lossy(&buf[..pos]).into_owned();
                    buf.drain(..pos + 2);
                    if in_data {
                        if line == "." {
                            in_data = false;
                            s.write_all(b"250 queued\r\n").await.unwrap();
                        } else {
                            data.push_str(&line);
                            data.push('\n');
                        }
                        continue;
                    }
                    let reply: &[u8] = if line.starts_with("EHLO") {
                        if starttls {
                            b"250-mock\r\n250 STARTTLS\r\n"
                        } else {
                            b"250-mock\r\n250 8BITMIME\r\n"
                        }
                    } else if line == "DATA" {
                        in_data = true;
                        b"354 go\r\n"
                    } else if line == "QUIT" {
                        s.write_all(b"221 bye\r\n").await.unwrap();
                        return data;
                    } else if line == "STARTTLS" {
                        b"454 no tls here\r\n"
                    } else {
                        b"250 ok\r\n"
                    };
                    s.write_all(reply).await.unwrap();
                }
            }
            data
        });
        (port, h)
    }

    #[tokio::test]
    async fn sends_utf8_mail_to_a_loopback_relay() {
        let (port, h) = mock_server(false).await;
        let s = Server {
            host: "127.0.0.1".into(),
            port,
            user: String::new(),
            password: None,
        };
        send(
            &s,
            "kor@example.com",
            &["me@example.com".into()],
            "Onay gerekiyor: ğüş",
            "Görev bekliyor\nsatır 2",
        )
        .await
        .unwrap();
        let data = h.await.unwrap();
        assert!(data.contains("Subject: =?UTF-8?B?"), "{data}");
        let body: String = data.split("\n\n").nth(1).unwrap().lines().collect();
        let decoded = String::from_utf8(B64.decode(body.trim()).unwrap()).unwrap();
        assert_eq!(decoded, "Görev bekliyor\r\nsatır 2");
    }

    #[tokio::test]
    async fn never_sends_a_password_in_clear() {
        let (port, _h) = mock_server(false).await;
        let s = Server {
            host: "127.0.0.1".into(),
            port,
            user: "me".into(),
            password: Some(SecretString::from("hunter2".to_owned())),
        };
        let e = send(&s, "kor@example.com", &["me@example.com".into()], "s", "b")
            .await
            .unwrap_err();
        assert!(e.contains("without TLS") && !e.contains("hunter2"), "{e}");
    }

    #[test]
    fn headers_cannot_be_injected() {
        let m = message("a@b.c", &["d@e.f".into()], "x\r\nBcc: evil@x.y", "b", "now");
        assert!(!m.contains("\r\nBcc:"));
        assert!(!address_ok("a@b.c>\r\nRCPT TO:<x@y.z"));
    }
}
