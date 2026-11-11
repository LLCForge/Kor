//! Kimi device-code login (RFC 8628) for `kimi.com` and `kimi.ai`, using Kimi Code's client id.

use serde_json::{Value, json};

use super::{
    Error, Net, Provider, Record, err, form, json_i64, json_ok, rfc3339_after, string_field,
    unix_millis, uuid_v4,
};

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub(super) struct DeviceStart {
    pub(super) device_code: String,
    pub(super) user_code: String,
    pub(super) verification_uri: String,
    pub(super) verification_uri_complete: Option<String>,
    pub(super) interval: std::time::Duration,
    pub(super) device_id: String,
}

pub(super) async fn start_device(net: &Net, ai: bool) -> Result<DeviceStart, Error> {
    let device_id = uuid_v4()?;
    let (status, text) = post(
        net,
        &device_url(ai),
        &form(&[("client_id", CLIENT_ID)]),
        &device_id,
    )
    .await?;
    let body = json_ok(status, &text)?;
    let device_code = string_field(&body, "device_code");
    if device_code.is_empty() {
        return Err(err("kimi device flow did not return a device code"));
    }
    let complete = string_field(&body, "verification_uri_complete");
    Ok(DeviceStart {
        device_code,
        user_code: string_field(&body, "user_code"),
        verification_uri: string_field(&body, "verification_uri"),
        verification_uri_complete: if complete.is_empty() {
            None
        } else {
            Some(complete)
        },
        interval: kimi_interval(
            u64::try_from(json_i64(&body, "interval")).unwrap_or(0),
            net.poll_floor,
        ),
        device_id,
    })
}

pub(super) async fn finish_device(
    net: &Net,
    ai: bool,
    device_code: &str,
    device_id: &str,
    interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<Record, Error> {
    let deadline = tokio::time::Instant::now() + timeout;
    let form_body = form(&[
        ("client_id", CLIENT_ID),
        ("device_code", device_code),
        ("grant_type", DEVICE_GRANT),
    ]);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(err("kimi device code expired"));
        }
        tokio::time::sleep(interval).await;
        let (status, text) = post(net, &token_url(ai), &form_body, device_id).await?;
        if !(200..300).contains(&status) {
            return Err(super::http_error(status, &text));
        }
        let body: Value =
            serde_json::from_str(&text).map_err(|_| err("kimi token response was not JSON"))?;
        match body.get("error").and_then(Value::as_str).unwrap_or("") {
            "" => {
                if string_field(&body, "access_token").is_empty() {
                    return Err(err("kimi token response has no access token"));
                }
                return Ok(record(ai, device_id, &body));
            }
            "authorization_pending" | "slow_down" => {}
            "expired_token" => return Err(err("kimi device code expired")),
            "access_denied" => return Err(err("kimi login was denied")),
            other => return Err(err(format!("kimi oauth error: {other}"))),
        }
    }
}

#[allow(clippy::similar_names)] // paired names (in/out, old/new) read best side by side
pub(super) async fn refresh(net: &Net, doc: &mut Value) -> Result<(), Error> {
    let refresh_token = string_field(doc, "refresh_token");
    if refresh_token.is_empty() {
        return Err(err("kimi auth file has no refresh token"));
    }
    let ai = doc.get("type").and_then(Value::as_str) == Some("kimi-ai");
    let device_id = string_field(doc, "device_id");
    let form_body = form(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
    ]);
    let (status, text) = post(net, &token_url(ai), &form_body, &device_id).await?;
    if status == 401 || status == 403 {
        return Err(err(format!("kimi refresh was rejected (HTTP {status})")));
    }
    let body = json_ok(status, &text)?;
    let access = string_field(&body, "access_token");
    if access.is_empty() {
        return Err(err("kimi refresh returned no access token"));
    }
    doc["access_token"] = json!(access);
    let next = string_field(&body, "refresh_token");
    if !next.is_empty() {
        doc["refresh_token"] = json!(next);
    }
    let expires_in = json_i64(&body, "expires_in");
    if expires_in > 0 {
        doc["expired"] = json!(rfc3339_after(expires_in));
    }
    Ok(())
}

fn record(ai: bool, device_id: &str, body: &Value) -> Record {
    let access = string_field(body, "access_token");
    let expires_in = json_i64(body, "expires_in");
    let (provider, type_name, domain, base) = if ai {
        (
            Provider::KimiAi,
            "kimi-ai",
            "kimi.ai",
            "https://api.kimi.ai/coding",
        )
    } else {
        (
            Provider::Kimi,
            "kimi",
            "kimi.com",
            "https://api.kimi.com/coding",
        )
    };
    let mut stored = json!({
        "access_token": access,
        "refresh_token": string_field(body, "refresh_token"),
        "token_type": string_field(body, "token_type"),
        "scope": string_field(body, "scope"),
        "device_id": device_id,
        "type": type_name,
        "domain": domain,
        "base_url": base,
        "timestamp": unix_millis(),
    });
    if expires_in > 0 {
        stored["expired"] = json!(rfc3339_after(expires_in));
    }
    Record {
        provider,
        file_name: format!("{type_name}-{}.json", unix_millis()),
        label: if ai {
            "Kimi.ai User".to_owned()
        } else {
            "Kimi User".to_owned()
        },
        body: stored,
        pretty: true,
    }
}

fn device_url(ai: bool) -> String {
    format!("{}/api/oauth/device_authorization", host(ai))
}

fn token_url(ai: bool) -> String {
    format!("{}/api/oauth/token", host(ai))
}

fn host(ai: bool) -> &'static str {
    if ai {
        "https://auth.kimi.ai"
    } else {
        "https://auth.kimi.com"
    }
}

fn kimi_interval(secs: u64, floor: std::time::Duration) -> std::time::Duration {
    let asked = std::time::Duration::from_secs(secs);
    if secs == 0 || asked < floor {
        floor
    } else {
        asked
    }
}

async fn post(
    net: &Net,
    url: &str,
    form_body: &str,
    device_id: &str,
) -> Result<(u16, String), Error> {
    let name = host_name();
    let model = device_model();
    let version = env!("CARGO_PKG_VERSION");
    let headers = [
        ("x-msh-platform", "kor-proxy"),
        ("x-msh-version", version),
        ("x-msh-device-name", name.as_str()),
        ("x-msh-device-model", model.as_str()),
        ("x-msh-device-id", device_id),
    ];
    net.post_form(url, form_body, &headers).await
}

fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn device_model() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    format!("{os} {}", std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_never_goes_below_the_floor() {
        let floor = std::time::Duration::from_secs(5);
        assert_eq!(kimi_interval(1, floor), floor);
        assert_eq!(kimi_interval(0, floor), floor);
        assert_eq!(kimi_interval(10, floor), std::time::Duration::from_secs(10));
    }

    #[tokio::test]
    async fn device_login_writes_a_kimi_auth_file() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/device_authorization"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"device_code":"dev","user_code":"ABCD-1234","verification_uri":"https://auth.kimi.com/device","verification_uri_complete":"https://auth.kimi.com/device?code=ABCD-1234","expires_in":600,"interval":0}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"k-at","refresh_token":"k-rt","token_type":"Bearer","expires_in":3600,"scope":"chat"}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"error":"authorization_pending"}"#),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let opts = test_opts(dir.path(), &server.uri());
        let pending = super::super::begin(super::super::Provider::Kimi, &opts)
            .await
            .unwrap();
        let code = match &pending.prompt {
            super::super::Prompt::Device { user_code, .. } => user_code.clone(),
            super::super::Prompt::Browser { .. } => panic!("expected a device prompt"),
        };
        assert_eq!(code, "ABCD-1234");
        let saved = pending.finish().await.unwrap();
        let text = std::fs::read_to_string(&saved.path).unwrap();
        assert!(text.contains("\"access_token\": \"k-at\""));
        assert!(text.contains("\"type\": \"kimi\""));
        assert!(saved.file_name.starts_with("kimi-"));
    }

    fn test_opts(dir: &std::path::Path, origin: &str) -> super::super::Options {
        super::super::Options {
            auth_dir: dir.to_owned(),
            open_browser: false,
            http_origin: Some(origin.to_owned()),
            callback_timeout: std::time::Duration::from_secs(3),
            poll_floor: std::time::Duration::from_millis(15),
        }
    }
}
