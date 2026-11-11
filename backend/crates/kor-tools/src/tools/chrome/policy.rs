//! Domain rules for the user's own browser (§18.2, ADR 0058): `kor chrome allow <domain>` and
//! `kor chrome deny <domain>`, kept in `<config dir>/chrome-domains.json`.
//!
//! A denied domain (and its subdomains) is never opened or acted on. When the allow list is not
//! empty, only its domains are. Every action on a page still asks for approval; the list narrows
//! where Kor may go at all. The extension receives the same lists and checks them again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Domains {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// `<config dir>/chrome-domains.json`.
pub fn file() -> Option<PathBuf> {
    kor_platform::config_dir().map(|d| d.join("chrome-domains.json"))
}

/// `https://Docs.Example.com/x` or `*.example.com` → `docs.example.com` / `example.com`.
///
/// # Errors
/// Not a host name.
pub fn normalize(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_ascii_lowercase();
    let s = s.split_once("://").map_or(s.as_str(), |(_, r)| r);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let s = s.rsplit_once('@').map_or(s, |(_, h)| h);
    let s = s
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("*.")
        .trim_end_matches('.');
    let ok = !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if ok {
        Ok(s.to_owned())
    } else {
        Err(format!("`{raw}` is not a domain name"))
    }
}

fn covers(rule: &str, host: &str) -> bool {
    host == rule || host.strip_suffix(rule).is_some_and(|p| p.ends_with('.'))
}

impl Domains {
    /// Read the file; a missing or broken file is the empty policy.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// # Errors
    /// The file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Add to the allow list (and drop it from the deny list).
    ///
    /// # Errors
    /// Not a domain.
    pub fn allow(&mut self, raw: &str) -> Result<String, String> {
        let d = normalize(raw)?;
        self.deny.retain(|x| *x != d);
        if !self.allow.contains(&d) {
            self.allow.push(d.clone());
            self.allow.sort();
        }
        Ok(d)
    }

    /// Add to the deny list (and drop it from the allow list).
    ///
    /// # Errors
    /// Not a domain.
    pub fn deny(&mut self, raw: &str) -> Result<String, String> {
        let d = normalize(raw)?;
        self.allow.retain(|x| *x != d);
        if !self.deny.contains(&d) {
            self.deny.push(d.clone());
            self.deny.sort();
        }
        Ok(d)
    }

    /// Remove from both lists; returns whether it was listed.
    ///
    /// # Errors
    /// Not a domain.
    pub fn forget(&mut self, raw: &str) -> Result<bool, String> {
        let d = normalize(raw)?;
        let before = self.allow.len() + self.deny.len();
        self.allow.retain(|x| *x != d);
        self.deny.retain(|x| *x != d);
        Ok(before != self.allow.len() + self.deny.len())
    }

    /// Whether Kor may open or act on `url` (http/https only).
    ///
    /// # Errors
    /// The reason, for the model and the user.
    pub fn check(&self, url: &str) -> Result<(), String> {
        let lower = url.trim().to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(format!("only http and https pages, not `{url}`"));
        }
        let host = normalize(url)?;
        if let Some(d) = self.deny.iter().find(|d| covers(d, &host)) {
            return Err(format!(
                "{host} is blocked by the Kor domain list (`kor chrome deny {d}`)"
            ));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|d| covers(d, &host)) {
            return Err(format!(
                "{host} is not on the Kor domain allow list; the user adds it with `kor chrome allow {host}`"
            ));
        }
        Ok(())
    }
}

/// The current policy from the user's config directory.
pub fn current() -> Domains {
    file().map(|f| Domains::load(&f)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_normalize_and_match_subdomains() {
        assert_eq!(
            normalize("https://User@Docs.Example.com:8443/a?b").unwrap(),
            "docs.example.com"
        );
        assert_eq!(normalize("*.example.com.").unwrap(), "example.com");
        assert!(
            normalize("exa mple.com").is_err()
                && normalize("").is_err()
                && normalize("-x.com").is_err()
        );
        let mut d = Domains::default();
        assert!(
            d.check("https://any.site/").is_ok(),
            "empty lists allow everything"
        );
        d.deny("evil.com").unwrap();
        assert!(d.check("https://shop.evil.com/x").is_err());
        assert!(
            d.check("https://notevil.com/").is_ok(),
            "suffix must be a whole label"
        );
        d.allow("example.com").unwrap();
        assert!(d.check("https://www.example.com/").is_ok());
        assert!(d.check("https://other.org/").is_err());
        assert!(d.check("file:///etc/passwd").is_err());
        d.allow("evil.com").unwrap();
        assert!(d.deny.is_empty(), "allow moves it off the deny list");
        assert!(d.forget("evil.com").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c.json");
        d.save(&f).unwrap();
        assert_eq!(Domains::load(&f), d);
    }
}
