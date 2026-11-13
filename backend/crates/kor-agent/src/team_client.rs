//! A member's side of the team server (§11.1): the client certificate (mTLS), the access token
//! and single sign-on.
//!
//! `kor server join <bundle>` imports what `kor server add-member` produced: the team CA, this
//! member's certificate and key, and a member token. All four go into the OS keychain
//! (`team:ca`, `team:client-cert`, `team:client-key`, `team:token`), never into a file. Every
//! request to the team server ([`builder`]) trusts the team CA and presents the certificate.
//! `kor server login` signs in through the team's identity provider with the OAuth device flow
//! (Keycloak, Authentik, Google, GitHub) and stores the session token the server hands back.

use std::time::Duration;

use kor_secrets::{ExposeSecret as _, SecretStore, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub const TOKEN: &str = "team:token";
pub const CA: &str = "team:ca";
pub const CERT: &str = "team:client-cert";
pub const KEY: &str = "team:client-key";

/// What `kor server add-member` writes for a new member.
#[derive(Clone, Deserialize)]
pub struct Bundle {
    pub server: String,
    pub email: String,
    pub role: String,
    pub ca: String,
    pub cert: String,
    pub key: String,
    pub token: String,
    #[serde(default)]
    pub expires_ms: i64,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("server", &self.server)
            .field("email", &self.email)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// The member's credentials, read from the store.
#[derive(Default)]
pub struct Material {
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<SecretString>,
    pub token: Option<SecretString>,
}

impl std::fmt::Debug for Material {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Material")
            .field("ca", &self.ca.is_some())
            .field("cert", &self.cert.is_some())
            .field("token", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

fn is_pem(s: &str, label: &str) -> bool {
    s.len() < kor_secrets::MAX_SECRET_LEN
        && s.contains(&format!("-----BEGIN {label}"))
        && s.contains("-----END ")
}

/// Check a bundle and store its credentials; the caller records the server address.
///
/// # Errors
/// Not a bundle, an `http://` server that is not this machine, or the store failed.
pub fn import(store: &dyn SecretStore, text: &str) -> Result<Bundle, String> {
    let b: Bundle = serde_json::from_str(text)
        .map_err(|_| "not a Kor team bundle (kor server add-member writes one)".to_owned())?;
    let u = reqwest::Url::parse(&b.server).map_err(|_| "the bundle has no valid server address")?;
    if u.scheme() != "https" {
        return Err("the team server must be https://".into());
    }
    if !is_pem(&b.ca, "CERTIFICATE")
        || !is_pem(&b.cert, "CERTIFICATE")
        || !is_pem(&b.key, "PRIVATE KEY")
    {
        return Err("the bundle's certificate or key is damaged".into());
    }
    // reqwest refuses a broken identity here rather than on the first request.
    reqwest::Identity::from_pem(format!("{}{}", b.cert, b.key).as_bytes())
        .map_err(|e| format!("the bundle's certificate or key is damaged: {e}"))?;
    if b.token.is_empty() || b.token.len() > 200 || b.token.chars().any(char::is_control) {
        return Err("the bundle's token is damaged".into());
    }
    let set = |k: &str, v: &str| {
        store
            .set(k, &SecretString::from(v.to_owned()))
            .map_err(|e| e.to_string())
    };
    set(CA, &b.ca)?;
    set(CERT, &b.cert)?;
    set(KEY, &b.key)?;
    set(TOKEN, &b.token)?;
    Ok(b)
}

/// Remove every team credential from the store.
///
/// # Errors
/// The store failed.
pub fn forget(store: &dyn SecretStore) -> Result<(), String> {
    for k in [CA, CERT, KEY, TOKEN] {
        store.delete(k).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// # Errors
/// The store failed.
pub fn material(store: &dyn SecretStore) -> Result<Material, String> {
    let get = |k: &str| store.get(k).map_err(|e| e.to_string());
    Ok(Material {
        ca: get(CA)?.map(|s| s.expose_secret().to_owned()),
        cert: get(CERT)?.map(|s| s.expose_secret().to_owned()),
        key: get(KEY)?,
        token: get(TOKEN)?,
    })
}

/// An HTTP client for the team server: trusts the team CA and presents the member certificate
/// when they are in the store (a server behind a public certificate needs neither).
///
/// # Errors
/// The store failed or the stored certificate is damaged.
pub fn builder_with(m: &Material, timeout: Duration) -> Result<reqwest::ClientBuilder, String> {
    let mut b = kor_core::net::client_builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(ca) = &m.ca {
        b = b.add_root_certificate(
            reqwest::Certificate::from_pem(ca.as_bytes()).map_err(|e| e.to_string())?,
        );
    }
    if let (Some(cert), Some(key)) = (&m.cert, &m.key) {
        let pem = format!("{cert}{}", key.expose_secret());
        b = b.identity(reqwest::Identity::from_pem(pem.as_bytes()).map_err(|e| e.to_string())?);
    }
    Ok(b)
}

/// [`builder_with`] from the OS keychain. Without a usable keychain the client has no
/// certificate, and a server that requires one refuses it (fails closed).
///
/// # Errors
/// As [`builder_with`].
pub fn builder(timeout: Duration) -> Result<reqwest::ClientBuilder, String> {
    builder_with(
        &material(&kor_secrets::KeychainStore).unwrap_or_default(),
        timeout,
    )
}

/// One sign-in method the server offers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub device_endpoint: String,
    #[serde(default)]
    pub token_endpoint: String,
}

/// The server's identity providers.
///
/// # Errors
/// The server cannot be reached.
pub async fn providers(client: &reqwest::Client, server: &str) -> Result<Vec<Provider>, String> {
    let v = get_json(client.get(format!(
        "{}/api/v1/auth/providers",
        server.trim_end_matches('/')
    )))
    .await?;
    serde_json::from_value(v["providers"].clone()).map_err(|e| e.to_string())
}

async fn get_json(r: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = r.send().await.map_err(|e| kor_core::net::describe(&e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["error"]
            .as_str()
            .or(v["error_description"].as_str())
            .unwrap_or(status.as_str());
        return Err(format!("{status}: {msg}"));
    }
    Ok(v)
}

fn form(pairs: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let enc = |s: &str| {
        s.bytes().fold(String::new(), |mut o, b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                o.push(b as char);
            } else {
                let _ = write!(o, "%{b:02X}");
            }
            o
        })
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// The first step of the device flow: what the user opens and types.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Device {
    pub device_code: String,
    pub user_code: String,
    #[serde(alias = "verification_url")]
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "five")]
    pub interval: u64,
    #[serde(default = "fifteen_min")]
    pub expires_in: u64,
}

fn five() -> u64 {
    5
}
fn fifteen_min() -> u64 {
    900
}

/// Start the device flow with the provider (OIDC scopes `openid email`; GitHub `read:user user:email`).
///
/// # Errors
/// The provider has no device endpoint or refused.
pub async fn device_start(client: &reqwest::Client, p: &Provider) -> Result<Device, String> {
    if p.device_endpoint.is_empty() {
        return Err(format!("{} offers no device sign-in", p.name));
    }
    let scope = if p.kind == "github" {
        "read:user user:email"
    } else {
        "openid email"
    };
    let v = get_json(
        client
            .post(&p.device_endpoint)
            .header("accept", "application/json")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form(&[("client_id", &p.client_id), ("scope", scope)])),
    )
    .await?;
    serde_json::from_value(v).map_err(|e| format!("device answer: {e}"))
}

/// One poll of the token endpoint: `Ok(None)` = not approved yet (wait `interval`), `Ok(Some)` =
/// the ID token (OIDC) or access token (GitHub).
///
/// # Errors
/// Denied, expired or a provider error.
pub async fn device_poll(
    client: &reqwest::Client,
    p: &Provider,
    d: &Device,
) -> Result<Option<String>, String> {
    let resp = client
        .post(&p.token_endpoint)
        .header("accept", "application/json")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &d.device_code),
            ("client_id", &p.client_id),
        ]))
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let v: Value =
        serde_json::from_str(&resp.text().await.map_err(|e| e.to_string())?).unwrap_or(Value::Null);
    match v["error"].as_str() {
        Some("authorization_pending" | "slow_down") => return Ok(None),
        Some(e) => return Err(v["error_description"].as_str().unwrap_or(e).to_owned()),
        None => {}
    }
    let field = if p.kind == "github" {
        "access_token"
    } else {
        "id_token"
    };
    v[field]
        .as_str()
        .map(|s| Some(s.to_owned()))
        .ok_or_else(|| format!("the provider sent no {field}"))
}

/// Trade the provider's token for a team session token (stored as `team:token`); the member's
/// e-mail and role.
///
/// # Errors
/// The server refused (not a member, invalid token).
pub async fn exchange(
    client: &reqwest::Client,
    store: &dyn SecretStore,
    server: &str,
    p: &Provider,
    token: &str,
) -> Result<(String, String), String> {
    let field = if p.kind == "github" {
        "access_token"
    } else {
        "id_token"
    };
    let v = get_json(
        client
            .post(format!(
                "{}/api/v1/auth/token",
                server.trim_end_matches('/')
            ))
            .header("content-type", "application/json")
            .body(json!({ "provider": p.name, field: token }).to_string()),
    )
    .await?;
    let session = v["token"].as_str().ok_or("the server sent no token")?;
    store
        .set(TOKEN, &SecretString::from(session.to_owned()))
        .map_err(|e| e.to_string())?;
    Ok((
        v["email"].as_str().unwrap_or_default().to_owned(),
        v["role"].as_str().unwrap_or_default().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Default)]
    struct Mem(std::sync::Mutex<std::collections::HashMap<String, String>>);
    impl SecretStore for Mem {
        fn get(&self, a: &str) -> Result<Option<SecretString>, kor_secrets::SecretError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(a)
                .map(|s| SecretString::from(s.clone())))
        }
        fn set(&self, a: &str, s: &SecretString) -> Result<(), kor_secrets::SecretError> {
            self.0
                .lock()
                .unwrap()
                .insert(a.into(), s.expose_secret().to_owned());
            Ok(())
        }
        fn delete(&self, a: &str) -> Result<bool, kor_secrets::SecretError> {
            Ok(self.0.lock().unwrap().remove(a).is_some())
        }
    }

    #[test]
    fn import_refuses_damaged_bundles() {
        let m = Mem::default();
        assert!(import(&m, "{}").is_err());
        let bad = json!({ "server": "http://kor.example.com", "email": "a@b.c", "role": "member",
            "ca": "x", "cert": "x", "key": "x", "token": "t" });
        assert!(import(&m, &bad.to_string()).unwrap_err().contains("https"));
        assert!(material(&m).unwrap().token.is_none());
    }

    #[tokio::test]
    async fn device_flow_then_exchange() {
        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dc", "user_code": "ABCD-EFGH", "verification_uri": "https://id.example.com/device", "interval": 1
            })))
            .mount(&idp)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("device_code=dc"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "id_token": "eyJ.x.y" })),
            )
            .mount(&idp)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/token"))
            .and(body_string_contains("eyJ.x.y"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "token": "kst_1", "email": "a@b.c", "role": "member" })),
            )
            .mount(&idp)
            .await;
        let p = Provider {
            name: "kc".into(),
            kind: "oidc".into(),
            client_id: "kor".into(),
            device_endpoint: format!("{}/device", idp.uri()),
            token_endpoint: format!("{}/token", idp.uri()),
        };
        let c = reqwest::Client::new();
        let d = device_start(&c, &p).await.unwrap();
        assert_eq!(d.user_code, "ABCD-EFGH");
        let tok = device_poll(&c, &p, &d).await.unwrap().unwrap();
        let m = Mem::default();
        let (email, role) = exchange(&c, &m, &idp.uri(), &p, &tok).await.unwrap();
        assert_eq!((email.as_str(), role.as_str()), ("a@b.c", "member"));
        assert_eq!(
            material(&m).unwrap().token.unwrap().expose_secret(),
            "kst_1"
        );
    }
}
