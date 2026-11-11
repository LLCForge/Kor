//! Claude Code OAuth (PKCE). Client id, scopes, and the token body match Claude Code 2.1.220:
//! authorize on `claude.ai`, exchange and refresh on `platform.claude.com`.

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    Error, Net, Provider, Record, err, form, hex_encode, json_i64, json_ok, rfc3339_after,
    rfc3339_now, rng, string_field,
};

pub(super) const PORT: u16 = 54545;
const AUTHORIZE: &str = "https://claude.ai/oauth/authorize";
const TOKEN: &str = "https://platform.claude.com/v1/oauth/token";
const PROFILE: &str = "https://api.anthropic.com/api/oauth/profile";
const ROLES: &str = "https://api.anthropic.com/api/oauth/claude_cli/roles";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const REDIRECT: &str = "http://localhost:54545/callback";
const REFRESH_SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const AUTHORIZE_SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const AXIOS: &[(&str, &str)] = &[
    ("accept", "application/json, text/plain, */*"),
    ("user-agent", "axios/1.15.2"),
    ("connection", "close"),
];

pub(super) fn authorize_url(challenge: &str, state: &str) -> String {
    let query = form(&[
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("scope", AUTHORIZE_SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ]);
    format!("{AUTHORIZE}?{query}")
}

pub(super) async fn exchange(
    net: &Net,
    code: &str,
    state: &str,
    verifier: &str,
) -> Result<Record, Error> {
    let (code, embedded_state) = split_code(code);
    let state = embedded_state.unwrap_or(state);
    let body = CodeExchange {
        grant_type: "authorization_code",
        code,
        redirect_uri: REDIRECT,
        client_id: CLIENT_ID,
        code_verifier: verifier,
        state,
    };
    let (status, text) = control_post(net, TOKEN, &body).await?;
    let token = json_ok(status, &text)?;
    account_record(&token, net).await
}

pub(super) async fn refresh(net: &Net, doc: &mut Value) -> Result<(), Error> {
    let refresh_token = string_field(doc, "refresh_token");
    if refresh_token.is_empty() {
        return Err(err("claude auth file has no refresh token"));
    }
    let body = RefreshBody {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token: &refresh_token,
        scope: REFRESH_SCOPE,
    };
    let (status, text) = control_post(net, TOKEN, &body).await?;
    let token = json_ok(status, &text)?;
    let access = string_field(&token, "access_token");
    if access.is_empty() {
        return Err(err("claude refresh returned no access token"));
    }
    let next_refresh = string_field(&token, "refresh_token");
    doc["access_token"] = json!(access);
    doc["refresh_token"] = json!(if next_refresh.is_empty() {
        refresh_token
    } else {
        next_refresh
    });
    doc["expired"] = json!(rfc3339_after(json_i64(&token, "expires_in")));
    doc["last_refresh"] = json!(rfc3339_now());
    if let Some(profile) = fetch_profile(net, &access).await {
        apply_profile(doc, &profile);
    }
    Ok(())
}

#[derive(Serialize)]
struct CodeExchange<'a> {
    grant_type: &'static str,
    code: &'a str,
    redirect_uri: &'static str,
    client_id: &'static str,
    code_verifier: &'a str,
    state: &'a str,
}

#[derive(Serialize)]
struct RefreshBody<'a> {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: &'a str,
    scope: &'static str,
}

async fn account_record(token: &Value, net: &Net) -> Result<Record, Error> {
    let access = string_field(token, "access_token");
    if access.is_empty() {
        return Err(err("claude token response has no access token"));
    }
    let refresh = string_field(token, "refresh_token");
    let mut email = pointer_str(token, "/account/email_address");
    let mut account = pointer_str(token, "/account/uuid");
    let mut organization = pointer_str(token, "/organization/uuid");
    let mut organization_name = pointer_str(token, "/organization/name");
    let _ = fetch_roles(net, &access).await;
    if let Some(profile) = fetch_profile(net, &access).await {
        if let Some(value) = profile.pointer("/account/uuid").and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            account = value.trim().to_owned();
        }
        if let Some(value) = profile.pointer("/account/email").and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            email = value.trim().to_owned();
        }
        if let Some(value) = profile
            .pointer("/organization/uuid")
            .and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            organization = value.trim().to_owned();
        }
        if let Some(value) = profile
            .pointer("/organization/name")
            .and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            organization_name = value.trim().to_owned();
        }
    }
    let device = hex_encode(&rng(32)?);
    let label = if email.is_empty() {
        "Claude".to_owned()
    } else {
        email.clone()
    };
    Ok(Record {
        provider: Provider::Claude,
        file_name: file_name(&email, &organization, &account),
        label,
        pretty: true,
        body: json!({
            "id_token": "",
            "access_token": access,
            "refresh_token": refresh,
            "last_refresh": rfc3339_now(),
            "email": email,
            "account_uuid": account,
            "organization_uuid": organization,
            "organization_name": organization_name,
            "claude_device_ids": [device],
            "type": "claude",
            "expired": rfc3339_after(json_i64(token, "expires_in")),
        }),
    })
}

fn apply_profile(doc: &mut Value, profile: &Value) {
    for (pointer, key) in [
        ("/account/email", "email"),
        ("/account/uuid", "account_uuid"),
        ("/organization/uuid", "organization_uuid"),
        ("/organization/name", "organization_name"),
    ] {
        if let Some(value) = profile.pointer(pointer).and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            doc[key] = json!(value.trim());
        }
    }
}

async fn control_post(net: &Net, url: &str, body: &impl Serialize) -> Result<(u16, String), Error> {
    let full = net.endpoint(url);
    if full.starts_with("https://") {
        super::fingerprint::post_json(&full, body, AXIOS).await
    } else {
        net.post_json(url, body, AXIOS).await
    }
}

async fn control_get(
    net: &Net,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<(u16, String), Error> {
    let full = net.endpoint(url);
    if full.starts_with("https://") {
        super::fingerprint::get(&full, headers).await
    } else {
        net.get(url, headers).await
    }
}

async fn fetch_profile(net: &Net, access: &str) -> Option<Value> {
    let authorization = format!("Bearer {access}");
    let headers = axios(&authorization);
    let (status, body) = control_get(net, PROFILE, &headers).await.ok()?;
    json_ok(status, &body).ok()
}

async fn fetch_roles(net: &Net, access: &str) -> Option<Value> {
    let authorization = format!("Bearer {access}");
    let headers = axios(&authorization);
    let (status, body) = control_get(net, ROLES, &headers).await.ok()?;
    json_ok(status, &body).ok()
}

fn axios(authorization: &str) -> [(&str, &str); 4] {
    [
        ("authorization", authorization),
        ("accept", "application/json, text/plain, */*"),
        ("user-agent", "axios/1.15.2"),
        ("connection", "close"),
    ]
}

fn pointer_str(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn split_code(code: &str) -> (&str, Option<&str>) {
    match code.split_once('#') {
        Some((head, tail)) if !tail.is_empty() => (head, Some(tail)),
        _ => (code, None),
    }
}

pub(super) fn file_name(email: &str, organization: &str, account: &str) -> String {
    let email = email.trim();
    let identity = if organization.trim().is_empty() {
        account.trim()
    } else {
        organization.trim()
    };
    if identity.is_empty() {
        format!("claude-{email}.json")
    } else {
        format!("claude-{}-{email}.json", super::hex_prefix8(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn file_name_hashes_the_organization_then_the_account() {
        let named = file_name("a@b.c", "org-1", "acc-1");
        assert!(named.starts_with("claude-"));
        assert!(named.ends_with("-a@b.c.json"));
        assert_ne!(named, file_name("a@b.c", "", "acc-1"));
        assert_eq!(file_name("a@b.c", "", ""), "claude-a@b.c.json");
    }

    #[tokio::test]
    async fn exchange_posts_the_native_body_and_saves_profile_identity() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/oauth/token"))
            .and(header("user-agent", "axios/1.15.2"))
            .and(body_string_contains("\"grant_type\":\"authorization_code\""))
            .and(body_string_contains("\"client_id\":\"9d1c250a-e61b-44d9-88ed-5944d1962f5e\""))
            .and(body_string_contains("\"code\":\"auth-code\""))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"account":{"uuid":"acc","email_address":"a@b.c"},"organization":{"uuid":"org","name":"Org"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/oauth/profile"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"account":{"uuid":"acc","email":"a@b.c"},"organization":{"uuid":"org","name":"Org"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/oauth/claude_cli/roles"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let net = net_at(&server.uri());
        let record = exchange(&net, "auth-code", "state", "verifier")
            .await
            .unwrap();
        assert_eq!(record.body["access_token"], "at");
        assert_eq!(record.body["email"], "a@b.c");
        assert_eq!(record.body["type"], "claude");
        assert!(record.file_name.starts_with("claude-"));
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
