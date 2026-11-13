//! Single sign-on (§11.1 "Kimlik"): OIDC ID tokens (Keycloak, Authentik, Google) checked against
//! the issuer's JWKS, and GitHub access tokens checked against the GitHub API. The result is the
//! member's verified e-mail; the server then looks up the role and issues a short session token.
//!
//! ID token checks: signature (RS256/384/512, ES256/384 only; `none` and HMAC algorithms are
//! refused, so a public key can never be used as an HMAC secret), `iss` equals the configured
//! issuer exactly, `aud` contains the client id (and `azp` matches when there are several
//! audiences), `exp`/`nbf` within 60 s of skew, a verified `email`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aws_lc_rs::signature::{self, RsaPublicKeyComponents, UnparsedPublicKey};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;

use crate::config::OidcProvider;
use crate::store::{Error, Result, normalize_email};

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const MAX_TOKEN: usize = 16 * 1024;
const SKEW_S: i64 = 60;
const JWKS_TTL: Duration = Duration::from_secs(600);
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    #[serde(default)]
    kid: Option<String>,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Default)]
struct Cache {
    /// Provider name → (fetched at, keys).
    jwks: HashMap<String, (Instant, Vec<Jwk>)>,
    /// Provider name → discovery document.
    discovery: HashMap<String, (Instant, Value)>,
}

/// Checks SSO tokens; keeps the providers' keys for ten minutes.
#[derive(Debug)]
pub struct Verifier {
    http: reqwest::Client,
    cache: tokio::sync::Mutex<Cache>,
}

fn unauthorized(msg: &str) -> Error {
    Error::new(401, msg)
}

fn now_s() -> i64 {
    crate::now_ms() / 1000
}

impl Verifier {
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn new() -> std::result::Result<Self, String> {
        let http = kor_core::net::client_builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            cache: tokio::sync::Mutex::new(Cache::default()),
        })
    }

    async fn get_json(&self, url: &str, bearer: Option<&str>) -> Result<Value> {
        let mut r = self
            .http
            .get(url)
            .header("accept", "application/json")
            .header("user-agent", "kor-server");
        if let Some(t) = bearer {
            r = r.bearer_auth(t);
        }
        let resp = r.send().await.map_err(|e| {
            Error::new(
                502,
                format!("identity provider: {}", kor_core::net::describe(&e)),
            )
        })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::new(502, format!("identity provider: {e}")))?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(unauthorized("the identity provider refused the token"));
        }
        if !status.is_success() || text.len() > 1024 * 1024 {
            return Err(Error::new(
                502,
                format!("identity provider answered {status}"),
            ));
        }
        serde_json::from_str(&text)
            .map_err(|_| Error::new(502, "identity provider sent invalid JSON"))
    }

    /// The issuer's discovery document (`jwks_uri`, `device_authorization_endpoint`, …);
    /// its `issuer` must equal the configured one.
    ///
    /// # Errors
    /// The provider cannot be reached or the document does not match.
    pub async fn discovery(&self, p: &OidcProvider) -> Result<Value> {
        if let Some((at, v)) = self.cache.lock().await.discovery.get(&p.name)
            && at.elapsed() < JWKS_TTL
        {
            return Ok(v.clone());
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            p.issuer.trim_end_matches('/')
        );
        let v = self.get_json(&url, None).await?;
        if v["issuer"].as_str() != Some(p.issuer.as_str()) {
            return Err(Error::new(
                502,
                "the discovery document names another issuer",
            ));
        }
        self.cache
            .lock()
            .await
            .discovery
            .insert(p.name.clone(), (Instant::now(), v.clone()));
        Ok(v)
    }

    async fn keys(&self, p: &OidcProvider, force: bool) -> Result<Vec<Jwk>> {
        {
            let c = self.cache.lock().await;
            if let Some((at, k)) = c.jwks.get(&p.name) {
                let fresh = at.elapsed() < JWKS_TTL;
                // An unknown `kid` refetches (key rotation), but not more than every 30 s.
                if fresh && (!force || at.elapsed() < JWKS_MIN_REFRESH) {
                    return Ok(k.clone());
                }
            }
        }
        let uri = if p.jwks_uri.is_empty() {
            self.discovery(p).await?["jwks_uri"]
                .as_str()
                .ok_or_else(|| Error::new(502, "no jwks_uri in the discovery document"))?
                .to_owned()
        } else {
            p.jwks_uri.clone()
        };
        let v = self.get_json(&uri, None).await?;
        let keys: Vec<Jwk> = serde_json::from_value(v["keys"].clone())
            .map_err(|_| Error::new(502, "the JWKS has no keys"))?;
        self.cache
            .lock()
            .await
            .jwks
            .insert(p.name.clone(), (Instant::now(), keys.clone()));
        Ok(keys)
    }

    /// Check an ID token from `p`; the verified e-mail.
    ///
    /// # Errors
    /// 401 for any invalid token; 502 when the provider cannot be reached.
    pub async fn verify_id_token(&self, p: &OidcProvider, token: &str) -> Result<String> {
        if token.len() > MAX_TOKEN {
            return Err(unauthorized("token too large"));
        }
        let mut parts = token.split('.');
        let (Some(h), Some(c), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(unauthorized("not a JWT"));
        };
        let header: Value = B64URL
            .decode(h)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| unauthorized("bad JWT header"))?;
        let claims: Value = B64URL
            .decode(c)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| unauthorized("bad JWT claims"))?;
        let sig = B64URL
            .decode(s)
            .map_err(|_| unauthorized("bad JWT signature"))?;
        if header.get("crit").is_some() {
            return Err(unauthorized("unsupported critical JWT header"));
        }
        let alg = header["alg"].as_str().unwrap_or_default();
        if !matches!(alg, "RS256" | "RS384" | "RS512" | "ES256" | "ES384") {
            return Err(unauthorized("unsupported JWT algorithm"));
        }
        let kid = header["kid"].as_str();
        let signed = &token[..h.len() + 1 + c.len()];
        let mut keys = self.keys(p, false).await?;
        if kid.is_some() && !keys.iter().any(|k| k.kid.as_deref() == kid) {
            keys = self.keys(p, true).await?;
        }
        let ok = keys
            .iter()
            .filter(|k| kid.is_none() || k.kid.as_deref() == kid)
            .filter(|k| k.usage.as_deref().is_none_or(|u| u == "sig"))
            .filter(|k| k.alg.as_deref().is_none_or(|a| a == alg))
            .any(|k| check_sig(k, alg, signed.as_bytes(), &sig));
        if !ok {
            return Err(unauthorized("JWT signature does not verify"));
        }
        check_claims(p, &claims, now_s())
    }

    /// The verified primary e-mail behind a GitHub access token (and, with `org`, active
    /// membership in that organisation).
    ///
    /// # Errors
    /// 401 for a refused token, no verified e-mail or no membership.
    pub async fn github_email(&self, p: &OidcProvider, token: &str) -> Result<String> {
        if token.is_empty() || token.len() > 512 {
            return Err(unauthorized("bad access token"));
        }
        let api = if p.api.is_empty() {
            "https://api.github.com"
        } else {
            p.api.trim_end_matches('/')
        };
        let emails = self
            .get_json(&format!("{api}/user/emails"), Some(token))
            .await?;
        let email = emails
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e["primary"] == true && e["verified"] == true)
            .and_then(|e| e["email"].as_str())
            .ok_or_else(|| unauthorized("no verified primary e-mail on the GitHub account"))?;
        if !p.org.is_empty() {
            let m = self
                .get_json(
                    &format!("{api}/user/memberships/orgs/{}", p.org),
                    Some(token),
                )
                .await
                .map_err(|_| unauthorized("not a member of the GitHub organisation"))?;
            if m["state"] != "active" {
                return Err(unauthorized(
                    "not an active member of the GitHub organisation",
                ));
            }
        }
        normalize_email(email)
    }
}

fn check_sig(k: &Jwk, alg: &str, msg: &[u8], sig: &[u8]) -> bool {
    let d = |s: &Option<String>| s.as_deref().and_then(|v| B64URL.decode(v).ok());
    match (k.kty.as_str(), alg) {
        ("RSA", "RS256" | "RS384" | "RS512") => {
            let (Some(n), Some(e)) = (d(&k.n), d(&k.e)) else {
                return false;
            };
            let params = match alg {
                "RS256" => &signature::RSA_PKCS1_2048_8192_SHA256,
                "RS384" => &signature::RSA_PKCS1_2048_8192_SHA384,
                _ => &signature::RSA_PKCS1_2048_8192_SHA512,
            };
            RsaPublicKeyComponents { n: &n, e: &e }
                .verify(params, msg, sig)
                .is_ok()
        }
        ("EC", "ES256" | "ES384") => {
            let (Some(x), Some(y)) = (d(&k.x), d(&k.y)) else {
                return false;
            };
            let (crv, alg_ref, len): (_, &'static dyn signature::VerificationAlgorithm, _) =
                if alg == "ES256" {
                    ("P-256", &signature::ECDSA_P256_SHA256_FIXED, 32)
                } else {
                    ("P-384", &signature::ECDSA_P384_SHA384_FIXED, 48)
                };
            if k.crv.as_deref() != Some(crv) || x.len() != len || y.len() != len {
                return false;
            }
            let mut point = Vec::with_capacity(1 + 2 * len);
            point.push(4);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            UnparsedPublicKey::new(alg_ref, point)
                .verify(msg, sig)
                .is_ok()
        }
        _ => false,
    }
}

/// Issuer, audience, time window and e-mail of verified claims.
///
/// # Errors
/// 401 naming the failed check.
pub fn check_claims(p: &OidcProvider, c: &Value, now: i64) -> Result<String> {
    if c["iss"].as_str() != Some(p.issuer.as_str()) {
        return Err(unauthorized("wrong issuer"));
    }
    let aud: Vec<&str> = match &c["aud"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !aud.contains(&p.client_id.as_str()) {
        return Err(unauthorized("wrong audience"));
    }
    if aud.len() > 1 && c["azp"].as_str() != Some(p.client_id.as_str()) {
        return Err(unauthorized("wrong authorized party"));
    }
    let exp = c["exp"].as_i64().ok_or_else(|| unauthorized("no expiry"))?;
    if now > exp + SKEW_S {
        return Err(unauthorized("token expired"));
    }
    if c["nbf"].as_i64().is_some_and(|nbf| now + SKEW_S < nbf) {
        return Err(unauthorized("token not valid yet"));
    }
    if c["email_verified"] == false || c["email_verified"] == "false" {
        return Err(unauthorized("e-mail not verified at the identity provider"));
    }
    let email = c["email"]
        .as_str()
        .ok_or_else(|| unauthorized("no e-mail claim"))?;
    normalize_email(email).map_err(|_| unauthorized("bad e-mail claim"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p() -> OidcProvider {
        OidcProvider {
            name: "kc".into(),
            kind: "oidc".into(),
            issuer: "https://id.example.com/realms/acme".into(),
            client_id: "kor".into(),
            ..OidcProvider::default()
        }
    }

    #[test]
    fn claims_are_checked() {
        let now = 1_700_000_000;
        let good = json!({ "iss": "https://id.example.com/realms/acme", "aud": "kor", "exp": now + 300, "email": "Ali@Example.com", "email_verified": true });
        assert_eq!(check_claims(&p(), &good, now).unwrap(), "ali@example.com");
        let with = |k: &str, v: Value| {
            let mut c = good.clone();
            c[k] = v;
            check_claims(&p(), &c, now).unwrap_err().message
        };
        assert_eq!(
            with("iss", json!("https://evil.example.com")),
            "wrong issuer"
        );
        assert_eq!(with("aud", json!("other")), "wrong audience");
        assert_eq!(
            with("aud", json!(["kor", "other"])),
            "wrong authorized party"
        );
        assert_eq!(with("exp", json!(now - 61)), "token expired");
        assert_eq!(with("nbf", json!(now + 120)), "token not valid yet");
        assert_eq!(
            with("email_verified", json!(false)),
            "e-mail not verified at the identity provider"
        );
        let mut c = good.clone();
        c.as_object_mut().unwrap().remove("exp");
        assert_eq!(
            check_claims(&p(), &c, now).unwrap_err().message,
            "no expiry"
        );
    }
}
