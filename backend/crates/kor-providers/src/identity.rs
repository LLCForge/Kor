//! §4.2: cloud identities instead of API keys. When a provider's key is not set, the request is
//! authorised just before it is sent:
//! - Google Vertex AI: Application Default Credentials (`GOOGLE_APPLICATION_CREDENTIALS`, else
//!   gcloud's `application_default_credentials.json`): an `authorized_user` refresh token, or a
//!   `service_account` key signing a JWT (RS256). The access token is cached until it expires.
//! - Azure OpenAI: Microsoft Entra ID client credentials (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
//!   `AZURE_CLIENT_SECRET`), scope `https://cognitiveservices.azure.com/.default`.
//! - OCI Generative AI: OCI request signing (HTTP signatures, rsa-sha256) with the API key of
//!   `~/.oci/config` (`OCI_CLI_PROFILE`, default `DEFAULT`).
//!
//! Secrets stay in this process; only tokens and signatures go to the provider and the identity
//! endpoints, through the same network guard as every request.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use kor_secrets::{ExposeSecret as _, SecretString};
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::ProviderManifest;

/// Adds the provider's credentials to a finished request.
#[async_trait]
pub trait Authorizer: Send + Sync + std::fmt::Debug {
    async fn authorize(&self, req: &mut reqwest::Request) -> Result<(), String>;
}

/// The identity a manifest can use when its key is missing, if one is configured here.
pub fn for_manifest(m: &ProviderManifest) -> Option<Arc<dyn Authorizer>> {
    match m.id.as_str() {
        "vertex" => GoogleAdc::load().map(|a| Arc::new(a) as Arc<dyn Authorizer>),
        "azure" => Entra::from_env().map(|a| Arc::new(a) as Arc<dyn Authorizer>),
        "oci" => OciSigner::load().map(|a| Arc::new(a) as Arc<dyn Authorizer>),
        _ => None,
    }
}

/// Vertex's OpenAI-compatible endpoint from `GOOGLE_CLOUD_PROJECT` (+ `GOOGLE_CLOUD_LOCATION`,
/// default `global`) when its URL variable is unset; `None` when nothing changes.
pub fn regional(m: &ProviderManifest) -> Option<ProviderManifest> {
    if m.id != "vertex" || !m.base_url.contains(".invalid") {
        return None;
    }
    if m.base_url_env
        .as_deref()
        .is_some_and(|v| std::env::var(v).is_ok_and(|s| !s.is_empty()))
    {
        return None;
    }
    let project = std::env::var("GOOGLE_CLOUD_PROJECT")
        .ok()
        .filter(|p| !p.trim().is_empty())?;
    let location = std::env::var("GOOGLE_CLOUD_LOCATION").unwrap_or_else(|_| "global".into());
    let host = if location == "global" {
        "aiplatform.googleapis.com".to_owned()
    } else {
        format!("{location}-aiplatform.googleapis.com")
    };
    let mut out = m.clone();
    out.base_url =
        format!("https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi");
    out.base_url_env = None;
    Some(out)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn http() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

fn set_bearer(req: &mut reqwest::Request, token: &str) -> Result<(), String> {
    let mut v = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| e.to_string())?;
    v.set_sensitive(true);
    req.headers_mut().insert(reqwest::header::AUTHORIZATION, v);
    Ok(())
}

/// An OAuth token endpoint's answer: the access token and when to refresh it (a minute early).
async fn token_request(url: &str, form: &[(&str, &str)]) -> Result<(SecretString, u64), String> {
    let body = form
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                crate::sigv4::encode_segment(k),
                crate::sigv4::encode_segment(v)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let resp = http()?
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("rejected");
        return Err(format!("token request failed ({status}): {why}"));
    }
    let token = v["access_token"]
        .as_str()
        .ok_or("no access_token in the answer")?;
    let ttl = v["expires_in"].as_u64().unwrap_or(3600);
    Ok((
        SecretString::from(token.to_owned()),
        now() + ttl.saturating_sub(60),
    ))
}

/// A cached access token, refreshed by `fetch` when it is about to expire.
#[derive(Debug, Default)]
struct Cache(tokio::sync::Mutex<Option<(SecretString, u64)>>);

impl Cache {
    async fn get<F, Fut>(&self, fetch: F) -> Result<String, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(SecretString, u64), String>>,
    {
        let mut g = self.0.lock().await;
        if let Some((t, until)) = g.as_ref()
            && *until > now()
        {
            return Ok(t.expose_secret().to_owned());
        }
        let (t, until) = fetch().await?;
        let out = t.expose_secret().to_owned();
        *g = Some((t, until));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// RSA (ring)
// ---------------------------------------------------------------------------------------------

/// An RSA private key from PEM: PKCS#8 (`BEGIN PRIVATE KEY`) or PKCS#1 (`BEGIN RSA PRIVATE KEY`).
/// Encrypted keys are refused with a message.
fn rsa_key(pem: &str) -> Result<ring::signature::RsaKeyPair, String> {
    if pem.contains("ENCRYPTED") {
        return Err("the private key is encrypted; use an unencrypted key file".into());
    }
    let pkcs1 = pem.contains("BEGIN RSA PRIVATE KEY");
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----") && !l.contains(':'))
        .collect::<Vec<_>>()
        .concat();
    let der = STANDARD
        .decode(body.trim())
        .map_err(|e| format!("key is not PEM: {e}"))?;
    let key = if pkcs1 {
        ring::signature::RsaKeyPair::from_der(&der)
    } else {
        ring::signature::RsaKeyPair::from_pkcs8(&der)
    };
    key.map_err(|e| format!("unreadable RSA key: {e}"))
}

fn rsa_sha256(key: &ring::signature::RsaKeyPair, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        data,
        &mut sig,
    )
    .map_err(|_| "signing failed".to_owned())?;
    Ok(sig)
}

// ---------------------------------------------------------------------------------------------
// Google Application Default Credentials
// ---------------------------------------------------------------------------------------------

const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug)]
enum GoogleCred {
    User {
        client_id: String,
        client_secret: SecretString,
        refresh_token: SecretString,
    },
    Service {
        email: String,
        key_pem: SecretString,
        token_uri: String,
    },
}

#[derive(Debug)]
pub struct GoogleAdc {
    cred: GoogleCred,
    cache: Cache,
}

impl GoogleAdc {
    fn adc_path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS") {
            return Some(PathBuf::from(p));
        }
        // gcloud: %APPDATA%\gcloud on Windows, ~/.config/gcloud elsewhere.
        let base = std::env::var_os("CLOUDSDK_CONFIG")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA")
                    .map(|a| PathBuf::from(a).join("gcloud"))
                    .filter(|p| p.is_dir())
                    .or_else(|| dirs::home_dir().map(|h| h.join(".config").join("gcloud")))
            })?;
        Some(base.join("application_default_credentials.json"))
    }

    fn load() -> Option<Self> {
        Self::from_file(&Self::adc_path()?).ok()
    }

    fn from_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&serde_json::from_str(&text).map_err(|e| e.to_string())?)
    }

    fn from_json(v: &Value) -> Result<Self, String> {
        let s = |k: &str| v[k].as_str().filter(|x| !x.is_empty()).map(str::to_owned);
        let cred = match v["type"].as_str() {
            Some("authorized_user") => GoogleCred::User {
                client_id: s("client_id").ok_or("no client_id")?,
                client_secret: SecretString::from(s("client_secret").ok_or("no client_secret")?),
                refresh_token: SecretString::from(s("refresh_token").ok_or("no refresh_token")?),
            },
            Some("service_account") => GoogleCred::Service {
                email: s("client_email").ok_or("no client_email")?,
                key_pem: SecretString::from(s("private_key").ok_or("no private_key")?),
                token_uri: s("token_uri").unwrap_or_else(|| GOOGLE_TOKEN_URI.into()),
            },
            other => return Err(format!("credentials of type {other:?} are not supported")),
        };
        Ok(Self {
            cred,
            cache: Cache::default(),
        })
    }

    /// The signed JWT a service account trades for an access token.
    fn assertion(email: &str, key_pem: &str, token_uri: &str, iat: u64) -> Result<String, String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            json!({"iss": email, "scope": GOOGLE_SCOPE, "aud": token_uri, "iat": iat, "exp": iat + 3600})
                .to_string(),
        );
        let input = format!("{header}.{claims}");
        let sig = rsa_sha256(&rsa_key(key_pem)?, input.as_bytes())?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig)))
    }

    async fn fetch(&self) -> Result<(SecretString, u64), String> {
        match &self.cred {
            GoogleCred::User {
                client_id,
                client_secret,
                refresh_token,
            } => {
                token_request(
                    GOOGLE_TOKEN_URI,
                    &[
                        ("grant_type", "refresh_token"),
                        ("client_id", client_id),
                        ("client_secret", client_secret.expose_secret()),
                        ("refresh_token", refresh_token.expose_secret()),
                    ],
                )
                .await
            }
            GoogleCred::Service {
                email,
                key_pem,
                token_uri,
            } => {
                let jwt = Self::assertion(email, key_pem.expose_secret(), token_uri, now())?;
                token_request(
                    token_uri,
                    &[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                        ("assertion", &jwt),
                    ],
                )
                .await
            }
        }
    }
}

#[async_trait]
impl Authorizer for GoogleAdc {
    async fn authorize(&self, req: &mut reqwest::Request) -> Result<(), String> {
        let t = self.cache.get(|| self.fetch()).await?;
        set_bearer(req, &t)
    }
}

// ---------------------------------------------------------------------------------------------
// Microsoft Entra ID (client credentials)
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Entra {
    authority: String,
    tenant: String,
    client_id: String,
    secret: SecretString,
    cache: Cache,
}

impl Entra {
    fn from_env() -> Option<Self> {
        let v = |k: &str| std::env::var(k).ok().filter(|x| !x.trim().is_empty());
        Some(Self {
            authority: v("AZURE_AUTHORITY_HOST")
                .unwrap_or_else(|| "https://login.microsoftonline.com".into()),
            tenant: v("AZURE_TENANT_ID")?,
            client_id: v("AZURE_CLIENT_ID")?,
            secret: SecretString::from(v("AZURE_CLIENT_SECRET")?),
            cache: Cache::default(),
        })
    }

    async fn fetch(&self) -> Result<(SecretString, u64), String> {
        let url = format!(
            "{}/{}/oauth2/v2.0/token",
            self.authority.trim_end_matches('/'),
            crate::sigv4::encode_segment(&self.tenant)
        );
        token_request(
            &url,
            &[
                ("grant_type", "client_credentials"),
                ("client_id", &self.client_id),
                ("client_secret", self.secret.expose_secret()),
                ("scope", "https://cognitiveservices.azure.com/.default"),
            ],
        )
        .await
    }
}

#[async_trait]
impl Authorizer for Entra {
    async fn authorize(&self, req: &mut reqwest::Request) -> Result<(), String> {
        let t = self.cache.get(|| self.fetch()).await?;
        set_bearer(req, &t)
    }
}

// ---------------------------------------------------------------------------------------------
// OCI request signing
// ---------------------------------------------------------------------------------------------

pub struct OciSigner {
    key_id: String,
    key: ring::signature::RsaKeyPair,
}

impl std::fmt::Debug for OciSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OciSigner")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl OciSigner {
    fn load() -> Option<Self> {
        let path = std::env::var_os("OCI_CLI_CONFIG_FILE")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".oci").join("config")))?;
        let text = std::fs::read_to_string(&path).ok()?;
        let profile = std::env::var("OCI_CLI_PROFILE").unwrap_or_else(|_| "DEFAULT".into());
        Self::from_config(&text, &profile).ok()
    }

    fn from_config(text: &str, profile: &str) -> Result<Self, String> {
        let mut section = false;
        let mut get = std::collections::BTreeMap::new();
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim() == profile;
            } else if section && let Some((k, v)) = line.split_once('=') {
                get.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
        let need = |k: &str| {
            get.get(k)
                .cloned()
                .ok_or_else(|| format!("~/.oci/config [{profile}] has no {k}"))
        };
        let key_file = need("key_file")?;
        let key_file = match key_file
            .strip_prefix("~/")
            .or_else(|| key_file.strip_prefix("~\\"))
        {
            Some(rest) => {
                dirs::home_dir().map_or_else(|| PathBuf::from(&key_file), |h| h.join(rest))
            }
            None => PathBuf::from(&key_file),
        };
        let pem = std::fs::read_to_string(&key_file)
            .map_err(|e| format!("{}: {e}", key_file.display()))?;
        Self::new(
            &need("tenancy")?,
            &need("user")?,
            &need("fingerprint")?,
            &pem,
        )
    }

    fn new(tenancy: &str, user: &str, fingerprint: &str, pem: &str) -> Result<Self, String> {
        Ok(Self {
            key_id: format!("{tenancy}/{user}/{fingerprint}"),
            key: rsa_key(pem)?,
        })
    }

    /// The signed headers and the `authorization` value for one request (date given for tests).
    fn sign(
        &self,
        method: &str,
        host: &str,
        path_and_query: &str,
        body: &[u8],
        date: &str,
        content_type: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let method = method.to_lowercase();
        let mut headers = vec![("date".to_owned(), date.to_owned())];
        let mut names = vec!["(request-target)", "host", "date"];
        let mut lines =
            format!("(request-target): {method} {path_and_query}\nhost: {host}\ndate: {date}");
        if matches!(method.as_str(), "post" | "put" | "patch") {
            let sha = STANDARD.encode(Sha256::digest(body));
            let _ = write!(
                lines,
                "\nx-content-sha256: {sha}\ncontent-type: {content_type}\ncontent-length: {}",
                body.len()
            );
            headers.push(("x-content-sha256".into(), sha));
            headers.push(("content-length".into(), body.len().to_string()));
            names.extend(["x-content-sha256", "content-type", "content-length"]);
        }
        let sig = STANDARD.encode(rsa_sha256(&self.key, lines.as_bytes())?);
        headers.push((
            "authorization".into(),
            format!(
                "Signature version=\"1\",keyId=\"{}\",algorithm=\"rsa-sha256\",headers=\"{}\",signature=\"{sig}\"",
                self.key_id,
                names.join(" ")
            ),
        ));
        Ok(headers)
    }
}

#[async_trait]
impl Authorizer for OciSigner {
    async fn authorize(&self, req: &mut reqwest::Request) -> Result<(), String> {
        let url = req.url().clone();
        let host = url.host_str().ok_or("no host")?.to_owned();
        let host = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host,
        };
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        let body = req
            .body()
            .and_then(reqwest::Body::as_bytes)
            .unwrap_or_default()
            .to_vec();
        let content_type = req
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_owned();
        let date = chrono::Utc::now()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        for (k, v) in self.sign(
            req.method().as_str(),
            &host,
            &path,
            &body,
            &date,
            &content_type,
        )? {
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| e.to_string())?;
            let mut value = HeaderValue::from_str(&v).map_err(|e| e.to_string())?;
            value.set_sensitive(k == "authorization");
            req.headers_mut().insert(name, value);
        }
        if !req.headers().contains_key(reqwest::header::CONTENT_TYPE) {
            req.headers_mut().insert(
                reqwest::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/identity")
                .join(name),
        )
        .unwrap()
    }

    fn verify(pem: &str, data: &[u8], sig: &[u8]) -> bool {
        let key = rsa_key(pem).unwrap();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public().as_ref())
            .verify(data, sig)
            .is_ok()
    }

    #[test]
    fn both_key_formats_sign_and_verify() {
        for name in ["test-only-rsa-pkcs8.pem", "test-only-rsa-pkcs1.pem"] {
            let pem = fixture(name);
            let sig = rsa_sha256(&rsa_key(&pem).unwrap(), b"hello").unwrap();
            assert!(verify(&pem, b"hello", &sig), "{name}");
        }
        assert!(
            rsa_key(
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----"
            )
            .is_err()
        );
    }

    #[test]
    fn service_account_jwt_is_rs256_with_the_right_claims() {
        let pem = fixture("test-only-rsa-pkcs8.pem");
        let jwt = GoogleAdc::assertion(
            "sa@p.iam.gserviceaccount.com",
            &pem,
            GOOGLE_TOKEN_URI,
            1_700_000_000,
        )
        .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["scope"], GOOGLE_SCOPE);
        assert_eq!(claims["exp"], 1_700_003_600);
        let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert!(verify(
            &pem,
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &sig
        ));
    }

    #[test]
    fn oci_signature_covers_the_body_and_verifies() {
        let pem = fixture("test-only-rsa-pkcs1.pem");
        let s = OciSigner::new("ocid1.tenancy.oc1..t", "ocid1.user.oc1..u", "aa:bb", &pem).unwrap();
        let body = br#"{"model":"m"}"#;
        let date = "Thu, 05 Jan 2014 21:31:40 GMT";
        let h = s
            .sign(
                "POST",
                "inference.x.oci.oraclecloud.com",
                "/openai/v1/chat/completions",
                body,
                date,
                "application/json",
            )
            .unwrap();
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(
            auth.contains(r#"keyId="ocid1.tenancy.oc1..t/ocid1.user.oc1..u/aa:bb""#),
            "{auth}"
        );
        assert!(auth.contains(
            r#"headers="(request-target) host date x-content-sha256 content-type content-length""#
        ));
        let sha = STANDARD.encode(Sha256::digest(body));
        let signed = format!(
            "(request-target): post /openai/v1/chat/completions\nhost: inference.x.oci.oraclecloud.com\ndate: {date}\nx-content-sha256: {sha}\ncontent-type: application/json\ncontent-length: {}",
            body.len()
        );
        let sig = STANDARD
            .decode(
                auth.rsplit("signature=\"")
                    .next()
                    .unwrap()
                    .trim_end_matches('"'),
            )
            .unwrap();
        assert!(verify(&pem, signed.as_bytes(), &sig));
        // A GET signs only the request line, host and date.
        let g = s
            .sign(
                "GET",
                "h",
                "/openai/v1/models",
                b"",
                date,
                "application/json",
            )
            .unwrap();
        assert!(
            g.iter().any(|(k, v)| k == "authorization"
                && v.contains(r#"headers="(request-target) host date""#))
        );
    }

    #[tokio::test]
    async fn adc_and_entra_get_tokens_and_cache_them() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant-type%3Ajwt-bearer"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "ya29.sa", "expires_in": 3600})),
            )
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/tenant-1/oauth2/v2.0/token"))
            .and(body_string_contains("client_credentials"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "eyJ.entra", "expires_in": 3600})),
            )
            .mount(&s)
            .await;
        let sa = GoogleAdc::from_json(&json!({
            "type": "service_account",
            "client_email": "sa@p.iam.gserviceaccount.com",
            "private_key": fixture("test-only-rsa-pkcs8.pem"),
            "token_uri": format!("{}/token", s.uri()),
        }))
        .unwrap();
        let mut req = reqwest::Request::new(
            reqwest::Method::POST,
            "https://example.invalid/x".parse().unwrap(),
        );
        sa.authorize(&mut req).await.unwrap();
        sa.authorize(&mut req).await.unwrap(); // cached: the token endpoint is asked once
        assert_eq!(req.headers()["authorization"], "Bearer ya29.sa");
        let e = Entra {
            authority: s.uri(),
            tenant: "tenant-1".into(),
            client_id: "app".into(),
            secret: SecretString::from("s".to_owned()),
            cache: Cache::default(),
        };
        e.authorize(&mut req).await.unwrap();
        assert_eq!(req.headers()["authorization"], "Bearer eyJ.entra");
        assert!(GoogleAdc::from_json(&json!({"type": "external_account"})).is_err());
    }
}
