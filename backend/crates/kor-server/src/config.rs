//! `server.toml` in the server directory.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One identity provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidcProvider {
    /// Short name members pick (`keycloak`, `google`, `github`).
    pub name: String,
    /// `oidc` (Keycloak, Authentik, Google: ID tokens checked against the issuer's JWKS) or
    /// `github` (an access token; the verified primary e-mail comes from the GitHub API).
    pub kind: String,
    /// `oidc`: the exact `iss` value (`https://id.example.com/realms/acme`,
    /// `https://accounts.google.com`, `https://auth.example.com/application/o/kor/`).
    pub issuer: String,
    /// The OAuth client id; for `oidc` the required `aud`.
    pub client_id: String,
    /// Overrides discovery (`<issuer>/.well-known/openid-configuration`).
    pub jwks_uri: String,
    /// Device authorization endpoint (overrides discovery).
    pub device_endpoint: String,
    /// Token endpoint (overrides discovery).
    pub token_endpoint: String,
    /// `github`: API base (`https://api.github.com`, or GitHub Enterprise's `/api/v3`).
    pub api: String,
    /// `github`: only members of this organisation may sign in.
    pub org: String,
}

/// Server settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Team name (the CA's organisation).
    pub team: String,
    /// Public host name members use; in the server certificate.
    pub domain: String,
    /// Address to listen on (`host:port`).
    pub listen: String,
    /// `sqlite:<file>` (relative to the server directory) or `postgres://…?sslmode=require`.
    pub db: String,
    /// Every member request must present a team client certificate (mTLS). Off only when a
    /// reverse proxy terminates TLS or browsers connect without one.
    pub require_client_cert: bool,
    /// Hours a session token from SSO lives.
    pub session_hours: u32,
    /// Days a member certificate and member token from `add-member` live.
    pub member_days: u32,
    /// Allow task callbacks to private/loopback addresses (tests, on-prem CI). Off: public only.
    pub allow_private_callbacks: bool,
    /// Largest request body in bytes.
    pub max_body: usize,
    /// Single sign-on providers.
    #[serde(rename = "oidc")]
    pub providers: Vec<OidcProvider>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            team: "team".into(),
            domain: "localhost".into(),
            listen: "0.0.0.0:8443".into(),
            db: "sqlite:server.sqlite".into(),
            require_client_cert: true,
            session_hours: 12,
            member_days: 365,
            allow_private_callbacks: false,
            max_body: 8 * 1024 * 1024,
            providers: Vec::new(),
        }
    }
}

/// A host name: letters, digits, `-`, `.`.
///
/// # Errors
/// Anything else.
pub fn check_domain(d: &str) -> Result<(), String> {
    let d = d.trim();
    let ok = !d.is_empty()
        && d.len() <= 253
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(format!("`{d}` is not a host name"))
    }
}

impl ServerConfig {
    /// # Errors
    /// A missing/unreadable/invalid file.
    pub fn load(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("{}: {e} (run `kor server init`)", file.display()))?;
        let c: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        c.validate()?;
        Ok(c)
    }

    /// # Errors
    /// An I/O error.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// # Errors
    /// The first invalid setting.
    pub fn validate(&self) -> Result<(), String> {
        if self.team.trim().is_empty()
            || self.team.len() > 64
            || self.team.chars().any(char::is_control)
        {
            return Err("team: 1–64 printable characters".into());
        }
        check_domain(&self.domain)?;
        self.listen
            .parse::<std::net::SocketAddr>()
            .map_err(|_| format!("listen: `{}` is not host:port", self.listen))?;
        if !(self.db.starts_with("sqlite:")
            || self.db.starts_with("postgres://")
            || self.db.starts_with("postgresql://"))
        {
            return Err("db: `sqlite:<file>` or `postgres://…`".into());
        }
        if self.session_hours == 0 || self.session_hours > 24 * 30 {
            return Err("session_hours: 1–720".into());
        }
        if self.member_days == 0 || self.member_days > 3650 {
            return Err("member_days: 1–3650".into());
        }
        if self.max_body < 1024 || self.max_body > 64 * 1024 * 1024 {
            return Err("max_body: 1 KiB – 64 MiB".into());
        }
        for p in &self.providers {
            if p.name.is_empty()
                || !p
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(format!("oidc name `{}`: letters, digits, - and _", p.name));
            }
            match p.kind.as_str() {
                "oidc" => {
                    if !p.issuer.starts_with("https://")
                        && !p.issuer.starts_with("http://127.0.0.1")
                    {
                        return Err(format!("oidc `{}`: issuer must be https://", p.name));
                    }
                    if p.client_id.is_empty() {
                        return Err(format!(
                            "oidc `{}`: client_id is required (the audience)",
                            p.name
                        ));
                    }
                }
                "github" => {}
                other => {
                    return Err(format!(
                        "oidc `{}`: kind `{other}` is not oidc or github",
                        p.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// The provider called `name`.
    pub fn provider(&self, name: &str) -> Option<&OidcProvider> {
        self.providers.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_round_trip() {
        let c = ServerConfig::default();
        c.validate().unwrap();
        let back: ServerConfig = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert!(check_domain("kor.example.com").is_ok());
        for bad in ["", "a..b", "-a.com", "a b", "kor.example.com/x"] {
            assert!(check_domain(bad).is_err(), "{bad}");
        }
        let mut c = ServerConfig::default();
        c.providers.push(OidcProvider {
            name: "kc".into(),
            kind: "oidc".into(),
            issuer: "http://id.example.com".into(),
            client_id: "kor".into(),
            ..OidcProvider::default()
        });
        assert!(c.validate().is_err(), "plain-http issuer");
    }
}
