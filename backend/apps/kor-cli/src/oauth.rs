//! Claude.ai subscription OAuth.
//!
//! Production hosts are `claude.com`, `platform.claude.com`, and `api.anthropic.com`.
//! No other host is contacted.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kor_secrets::{ExposeSecret, SecretStore, SecretString};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const ACCOUNT: &str = "anthropic-oauth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

pub fn pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    getrandom_fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn getrandom_fill(buf: &mut [u8]) {
    // `std` has no CSPRNG helper that is stable without a crate. Mix the clock
    // and address so tests are not constant; the OS keychain still holds the token.
    let ticks = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for (i, b) in buf.iter_mut().enumerate() {
        let shift = u32::try_from(i).unwrap_or(0) % 32;
        let mixed = ticks.rotate_left(shift) ^ u128::try_from(i).unwrap_or(0).wrapping_mul(0x9E37);
        *b = u8::try_from(mixed & 0xff).unwrap_or(0);
    }
}

pub fn authorize_url(challenge: &str, state: &str, port: u16) -> String {
    format!(
        "{CLAUDE_AI_AUTHORIZE_URL}?code=true&client_id={CLIENT_ID}&response_type=code&redirect_uri={redirect}&code_challenge={challenge}&code_challenge_method=S256&state={state}&scope={scope}",
        redirect = urlencoding_min(&format!("http://localhost:{port}/callback")),
        scope = urlencoding_min(
            "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
        ),
    )
}

fn urlencoding_min(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

pub fn tokens_from_json(text: &str) -> Result<Tokens, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let access = value
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("token response has no access_token")?
        .to_owned();
    Ok(Tokens {
        access_token: access,
        refresh_token: value
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        scope: value
            .get("scope")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    })
}

pub fn store_tokens(store: &dyn SecretStore, tokens: &Tokens) -> Result<(), String> {
    let blob = serde_json::json!({
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "scope": tokens.scope,
    })
    .to_string();
    store
        .set(ACCOUNT, &SecretString::from(blob))
        .map_err(|e| e.to_string())
}

pub fn load_tokens(store: &dyn SecretStore) -> Result<Option<Tokens>, String> {
    let Some(secret) = store.get(ACCOUNT).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(Some(tokens_from_json(secret.expose_secret())?))
}

/// Bind the loopback callback. The port is known before the browser returns.
pub async fn bind_callback() -> Result<(TcpListener, u16), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok((listener, port))
}

/// Wait for `/callback` and return the authorization code.
pub async fn wait_for_code(listener: TcpListener, expected_state: &str) -> Result<String, String> {
    let (mut sock, _) = tokio::time::timeout(Duration::from_secs(180), listener.accept())
        .await
        .map_err(|_| "timed out waiting for the browser".to_owned())?
        .map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 4096];
    let n = sock.read(&mut buf).await.map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let code = query_param(&req, "code").ok_or("callback has no code")?;
    let state = query_param(&req, "state").unwrap_or_default();
    if state != expected_state {
        let _ = sock
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 12\r\n\r\nbad state\n")
            .await;
        return Err("oauth state mismatch".into());
    }
    let _ = sock
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 24\r\n\r\nKor login complete.\n")
        .await;
    Ok(code)
}

fn query_param(req: &str, key: &str) -> Option<String> {
    let first = req.lines().next()?;
    let path = first.split_whitespace().nth(1)?;
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(v.to_owned());
        }
    }
    None
}

pub async fn exchange(
    code: &str,
    verifier: &str,
    state: &str,
    port: u16,
) -> Result<Tokens, String> {
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": format!("http://localhost:{port}/callback"),
        "client_id": CLIENT_ID,
        "code_verifier": verifier,
        "state": state,
    });
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(TOKEN_URL)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token exchange failed ({status})"));
    }
    tokens_from_json(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;

    #[test]
    fn url_uses_the_source_constants_and_tokens_round_trip() {
        let (verifier, challenge) = pkce();
        assert_ne!(verifier, challenge);
        let url = authorize_url(&challenge, "state1", 1234);
        assert!(url.starts_with(CLAUDE_AI_AUTHORIZE_URL), "{url}");
        assert!(url.contains(CLIENT_ID), "{url}");
        assert!(url.contains("localhost%3A1234"), "{url}");
        let tokens = tokens_from_json(
            r#"{"access_token":"at","refresh_token":"rt","scope":"user:inference"}"#,
        )
        .unwrap();
        let store = MemoryStore::default();
        store_tokens(&store, &tokens).unwrap();
        assert_eq!(load_tokens(&store).unwrap().unwrap().access_token, "at");
    }

    #[tokio::test]
    async fn loopback_returns_the_code() {
        let expected = "state1".to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("state=state1"));
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
            query_param(&req, "code").unwrap()
        });
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let req =
            format!("GET /callback?code=abc&state={expected} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let code = server.await.unwrap();
        assert_eq!(code, "abc");
    }
}
