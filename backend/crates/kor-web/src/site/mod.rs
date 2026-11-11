//! Sites: hosting, WordPress, plugins, redirects, proposals, client portal (spec §29, ADR 0054).
//!
//! Everything the user configures lives in Kor's user folder (`kor_config::user_dir()`), never in a
//! project, so a cloned repository cannot point Kor at a server:
//! - `hosts/<name>.toml` panel or server profiles (§29.1), `sites/<name>.toml` WordPress sites (§29.3),
//! - `clients/<id>/` client workspaces with the time log and tickets (§29.13),
//! - `business.toml` rates and company details (§29.12), `templates/web/` editable templates.
//!
//! Secrets are only names here: the values live in the vault (`kor vault add NAME`) or an environment
//! variable of the same name. Every HTTP request goes through `kor_core::net::client_builder()`.

// Product names (WordPress, WooCommerce, cPanel, DirectAdmin…) read better without backticks.
#![allow(clippy::doc_markdown)]

pub mod client;
pub mod cms;
pub mod docs;
pub mod doctor;
pub mod host;
pub mod http;
pub mod panels;
pub mod proposal;
pub mod redirects;
pub mod tls;
pub mod tools;
pub mod tr;
pub mod wp;

use std::path::{Path, PathBuf};

use kor_secrets::{SecretStore, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub type Result<T> = std::result::Result<T, String>;

/// Kor's user folder, where profiles, clients and templates live.
pub fn base() -> Result<PathBuf> {
    std::env::var_os("KOR_WEB_DIR")
        .map(PathBuf::from)
        .or_else(kor_config::user_dir)
        .ok_or_else(|| "no user configuration folder".to_owned())
}

/// Profile, client and site names: `a-z`, `0-9`, `-`, 1–40 characters, starting with a letter or digit.
pub fn valid_name(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub(crate) fn check_name(id: &str) -> Result<()> {
    if valid_name(id) {
        Ok(())
    } else {
        Err(format!("invalid name `{id}`: use a-z, 0-9 and -"))
    }
}

/// Domain names only (no scheme, path or anything a shell could read as an option).
pub fn valid_domain(d: &str) -> bool {
    (3..=253).contains(&d.len())
        && d.contains('.')
        && !d.starts_with(['-', '.'])
        && !d.ends_with('.')
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

pub(crate) fn check_domain(d: &str) -> Result<()> {
    if valid_domain(d) {
        Ok(())
    } else {
        Err(format!("invalid domain `{d}`"))
    }
}

pub(crate) fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_file(path, &text)
}

/// Write through a temporary file and rename, so a crash never leaves half a profile.
pub(crate) fn write_file(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Names (file stems) of the `*.toml` files in `dir`, sorted.
pub(crate) fn names_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "toml"))
                .then(|| p.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .filter(|n| valid_name(n))
        .collect();
    out.sort();
    out
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A secret by name: the environment variable first (CI, tests), then the vault.
pub fn secret(store: Option<&dyn SecretStore>, name: &str) -> Result<SecretString> {
    if let Ok(v) = std::env::var(name)
        && !v.is_empty()
    {
        return Ok(SecretString::from(v));
    }
    let store = store.ok_or_else(|| format!("secret {name} is not set (kor vault add {name})"))?;
    kor_secrets::vault::get(store, name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("secret {name} is not in the vault (kor vault add {name})"))
}

/// The user's configured vault (keychain, or the encrypted file with `KOR_VAULT_PASSPHRASE`).
pub fn vault() -> Result<Box<dyn SecretStore>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let cfg = kor_config::load(&kor_config::Paths::discover(&cwd), std::env::vars(), &[])
        .map(|l| l.config.vault)
        .unwrap_or_default();
    kor_runtime::secure::vault_store(&cfg, None).map_err(|e| e.to_string())
}

/// A strong random password (letters and digits, 24 characters) for accounts Kor creates.
pub fn random_password() -> Result<String> {
    const ABC: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    // 256 % 57 bias is irrelevant at 24 characters of ~5.8 bits each.
    Ok(buf
        .iter()
        .map(|b| ABC[usize::from(*b) % ABC.len()] as char)
        .collect())
}

/// Vault entry name for a credential Kor created: `KOR_WEB_<KIND>_<WHAT>` in upper case.
pub fn credential_name(kind: &str, what: &str) -> String {
    let clean: String = what
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let mut n = format!("KOR_WEB_{}_{clean}", kind.to_ascii_uppercase());
    n.truncate(120);
    n
}

/// Escape for HTML text and attributes.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// Markdown table cell: no pipes or newlines.
pub(crate) fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_domains() {
        assert!(valid_name("acme-shop") && !valid_name("Acme") && !valid_name("-x"));
        assert!(valid_domain("örnek.com".replace('ö', "o").as_str()));
        assert!(!valid_domain("a;b.com") && !valid_domain("-x.com") && !valid_domain("x"));
    }

    #[test]
    fn credential_names_fit_the_vault() {
        let n = credential_name("db", "shop_db");
        assert_eq!(n, "KOR_WEB_DB_SHOP_DB");
        assert!(kor_secrets::vault::valid_name(&n));
        let p = random_password().unwrap();
        assert_eq!(p.len(), 24);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
