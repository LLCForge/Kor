//! GitHub Copilot device login. The device grant uses the Copilot VS Code OAuth client,
//! then exchanges the GitHub token for a short-lived Copilot API token.

use serde_json::{Value, json};

use super::{Error, Net, Provider, Record, err, json_i64, json_ok, string_field, unix_millis};

const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEVICE: &str = "https://github.com/login/device/code";
const ACCESS: &str = "https://github.com/login/oauth/access_token";
const COPILOT: &str = "https://api.github.com/copilot_internal/v2/token";
const USER: &str = "https://api.github.com/user";
const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const EDITOR: &str = "vscode/1.98.2";
const PLUGIN: &str = "copilot-chat/0.26.7";
const USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

pub(super) struct DeviceStart {
    pub(super) device_code: String,
    pub(super) user_code: String,
    pub(super) verification_uri: String,
    pub(super) verification_uri_complete: Option<String>,
    pub(super) interval: std::time::Duration,
}

pub(super) async fn start_device(net: &Net) -> Result<DeviceStart, Error> {
    let (status, text) = net
        .post_json(
            DEVICE,
            &json!({ "client_id": CLIENT_ID, "scope": "read:user" }),
            &[],
        )
        .await?;
    let body = json_ok(status, &text)?;
    let device_code = string_field(&body, "device_code");
    let user_code = string_field(&body, "user_code");
    if device_code.is_empty() || user_code.is_empty() {
        return Err(err("copilot device flow did not return a user code"));
    }
    let secs = u64::try_from(json_i64(&body, "interval")).unwrap_or(0);
    let complete = string_field(&body, "verification_uri_complete");
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri: {
            let uri = string_field(&body, "verification_uri");
            if uri.is_empty() {
                "https://github.com/login/device".to_owned()
            } else {
                uri
            }
        },
        verification_uri_complete: if complete.is_empty() {
            None
        } else {
            Some(complete)
        },
        interval: if secs == 0 {
            net.poll_floor
        } else {
            std::time::Duration::from_secs(secs)
        },
    })
}

pub(super) async fn finish_device(
    net: &Net,
    device_code: &str,
    mut interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<Record, Error> {
    if interval.is_zero() {
        interval = net.poll_floor;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let github = loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(err("copilot device login timed out"));
        }
        let (status, text) = net
            .post_json(
                ACCESS,
                &json!({ "client_id": CLIENT_ID, "device_code": device_code, "grant_type": GRANT }),
                &[],
            )
            .await?;
        match poll_state(status, &text)? {
            Poll::Pending => tokio::time::sleep(interval).await,
            Poll::SlowDown => {
                interval += std::time::Duration::from_secs(5);
                tokio::time::sleep(interval).await;
            }
            Poll::Ready(token) => break token,
        }
    };
    let copilot = copilot_token(net, &github).await?;
    let login = github_login(net, &github).await.unwrap_or_default();
    let file_name = if login.is_empty() {
        format!("github-copilot-{}.json", unix_millis())
    } else {
        format!("github-copilot-{login}.json")
    };
    Ok(Record {
        provider: Provider::Copilot,
        label: if login.is_empty() {
            "GitHub Copilot".to_owned()
        } else {
            login.clone()
        },
        file_name,
        pretty: true,
        body: json!({
            "type": "github-copilot",
            "access_token": github,
            "token": string_field(&copilot, "token"),
            "expires_at": json_i64(&copilot, "expires_at"),
            "refresh_in": json_i64(&copilot, "refresh_in"),
            "login": login,
            "endpoints": copilot.get("endpoints").cloned().unwrap_or(Value::Null),
        }),
    })
}

pub(super) async fn refresh(net: &Net, doc: &mut Value) -> Result<(), Error> {
    let github = string_field(doc, "access_token");
    if github.is_empty() {
        return Err(err("copilot auth file has no GitHub access token"));
    }
    let copilot = copilot_token(net, &github).await?;
    let token = string_field(&copilot, "token");
    if token.is_empty() {
        return Err(err("copilot token refresh returned an empty token"));
    }
    doc["token"] = json!(token);
    doc["expires_at"] = json!(json_i64(&copilot, "expires_at"));
    doc["refresh_in"] = json!(json_i64(&copilot, "refresh_in"));
    if copilot.get("endpoints").is_some() {
        doc["endpoints"] = copilot["endpoints"].clone();
    }
    Ok(())
}

#[derive(Debug)]
enum Poll {
    Pending,
    SlowDown,
    Ready(String),
}

fn poll_state(status: u16, body: &str) -> Result<Poll, Error> {
    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    match parsed.get("error").and_then(Value::as_str).unwrap_or("") {
        "authorization_pending" => Ok(Poll::Pending),
        "slow_down" => Ok(Poll::SlowDown),
        "expired_token" | "expired" => Err(err("copilot device code expired")),
        "access_denied" => Err(err("copilot device login was denied")),
        "" if (200..300).contains(&status) => {
            let token = string_field(&parsed, "access_token");
            if token.is_empty() {
                Err(err("copilot device login returned no GitHub token"))
            } else {
                Ok(Poll::Ready(token))
            }
        }
        other if !other.is_empty() => Err(err(format!("copilot device login: {other}"))),
        _ => Err(super::http_error(status, body)),
    }
}

async fn copilot_token(net: &Net, github: &str) -> Result<Value, Error> {
    let authorization = format!("token {github}");
    let (status, text) = net
        .get(
            COPILOT,
            &[
                ("authorization", &authorization),
                ("editor-version", EDITOR),
                ("editor-plugin-version", PLUGIN),
                ("user-agent", USER_AGENT),
            ],
        )
        .await?;
    let body = json_ok(status, &text)?;
    if string_field(&body, "token").is_empty() {
        return Err(err("copilot token response has no token"));
    }
    Ok(body)
}

async fn github_login(net: &Net, github: &str) -> Result<String, Error> {
    let authorization = format!("token {github}");
    let (status, text) = net
        .get(
            USER,
            &[
                ("authorization", &authorization),
                ("user-agent", USER_AGENT),
            ],
        )
        .await?;
    let body = json_ok(status, &text)?;
    Ok(string_field(&body, "login"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_and_denied_are_distinct() {
        let pending = poll_state(400, r#"{"error":"authorization_pending"}"#).unwrap();
        assert!(matches!(pending, Poll::Pending));
        let denied = poll_state(400, r#"{"error":"access_denied"}"#).unwrap_err();
        assert!(denied.to_string().contains("denied"));
    }

    #[tokio::test]
    async fn device_login_stores_the_github_token_and_the_copilot_token() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"device_code":"dev","user_code":"WXYZ-9876","verification_uri":"https://github.com/login/device","expires_in":900,"interval":0}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"gho_test","token_type":"bearer","scope":"read:user"}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"authorization_pending"}"#),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/copilot_internal/v2/token"))
            .and(header("authorization", "token gho_test"))
            .and(header("editor-version", "vscode/1.98.2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"token":"tid=copilot","expires_at":1893456000,"refresh_in":1500,"endpoints":{"api":"https://api.githubcopilot.com"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"login":"octocat"}"#))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let pending = super::super::begin(
            super::super::Provider::Copilot,
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
        let saved = pending.finish().await.unwrap();
        assert_eq!(
            saved.path.file_name().and_then(|n| n.to_str()),
            Some("github-copilot-octocat.json")
        );
        let text = std::fs::read_to_string(&saved.path).unwrap();
        assert!(text.contains("gho_test"));
        assert!(text.contains("tid=copilot"));
        assert!(text.contains("github-copilot"));
    }
}
