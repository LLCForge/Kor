//! Antigravity OAuth. The Google client id, secret, scopes, and Cloud Code project discovery
//! (`loadCodeAssist`, then `onboardUser`) match the official Antigravity client.

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    Error, Net, Provider, Record, err, form, json_i64, json_ok, rfc3339_after, string_field,
    unix_millis,
};

pub(super) const PORT: u16 = 51121;
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const USERINFO: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
const LOAD: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const ONBOARD: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal:onboardUser";
const VERSION: &str = "2.29.0";
const NODE_UA: &str = "google-api-nodejs-client/10.3.0";
const GOOG_API_CLIENT: &str = "gl-node/22.21.1";

const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/aicode",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];

pub(super) fn authorize_url(state: &str, redirect: &str) -> String {
    let query = form(&[
        ("access_type", "offline"),
        ("client_id", CLIENT_ID),
        ("prompt", "consent"),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", &SCOPES.join(" ")),
        ("state", state),
    ]);
    format!("{AUTH}?{query}")
}

pub(super) async fn exchange(net: &Net, code: &str, redirect: &str) -> Result<Record, Error> {
    let form_body = form(&[
        ("code", code),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("redirect_uri", redirect),
        ("grant_type", "authorization_code"),
    ]);
    let (status, text) = net.post_form(TOKEN, &form_body, &[]).await?;
    let token = json_ok(status, &text)?;
    let access = string_field(&token, "access_token");
    if access.is_empty() {
        return Err(err("antigravity token response has no access token"));
    }
    let email = user_email(net, &access).await?;
    let project = project_id(net, &access).await?;
    Ok(stored(
        access,
        string_field(&token, "refresh_token"),
        json_i64(&token, "expires_in"),
        email,
        project,
    ))
}

#[allow(clippy::similar_names)] // paired names (in/out, old/new) read best side by side
pub(super) async fn refresh(net: &Net, doc: &mut Value) -> Result<(), Error> {
    let refresh_token = string_field(doc, "refresh_token");
    if refresh_token.is_empty() {
        return Err(err("antigravity auth file has no refresh token"));
    }
    let form_body = form(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
    ]);
    let (status, text) = net
        .post_form(TOKEN, &form_body, &[("user-agent", "Go-http-client/2.0")])
        .await?;
    let token = json_ok(status, &text)?;
    let access = string_field(&token, "access_token");
    if access.is_empty() {
        return Err(err("antigravity refresh returned no access token"));
    }
    let expires_in = json_i64(&token, "expires_in");
    doc["access_token"] = json!(access);
    let next = string_field(&token, "refresh_token");
    if !next.is_empty() {
        doc["refresh_token"] = json!(next);
    }
    doc["expires_in"] = json!(expires_in);
    doc["timestamp"] = json!(unix_millis());
    doc["expired"] = json!(rfc3339_after(expires_in));
    doc["type"] = json!("antigravity");
    Ok(())
}

fn stored(
    access: String,
    refresh: String,
    expires_in: i64,
    email: String,
    project: String,
) -> Record {
    Record {
        provider: Provider::Antigravity,
        file_name: if email.is_empty() {
            "antigravity.json".to_owned()
        } else {
            format!("antigravity-{email}.json")
        },
        label: if email.is_empty() {
            "antigravity".to_owned()
        } else {
            email.clone()
        },
        pretty: true,
        body: json!({
            "type": "antigravity",
            "access_token": access,
            "refresh_token": refresh,
            "expires_in": expires_in,
            "timestamp": unix_millis(),
            "expired": rfc3339_after(expires_in),
            "email": email,
            "project_id": project,
        }),
    }
}

fn user_agent() -> String {
    format!("antigravity/hub/{VERSION} (aidev_client; os_type=darwin; arch=arm64; cl=1013060670)")
}

async fn user_email(net: &Net, access: &str) -> Result<String, Error> {
    let ua = user_agent();
    let authorization = format!("Bearer {access}");
    let (status, text) = net
        .get(
            USERINFO,
            &[("authorization", &authorization), ("user-agent", &ua)],
        )
        .await?;
    let body = json_ok(status, &text)?;
    let email = string_field(&body, "email");
    if email.is_empty() {
        return Err(err("antigravity user info did not include an email"));
    }
    Ok(email)
}

async fn project_id(net: &Net, access: &str) -> Result<String, Error> {
    let ua = user_agent();
    let authorization = format!("Bearer {access}");
    let (status, text) = net
        .post_json(
            LOAD,
            &LoadBody {
                metadata: LoadMeta {
                    ide_type: "ANTIGRAVITY",
                },
            },
            &[("authorization", &authorization), ("user-agent", &ua)],
        )
        .await?;
    let body = json_ok(status, &text)?;
    if let Some(project) = project_of(&body) {
        return Ok(project);
    }
    onboard(net, access, &tier_of(&body)).await
}

async fn onboard(net: &Net, access: &str, tier: &str) -> Result<String, Error> {
    let ua = format!("{} {NODE_UA}", user_agent());
    let authorization = format!("Bearer {access}");
    let body = Onboard {
        tier_id: tier,
        metadata: OnboardMeta {
            ide_type: "ANTIGRAVITY",
            ide_version: VERSION,
            ide_name: "antigravity",
        },
    };
    for attempt in 0..5 {
        let (status, text) = net
            .post_json(
                ONBOARD,
                &body,
                &[
                    ("authorization", &authorization),
                    ("user-agent", &ua),
                    ("x-goog-api-client", GOOG_API_CLIENT),
                ],
            )
            .await?;
        let parsed = json_ok(status, &text)?;
        if parsed.get("done").and_then(Value::as_bool) == Some(true) {
            let project = parsed
                .get("response")
                .and_then(project_of)
                .unwrap_or_default();
            if project.is_empty() {
                return Err(err("antigravity onboard response has no project id"));
            }
            return Ok(project);
        }
        if attempt < 4 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    Err(err("antigravity onboard did not finish"))
}

fn project_of(value: &Value) -> Option<String> {
    for key in ["cloudaicompanionProject", "projectId", "project"] {
        match value.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => {
                return Some(text.trim().to_owned());
            }
            Some(Value::Object(map)) => {
                if let Some(id) = map.get("id").and_then(Value::as_str)
                    && !id.trim().is_empty()
                {
                    return Some(id.trim().to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

fn tier_of(value: &Value) -> String {
    if let Some(tiers) = value.get("allowedTiers").and_then(Value::as_array) {
        for tier in tiers {
            if tier.get("isDefault").and_then(Value::as_bool) == Some(true)
                && let Some(id) = tier.get("id").and_then(Value::as_str)
                && !id.trim().is_empty()
            {
                return id.trim().to_owned();
            }
        }
    }
    value
        .pointer("/currentTier/id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map_or_else(|| "free-tier".to_owned(), |id| id.trim().to_owned())
}

#[derive(Serialize)]
struct LoadBody {
    metadata: LoadMeta,
}

#[derive(Serialize)]
struct LoadMeta {
    #[serde(rename = "ideType")]
    ide_type: &'static str,
}

#[derive(Serialize)]
struct Onboard<'a> {
    tier_id: &'a str,
    metadata: OnboardMeta,
}

#[derive(Serialize)]
struct OnboardMeta {
    ide_type: &'static str,
    ide_version: &'static str,
    ide_name: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn authorize_url_carries_the_aicode_scope() {
        let url = authorize_url("state", "http://localhost:51121/oauth-callback");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains(
            "client_id=1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
        ));
        assert!(url.contains("auth%2Faicode"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
    }

    #[tokio::test]
    async fn exchange_stores_email_and_project() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("client_id=1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"Bearer"}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/oauth2/v2/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"email":"a@b.c"}"#))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .and(body_string_contains("ANTIGRAVITY"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"cloudaicompanionProject":"proj-1"}"#),
            )
            .mount(&server)
            .await;
        let record = exchange(
            &net_at(&server.uri()),
            "code",
            "http://localhost:51121/oauth-callback",
        )
        .await
        .unwrap();
        assert_eq!(record.body["email"], "a@b.c");
        assert_eq!(record.body["project_id"], "proj-1");
        assert_eq!(record.file_name, "antigravity-a@b.c.json");
        assert_eq!(record.body["type"], "antigravity");
    }

    fn net_at(origin: &str) -> Net {
        Net {
            client: super::super::http_client().unwrap(),
            origin: Some(origin.to_owned()),
            poll_floor: std::time::Duration::from_millis(15),
            timeout: std::time::Duration::from_secs(3),
        }
    }
}
