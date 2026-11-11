//! Devin CLI PKCE login. The authorize query keeps the official parameter order, and the
//! session token is stored with the `devin-session-token$` prefix the API expects.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Error, Net, Provider, Record, err, hex_encode, json_ok, query_raw, string_field};

const APP: &str = "https://app.devin.ai";
const API: &str = "https://api.devin.ai";
const SERVER: &str = "https://server.codeium.com";
const PREFIX: &str = "devin-session-token$";

pub(super) fn authorize_url(redirect: &str, challenge: &str, state: &str) -> String {
    let mut pairs = Vec::new();
    if !redirect.is_empty() {
        pairs.push(("redirect_uri", redirect));
    }
    if !state.is_empty() {
        pairs.push(("state", state));
    }
    pairs.push(("prompt", "select_account"));
    pairs.push(("code_challenge", challenge));
    pairs.push(("code_challenge_method", "S256"));
    if redirect.is_empty() {
        pairs.push(("cli_pkce_marker", "1"));
    }
    format!("{APP}/auth/cli/continue?{}", query_raw(&pairs))
}

pub(super) async fn exchange(net: &Net, code: &str, verifier: &str) -> Result<Record, Error> {
    let url = concat_api("/auth/cli/token");
    let (status, text) = net
        .post_json(
            &url,
            &json!({ "code": code.trim(), "code_verifier": verifier.trim() }),
            &[],
        )
        .await?;
    let body = json_ok(status, &text)?;
    let raw = string_field(&body, "token");
    if raw.is_empty() {
        return Err(err("devin token response has no token"));
    }
    let session = format_session(&raw);
    let profile = fetch_self(net, &session).await;
    let user_name = profile
        .as_ref()
        .map_or(String::new(), |p| string_field(p, "user_name"));
    let user_id = profile
        .as_ref()
        .map_or(String::new(), |p| string_field(p, "user_id"));
    let org_id = profile
        .as_ref()
        .map_or(String::new(), |p| string_field(p, "org_id"));
    let identifier = file_identifier(&session, &user_name, &user_id);
    let label = format!("Devin ({identifier})");
    Ok(Record {
        provider: Provider::Devin,
        file_name: format!("devin-{identifier}.json"),
        label,
        pretty: true,
        body: json!({
            "type": "devin",
            "api_key": session,
            "session_token": session,
            "user_name": user_name,
            "user_id": user_id,
            "org_id": org_id,
            "base_url": SERVER,
            "auth_kind": "oauth",
        }),
    })
}

fn concat_api(path: &str) -> String {
    format!("{API}{path}")
}

async fn fetch_self(net: &Net, session: &str) -> Option<Value> {
    let url = concat_api("/v3/self");
    let authorization = format!("Bearer {session}");
    let (status, text) = net
        .get(&url, &[("authorization", authorization.as_str())])
        .await
        .ok()?;
    json_ok(status, &text).ok()
}

fn format_session(raw: &str) -> String {
    let token = raw.trim();
    if token.starts_with(PREFIX) || !token.starts_with("eyJ") {
        token.to_owned()
    } else {
        format!("{PREFIX}{token}")
    }
}

fn file_identifier(session: &str, user_name: &str, user_id: &str) -> String {
    let identifier = if !user_name.is_empty() {
        user_name
    } else if !user_id.is_empty() {
        user_id
    } else {
        return hash_user(session);
    };
    let cleaned: String = identifier
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned != identifier || cleaned.len() > 160 {
        hash_user(identifier)
    } else {
        cleaned
    }
}

fn hash_user(data: &str) -> String {
    let digest = Sha256::digest(data.as_bytes());
    format!("user-{}", hex_encode(&digest[..8]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn authorize_url_keeps_the_cli_parameter_order() {
        let url = authorize_url("http://localhost:9/callback", "challenge", "state");
        assert_eq!(
            url,
            "https://app.devin.ai/auth/cli/continue?redirect_uri=http%3A%2F%2Flocalhost%3A9%2Fcallback&state=state&prompt=select_account&code_challenge=challenge&code_challenge_method=S256"
        );
        assert!(authorize_url("", "challenge", "").contains("cli_pkce_marker=1"));
    }

    #[test]
    fn session_tokens_gain_the_required_prefix() {
        assert_eq!(format_session("eyJabc"), "devin-session-token$eyJabc");
        assert_eq!(
            format_session("devin-session-token$eyJabc"),
            "devin-session-token$eyJabc"
        );
        assert_eq!(format_session("plain"), "plain");
    }

    #[tokio::test]
    async fn login_writes_a_prefixed_session_file() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/cli/token"))
            .and(body_string_contains("the-code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"token":"eyJabc"}"#))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/self"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"user_name":"ada","user_id":"u1","org_id":"o1"}"#),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let pending = super::super::begin(
            super::super::Provider::Devin,
            &super::super::Options {
                auth_dir: dir.path().to_owned(),
                open_browser: false,
                http_origin: Some(server.uri()),
                callback_timeout: std::time::Duration::from_secs(3),
                poll_floor: std::time::Duration::from_millis(15),
            },
        )
        .await
        .unwrap();
        let url = match &pending.prompt {
            super::super::Prompt::Browser { url } => url.clone(),
            super::super::Prompt::Device { .. } => panic!("expected a browser prompt"),
        };
        approve(&url, "the-code").await;
        let saved = pending.finish().await.unwrap();
        assert_eq!(saved.file_name, "devin-ada.json");
        let text = std::fs::read_to_string(&saved.path).unwrap();
        assert!(text.contains("devin-session-token$eyJabc"));
        assert!(text.contains("https://server.codeium.com"));
    }

    async fn approve(url: &str, code: &str) {
        let query = url.split_once('?').expect("query").1;
        let mut state = String::new();
        let mut redirect = String::new();
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key == "state" {
                state = super::super::pct_decode(value);
            } else if key == "redirect_uri" {
                redirect = super::super::pct_decode(value);
            }
        }
        let port: u16 = redirect
            .trim_start_matches("http://localhost:")
            .trim_end_matches("/callback")
            .parse()
            .expect("port");
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request = format!(
            "GET /callback?code={code}&state={state} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.expect("write");
    }
}
