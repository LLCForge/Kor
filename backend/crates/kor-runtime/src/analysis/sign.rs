//! §9.1: link a written report to the hash-chained audit log and optionally sign it (ADR 0045).
//!
//! What is signed is the report file exactly as written (its bytes), nothing else:
//! - always: an `analysis.report` entry in `audit.jsonl` with the file's SHA-256, the tool and
//!   the per-severity counts. The chain makes that entry tamper-evident; it is the "signature"
//!   when no key is configured.
//! - with a minisign secret key (`KOR_AUDIT_SIGN_KEY`, else `<kor home>/audit-sign.key` when it
//!   exists; password from `KOR_AUDIT_SIGN_PASSWORD`, empty for unencrypted keys): a
//!   `<report>.minisig` next to the file. Its trusted comment carries the SHA-256 and the audit
//!   entry hash; the public key is stored in the audit entry so `verify-report` needs no extra
//!   configuration (or `minisign -Vm <report> -P <pubkey>` from the outside).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Report;

pub const ACTION: &str = "analysis.report";
pub const KEY_ENV: &str = "KOR_AUDIT_SIGN_KEY";
pub const PASSWORD_ENV: &str = "KOR_AUDIT_SIGN_PASSWORD";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignOutcome {
    pub sha256: String,
    /// Hash of the audit entry; `None` when the audit log is off (no data directory).
    pub audit_hash: Option<String>,
    /// The written `.minisig`, when a key was configured.
    pub signature: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyOutcome {
    pub sha256: String,
    /// Timestamp of the matching `analysis.report` entry.
    pub linked_at: Option<String>,
    /// The audit chain itself verifies.
    pub chain_ok: bool,
    /// `Some(true)` valid `.minisig`, `Some(false)` present but invalid, `None` no signature.
    pub signature_ok: Option<bool>,
}

impl VerifyOutcome {
    pub fn ok(&self) -> bool {
        self.linked_at.is_some() && self.chain_ok && self.signature_ok != Some(false)
    }
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

fn sig_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(".minisig");
    PathBuf::from(s)
}

/// The configured minisign secret key file, if any.
pub fn key_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(KEY_ENV).filter(|v| !v.is_empty()) {
        return Some(p.into());
    }
    crate::kor_home()
        .map(|h| h.join("audit-sign.key"))
        .filter(|p| p.is_file())
}

/// Link `file` (the report as written) to the audit log at `audit` and sign it with `key`.
pub fn sign_and_link_at(
    report: &Report,
    file: &Path,
    audit: Option<&Path>,
    key: Option<&Path>,
) -> Result<SignOutcome, String> {
    let sha256 = sha256_file(file).map_err(|e| e.to_string())?;
    let c = report.counts();
    let mut fields = json!({
        "tool": report.tool,
        "target": file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "sha256": sha256,
        "findings": report.findings.len(),
        "critical": c[0], "high": c[1], "medium": c[2], "low": c[3], "info": c[4],
    });
    let sk = match key {
        Some(k) => {
            let pw = std::env::var(PASSWORD_ENV).unwrap_or_default();
            // Always pass a password: `None` would prompt on the terminal.
            let sk = minisign::SecretKey::from_file(k, Some(pw)).map_err(|e| e.to_string())?;
            let pk = minisign::PublicKey::from_secret_key(&sk).map_err(|e| e.to_string())?;
            fields["pubkey"] = json!(pk.to_base64());
            Some(sk)
        }
        None => None,
    };
    let audit_hash = match audit {
        Some(a) => Some(kor_agent::audit::append(a, ACTION, fields).map_err(|e| e.to_string())?),
        None => None,
    };
    let signature = match sk {
        Some(sk) => {
            let trusted = format!(
                "kor {} sha256:{sha256} audit:{}",
                report.tool,
                audit_hash.as_deref().unwrap_or("-")
            );
            let data = std::fs::read(file).map_err(|e| e.to_string())?;
            let sig = minisign::sign(
                None,
                &sk,
                std::io::Cursor::new(data),
                Some(&trusted),
                Some("signed by kor audit"),
            )
            .map_err(|e| e.to_string())?;
            let out = sig_path(file);
            std::fs::write(&out, sig.into_string()).map_err(|e| e.to_string())?;
            Some(out)
        }
        None => None,
    };
    Ok(SignOutcome {
        sha256,
        audit_hash,
        signature,
    })
}

/// [`sign_and_link_at`] with the user's audit log and configured key.
pub fn sign_and_link(report: &Report, file: &Path) -> Result<SignOutcome, String> {
    let audit = crate::secure::audit_path();
    let key = key_path();
    sign_and_link_at(report, file, audit.as_deref(), key.as_deref())
}

/// Find the report's audit entry, check the chain and the `.minisig` (if present).
pub fn verify_report_at(file: &Path, audit: &Path) -> Result<VerifyOutcome, String> {
    let sha256 = sha256_file(file).map_err(|e| e.to_string())?;
    let mut entry = None;
    kor_agent::audit::each(audit, |v| {
        if v["action"] == ACTION && v["sha256"] == sha256.as_str() {
            entry = Some(v);
        }
    })
    .map_err(|e| e.to_string())?;
    let chain_ok = kor_agent::audit::verify(audit)
        .map_err(|e| e.to_string())?
        .ok();
    let sig = sig_path(file);
    let signature_ok = if sig.is_file() {
        let pk = entry.as_ref().and_then(|e| e["pubkey"].as_str());
        Some(pk.is_some_and(|pk| check_sig(file, &sig, pk)))
    } else {
        None
    };
    Ok(VerifyOutcome {
        sha256,
        linked_at: entry.and_then(|e| e["ts"].as_str().map(str::to_owned)),
        chain_ok,
        signature_ok,
    })
}

pub fn verify_report(file: &Path) -> Result<VerifyOutcome, String> {
    let audit = crate::secure::audit_path().ok_or("no data directory")?;
    verify_report_at(file, &audit)
}

fn check_sig(file: &Path, sig: &Path, pk: &str) -> bool {
    let Ok(pk) = minisign_verify::PublicKey::from_base64(pk) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(sig) else {
        return false;
    };
    let Ok(sig) = minisign_verify::Signature::decode(&text) else {
        return false;
    };
    std::fs::read(file).is_ok_and(|data| pk.verify(&data, &sig, false).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_sign_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let audit = dir.path().join("audit.jsonl");
        let file = dir.path().join("r.sarif");
        std::fs::write(&file, "{}").unwrap();
        let key = dir.path().join("k.key");
        // Empty password = an unencrypted key (`minisign -G -W`).
        let kp = minisign::KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
        std::fs::write(&key, kp.sk.to_box(None).unwrap().into_string()).unwrap();

        let r = Report::new("security");
        let out = sign_and_link_at(&r, &file, Some(&audit), Some(&key)).unwrap();
        assert!(out.audit_hash.is_some());
        assert!(out.signature.as_ref().unwrap().is_file());
        let v = verify_report_at(&file, &audit).unwrap();
        assert!(v.ok(), "{v:?}");
        assert_eq!(v.signature_ok, Some(true));

        // Any change to the report breaks both links.
        std::fs::write(&file, "{ }").unwrap();
        let v = verify_report_at(&file, &audit).unwrap();
        assert!(v.linked_at.is_none());
        assert_eq!(v.signature_ok, Some(false));
        assert!(!v.ok());
    }

    #[test]
    fn hash_only_without_key() {
        let dir = tempfile::tempdir().unwrap();
        let audit = dir.path().join("audit.jsonl");
        let file = dir.path().join("r.md");
        std::fs::write(&file, "# r").unwrap();
        let out = sign_and_link_at(&Report::new("x"), &file, Some(&audit), None).unwrap();
        assert!(out.signature.is_none());
        let v = verify_report_at(&file, &audit).unwrap();
        assert!(v.ok());
        assert_eq!(v.signature_ok, None);
    }
}
