//! Remote MCP OAuth. Discovery is:
//! `/.well-known/oauth-protected-resource`, then
//! `/.well-known/oauth-authorization-server` on the advertised issuer.
//! Tokens go through the existing secret-store trait. No URL is invented.

use std::time::Duration;

use kor_secrets::{ExposeSecret, SecretStore, SecretString};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEndpoints {
    pub authorization: String,
    pub token: String,
}

pub fn account(server: &str) -> String {
    format!("mcp-oauth-{server}")
}

/// RFC 9728 then RFC 8414 against `mcp_base` (no path).
pub async fn discover(mcp_base: &str) -> Result<AuthEndpoints, String> {
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let protected = client
        .get(format!(
            "{}/.well-known/oauth-protected-resource",
            mcp_base.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !protected.status().is_success() {
        return Err(format!(
            "oauth-protected-resource returned {}",
            protected.status()
        ));
    }
    let body: Value = serde_json::from_str(&protected.text().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let issuer = body
        .get("authorization_servers")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or("authorization_servers[0] missing")?;
    let meta = client
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            issuer.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !meta.status().is_success() {
        return Err(format!(
            "oauth-authorization-server returned {}",
            meta.status()
        ));
    }
    let doc: Value = serde_json::from_str(&meta.text().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let authorization = doc
        .get("authorization_endpoint")
        .and_then(Value::as_str)
        .ok_or("authorization_endpoint missing")?
        .to_owned();
    let token = doc
        .get("token_endpoint")
        .and_then(Value::as_str)
        .ok_or("token_endpoint missing")?
        .to_owned();
    Ok(AuthEndpoints {
        authorization,
        token,
    })
}

pub fn store_token(store: &dyn SecretStore, server: &str, access: &str) -> Result<(), String> {
    store
        .set(
            &account(server),
            &SecretString::from(format!(r#"{{"access_token":"{access}"}}"#)),
        )
        .map_err(|e| e.to_string())
}

pub fn load_token(store: &dyn SecretStore, server: &str) -> Result<Option<String>, String> {
    let Some(secret) = store.get(&account(server)).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(secret.expose_secret()).map_err(|e| e.to_string())?;
    Ok(value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// Accept one `/callback?code=` on an already-bound listener.
pub async fn wait_code(listener: TcpListener) -> Result<String, String> {
    let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 2048];
    let n = sock.read(&mut buf).await.map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let code = req
        .split_once("code=")
        .and_then(|(_, rest)| rest.split(['&', ' ']).next())
        .ok_or("callback has no code")?
        .to_owned();
    let _ = sock
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
        .await;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_discovery(listener: TcpListener, issuer: String, auth: String, token: String) {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 2048];
            let Ok(n) = sock.read(&mut buf).await else {
                break;
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let body = if req.contains("oauth-protected-resource") {
                format!(r#"{{"authorization_servers":["{issuer}"]}}"#)
            } else if req.contains("oauth-authorization-server") {
                format!(r#"{{"authorization_endpoint":"{auth}","token_endpoint":"{token}"}}"#)
            } else {
                "{}".into()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    }

    #[tokio::test]
    async fn discovery_and_token_store() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let auth = format!("{base}/authorize");
        let token = format!("{base}/token");
        tokio::spawn(serve_discovery(
            listener,
            base.clone(),
            auth.clone(),
            token.clone(),
        ));
        let found = discover(&base).await.unwrap();
        assert_eq!(found.authorization, auth);
        assert_eq!(found.token, token);
        let cb = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cb_port = cb.local_addr().unwrap().port();
        let wait = tokio::spawn(wait_code(cb));
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", cb_port))
            .await
            .unwrap();
        client
            .write_all(b"GET /callback?code=abc&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        assert_eq!(wait.await.unwrap().unwrap(), "abc");
        let store = MemoryStore::default();
        store_token(&store, "remote", "tok").unwrap();
        assert_eq!(
            load_token(&store, "remote").unwrap().as_deref(),
            Some("tok")
        );
    }
}
