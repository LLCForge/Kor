//! Codex OAuth. The callback flow and the device-code flow both exchange at
//! `auth.openai.com` with the Codex CLI client id. Device approval returns an authorization
//! code plus the PKCE verifier the token endpoint expects.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    Error, Net, Provider, Record, b64_url, err, form, hex_prefix8, json_i64, json_ok,
    rfc3339_after, rfc3339_now, string_field,
};

pub(super) const PORT: u16 = 1455;
pub(super) const REDIRECT: &str = "http://localhost:1455/auth/callback";
pub(super) const DEVICE_VERIFY: &str = "https://auth.openai.com/codex/device";
const AUTHORIZE: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_USERCODE: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_REDIRECT: &str = "https://auth.openai.com/deviceauth/callback";
const PLAN_FREE: &str = "free";

pub(super) struct DeviceStart {
    pub(super) device_auth_id: String,
    pub(super) user_code: String,
    pub(super) interval: std::time::Duration,
}

pub(super) fn authorize_url(challenge: &str, state: &str) -> String {
    let query = form(&[
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("scope", "openid email profile offline_access"),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("prompt", "login"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
    ]);
    format!("{AUTHORIZE}?{query}")
}

pub(super) async fn exchange(
    net: &Net,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<Record, Error> {
    let form_body = form(&[
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
    ]);
    let (status, text) = net.post_form(TOKEN, &form_body, &[]).await?;
    let token = json_ok(status, &text)?;
    record_from_token(&token)
}

pub(super) async fn start_device(net: &Net) -> Result<DeviceStart, Error> {
    let (status, text) = net
        .post_json(DEVICE_USERCODE, &json!({ "client_id": CLIENT_ID }), &[])
        .await?;
    let body = json_ok(status, &text)?;
    let user_code = {
        let primary = string_field(&body, "user_code");
        if primary.is_empty() {
            string_field(&body, "usercode")
        } else {
            primary
        }
    };
    let device_auth_id = string_field(&body, "device_auth_id");
    if user_code.is_empty() || device_auth_id.is_empty() {
        return Err(err("codex device flow did not return a user code"));
    }
    let interval = match body.get("interval") {
        Some(value) => super::json_i64(&json!({ "interval": value }), "interval"),
        None => 0,
    };
    let interval = if interval > 0 {
        std::time::Duration::from_secs(u64::try_from(interval).unwrap_or(5))
    } else {
        net.poll_floor
    };
    Ok(DeviceStart {
        device_auth_id,
        user_code,
        interval,
    })
}

pub(super) async fn finish_device(
    net: &Net,
    device_auth_id: &str,
    user_code: &str,
    mut interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<Record, Error> {
    if interval.is_zero() {
        interval = net.poll_floor;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(err("codex device authentication timed out"));
        }
        let (status, text) = net
            .post_json(
                DEVICE_TOKEN,
                &json!({ "device_auth_id": device_auth_id, "user_code": user_code }),
                &[],
            )
            .await?;
        if (200..300).contains(&status) {
            let body = json_ok(status, &text)?;
            let code = string_field(&body, "authorization_code");
            let verifier = string_field(&body, "code_verifier");
            if code.is_empty() || verifier.is_empty() {
                return Err(err(
                    "codex device flow token response is missing the authorization code",
                ));
            }
            return exchange(net, &code, &verifier, DEVICE_REDIRECT).await;
        }
        if status == 403 || status == 404 {
            tokio::time::sleep(interval).await;
            continue;
        }
        return Err(super::http_error(status, &text));
    }
}

#[allow(clippy::similar_names)] // paired names (in/out, old/new) read best side by side
pub(super) async fn refresh(net: &Net, doc: &mut Value) -> Result<(), Error> {
    let refresh_token = string_field(doc, "refresh_token");
    if refresh_token.is_empty() {
        return Err(err("codex auth file has no refresh token"));
    }
    let form_body = form(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
        ("scope", "openid profile email"),
    ]);
    let (status, text) = net.post_form(TOKEN, &form_body, &[]).await?;
    let token = json_ok(status, &text)?;
    let access = string_field(&token, "access_token");
    let next = string_field(&token, "refresh_token");
    if access.is_empty() || next.is_empty() || json_i64(&token, "expires_in") <= 0 {
        return Err(err("codex refresh response is missing tokens"));
    }
    let claims = claims_of(&string_field(&token, "id_token"));
    doc["access_token"] = json!(access);
    doc["refresh_token"] = json!(next);
    doc["id_token"] = json!(string_field(&token, "id_token"));
    doc["expired"] = json!(rfc3339_after(json_i64(&token, "expires_in")));
    doc["last_refresh"] = json!(rfc3339_now());
    if !claims.email.is_empty() {
        doc["email"] = json!(claims.email);
    }
    if !claims.account_id.is_empty() {
        doc["account_id"] = json!(claims.account_id);
    }
    doc["plan_type"] = json!(claims.plan);
    Ok(())
}

fn record_from_token(token: &Value) -> Result<Record, Error> {
    let access = string_field(token, "access_token");
    let refresh = string_field(token, "refresh_token");
    let id_token = string_field(token, "id_token");
    if access.is_empty() {
        return Err(err("codex token response has no access token"));
    }
    let claims = claims_of(&id_token);
    if claims.email.is_empty() {
        return Err(err("codex token is missing the account email"));
    }
    let hash = if claims.account_id.is_empty() {
        String::new()
    } else {
        hex_prefix8(&claims.account_id)
    };
    Ok(Record {
        provider: Provider::Codex,
        file_name: file_name(&claims.email, &claims.plan, &hash),
        label: claims.email.clone(),
        pretty: true,
        body: json!({
            "id_token": id_token,
            "access_token": access,
            "refresh_token": refresh,
            "account_id": claims.account_id,
            "last_refresh": rfc3339_now(),
            "email": claims.email,
            "type": "codex",
            "expired": rfc3339_after(json_i64(token, "expires_in")),
            "plan_type": claims.plan,
        }),
    })
}

struct Identity {
    email: String,
    account_id: String,
    plan: String,
}

fn claims_of(id_token: &str) -> Identity {
    let Some(payload) = id_token.split('.').nth(1) else {
        return Identity {
            email: String::new(),
            account_id: String::new(),
            plan: PLAN_FREE.to_owned(),
        };
    };
    let Ok(bytes) = b64_url(payload) else {
        return Identity {
            email: String::new(),
            account_id: String::new(),
            plan: PLAN_FREE.to_owned(),
        };
    };
    let Ok(claims) = serde_json::from_slice::<Claims>(&bytes) else {
        return Identity {
            email: String::new(),
            account_id: String::new(),
            plan: PLAN_FREE.to_owned(),
        };
    };
    let plan = claims
        .auth
        .as_ref()
        .and_then(|auth| auth.chatgpt_plan_type.clone())
        .unwrap_or_default();
    Identity {
        email: claims.email.unwrap_or_default().trim().to_owned(),
        account_id: claims
            .auth
            .as_ref()
            .and_then(|auth| auth.chatgpt_account_id.clone())
            .unwrap_or_default(),
        plan: if plan.trim().is_empty() {
            PLAN_FREE.to_owned()
        } else {
            plan.trim().to_owned()
        },
    }
}

#[derive(Deserialize)]
struct Claims {
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaim>,
}

#[derive(Deserialize)]
struct AuthClaim {
    chatgpt_account_id: Option<String>,
    chatgpt_plan_type: Option<String>,
}

pub(super) fn file_name(email: &str, plan: &str, account_hash: &str) -> String {
    let email = email.trim();
    let plan = normalize_plan(plan);
    let account_hash = account_hash.trim();
    match (account_hash.is_empty(), plan.is_empty()) {
        (false, false) => format!("codex-{account_hash}-{email}-{plan}.json"),
        (false, true) => format!("codex-{account_hash}-{email}.json"),
        (true, false) => format!("codex-{email}-{plan}.json"),
        (true, true) => format!("codex-{email}.json"),
    }
}

fn normalize_plan(plan: &str) -> String {
    plan.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn authorize_url_uses_the_codex_client_and_callback() {
        let url = authorize_url("challenge", "state");
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn file_name_includes_plan_and_account_hash() {
        assert_eq!(
            file_name("a@b.c", "Plus", "abcd1234"),
            "codex-abcd1234-a@b.c-plus.json"
        );
        assert_eq!(file_name("a@b.c", "", ""), "codex-a@b.c.json");
    }

    #[tokio::test]
    async fn exchange_reads_the_plan_from_the_id_token() {
        let server = MockServer::start().await;
        let id_token = fake_jwt(
            r#"{"email":"a@b.c","https://api.openai.com/auth":{"chatgpt_account_id":"acc-1","chatgpt_plan_type":"pro"}}"#,
        );
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"))
            .and(body_string_contains("code=the-code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"access_token":"at","refresh_token":"rt","id_token":"{id_token}","expires_in":3600}}"#
            )))
            .mount(&server)
            .await;
        let record = exchange(&net_at(&server.uri()), "the-code", "verifier", REDIRECT)
            .await
            .unwrap();
        assert_eq!(record.body["email"], "a@b.c");
        assert_eq!(record.body["plan_type"], "pro");
        assert_eq!(record.body["account_id"], "acc-1");
        assert!(record.file_name.starts_with("codex-"));
        assert!(record.file_name.contains(&hex_prefix8("acc-1")));
    }

    fn fake_jwt(payload: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.as_bytes()))
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
