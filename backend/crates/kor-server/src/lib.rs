//! Kor team server (spec §11.1): the same `kor` binary, `kor server init|add-member|status|start`.
//!
//! One HTTPS listener (rustls) serves the team: shared rules, templates, hooks and tools (written by
//! admins, synced to every member), shared graph and memory (written by members), a task queue with
//! signed callbacks, a team vault encrypted at rest, expiring share links for customer reports, a
//! hash-chained audit log of every change, and a WebSocket relay for peers and the browser
//! extension (§31.1). Storage is `SQLite` (embedded) or Postgres (`db = "postgres://…"`).
//!
//! Identity: a member is an e-mail with a role (admin, member, viewer). A request proves it with a
//! client certificate from the team CA (mTLS) and/or a bearer token: a member token from
//! `add-member`, or a session token exchanged for an OIDC ID token (Keycloak, Authentik, Google)
//! or a GitHub access token. Unknown identities are refused; nothing is open by default.
//! See `backend/crates/kor-server/README.md`.
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

pub mod auth;
pub mod config;
pub mod db;
pub mod http;
mod pg_tls;
pub mod pki;
pub mod relay;
pub mod store;

pub use config::{OidcProvider, ServerConfig};
pub use store::{Role, Store};

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

pub(crate) fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// 32 random bytes, base64url, with a readable prefix (`kst_…`).
///
/// # Errors
/// The system random source failed.
pub(crate) fn random_token(prefix: &str) -> Result<String, String> {
    let mut b = [0u8; 32];
    aws_lc_rs::rand::fill(&mut b).map_err(|_| "no system randomness".to_owned())?;
    Ok(format!(
        "{prefix}_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    ))
}

/// Files of an initialised server directory.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The server directory.
    pub dir: PathBuf,
}

impl Layout {
    /// The layout of `dir`.
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }
    /// `server.toml`.
    pub fn config(&self) -> PathBuf {
        self.dir.join("server.toml")
    }
    /// Team CA certificate.
    pub fn ca_cert(&self) -> PathBuf {
        self.dir.join("ca.pem")
    }
    /// Team CA private key.
    pub fn ca_key(&self) -> PathBuf {
        self.dir.join("ca.key")
    }
    /// Server certificate.
    pub fn server_cert(&self) -> PathBuf {
        self.dir.join("server.pem")
    }
    /// Server private key.
    pub fn server_key(&self) -> PathBuf {
        self.dir.join("server.key")
    }
    /// Vault encryption key.
    pub fn vault_key(&self) -> PathBuf {
        self.dir.join("vault.key")
    }
    /// Folder of member bundles.
    pub fn members(&self) -> PathBuf {
        self.dir.join("members")
    }
}

/// `kor server init`: config, CA, server certificate, vault key and the first admin.
/// Refuses to overwrite an existing server.
///
/// # Errors
/// An existing server, a bad domain/e-mail, or an I/O/database error.
pub async fn init(
    dir: &Path,
    team: &str,
    domain: &str,
    admin: &str,
    db: Option<&str>,
) -> Result<(ServerConfig, store::Enrollment), String> {
    let l = Layout::new(dir);
    if l.config().exists() {
        return Err(format!("{} already exists", l.config().display()));
    }
    config::check_domain(domain)?;
    let mut cfg = ServerConfig {
        team: team.trim().to_owned(),
        domain: domain.trim().to_owned(),
        ..ServerConfig::default()
    };
    if let Some(db) = db {
        cfg.db = db.to_owned();
    }
    cfg.validate()?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let ca = pki::create_ca(&cfg.team)?;
    let server = pki::issue_server(&cfg.team, &ca.key_pem, &cfg.domain)?;
    std::fs::write(l.ca_cert(), &ca.cert_pem).map_err(|e| e.to_string())?;
    pki::write_private(&l.ca_key(), &ca.key_pem)?;
    std::fs::write(
        l.server_cert(),
        format!("{}{}", server.cert_pem, ca.cert_pem),
    )
    .map_err(|e| e.to_string())?;
    pki::write_private(&l.server_key(), &server.key_pem)?;
    let mut key = [0u8; 32];
    aws_lc_rs::rand::fill(&mut key).map_err(|_| "no system randomness".to_owned())?;
    pki::write_private(&l.vault_key(), &hex(&key))?;
    cfg.save(&l.config())?;
    let st = Store::open(&cfg, dir).await?;
    let e = st
        .add_member(&cfg, &l, admin, Role::Admin, "init")
        .await
        .map_err(|e| e.message)?;
    Ok((cfg, e))
}
