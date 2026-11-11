//! Pairing the browser extension with Kor (§18.1, ADR 0058).
//!
//! `kor chrome pair` (or the desktop button) writes a one-time code: 8 characters from an
//! alphabet without look-alikes, shown as `ABCD-EFGH`, valid 10 minutes, at most 5 wrong tries.
//! Only its SHA-256 is stored (`<data dir>/chrome/pair.json`), so the process running the bridge
//! (desktop or CLI) can check it. The extension sends the code once; the bridge answers with a
//! 256-bit key that the extension keeps in `chrome.storage.local`. Kor keeps only the keys'
//! SHA-256 in the OS keychain (`kor-secrets`, account `chrome-bridge:keys`), never the key itself.
//! Every later connection must present a known key; anything else is closed.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kor_secrets::{ExposeSecret as _, SecretStore, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const ACCOUNT: &str = "chrome-bridge:keys";
const CODE_TTL: Duration = Duration::from_secs(600);
const MAX_TRIES: u32 = 5;
/// No 0/O, 1/I/L, 5/S, 8/B.
const ALPHABET: &[u8] = b"ACDEFGHJKMNPQRTUVWXYZ2345679";
const MAX_KEYS: usize = 16;

#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    hash: String,
    expires: u64,
    tries: u32,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn sha(text: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn random(n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| format!("no secure random source: {e}"))?;
    Ok(b)
}

/// `ab cd-EFGH` → `ABCDEFGH`.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn pending_file(dir: &Path) -> PathBuf {
    dir.join("pair.json")
}

fn write_pending(dir: &Path, p: &Pending) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join("pair.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(p).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, pending_file(dir)).map_err(|e| e.to_string())
}

/// A new one-time code (replacing any earlier one) and when it expires (Unix seconds).
///
/// # Errors
/// No random source or the file cannot be written.
pub fn new_code(dir: &Path) -> Result<(String, u64), String> {
    let raw: String = random(8)?
        .iter()
        // 256 % 28 leaves a bias of < 1 %; fine for a short-lived, try-limited code.
        .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()]))
        .collect();
    let expires = now() + CODE_TTL.as_secs();
    write_pending(
        dir,
        &Pending {
            hash: sha(&raw),
            expires,
            tries: 0,
        },
    )?;
    Ok((format!("{}-{}", &raw[..4], &raw[4..]), expires))
}

/// Whether a code is waiting (and not expired).
pub fn code_pending(dir: &Path) -> bool {
    std::fs::read(pending_file(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Pending>(&b).ok())
        .is_some_and(|p| p.expires > now() && p.tries < MAX_TRIES)
}

fn keys(store: &dyn SecretStore) -> Vec<String> {
    store
        .get(ACCOUNT)
        .ok()
        .flatten()
        .map(|s| {
            s.expose_secret()
                .lines()
                .filter(|l| l.len() == 64)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Trade a one-time code for a new key. The code is used up on success, and after
/// [`MAX_TRIES`] wrong codes.
///
/// # Errors
/// Wrong, expired or missing code (one generic message), or the keychain refused the write.
pub fn redeem(dir: &Path, code: &str, store: &dyn SecretStore) -> Result<String, String> {
    const REFUSED: &str = "pairing code is wrong or expired; run `kor chrome pair` again";
    let file = pending_file(dir);
    let mut p: Pending = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(REFUSED)?;
    if p.expires <= now() || p.tries >= MAX_TRIES {
        let _ = std::fs::remove_file(&file);
        return Err(REFUSED.into());
    }
    if !super::server::same(sha(&normalize(code)).as_bytes(), p.hash.as_bytes()) {
        p.tries += 1;
        if p.tries >= MAX_TRIES {
            let _ = std::fs::remove_file(&file);
        } else {
            write_pending(dir, &p)?;
        }
        return Err(REFUSED.into());
    }
    let _ = std::fs::remove_file(&file);
    let key: String = random(32)?.iter().fold(String::new(), |mut s, b| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        s
    });
    let mut list = keys(store);
    list.push(sha(&key));
    // Oldest pairings fall off; a browser profile that was reset does not keep a slot forever.
    let skip = list.len().saturating_sub(MAX_KEYS);
    let text = list[skip..].join("\n");
    store
        .set(ACCOUNT, &SecretString::from(text))
        .map_err(|e| format!("cannot store the pairing in the keychain: {e}"))?;
    Ok(key)
}

/// Whether `key` was issued by [`redeem`] and not revoked.
pub fn known_key(store: &dyn SecretStore, key: &str) -> bool {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let h = sha(key);
    keys(store).iter().fold(false, |hit, k| {
        hit | super::server::same(k.as_bytes(), h.as_bytes())
    })
}

/// Number of paired browsers.
pub fn paired_count(store: &dyn SecretStore) -> usize {
    keys(store).len()
}

/// Forget every pairing (and any waiting code).
///
/// # Errors
/// The keychain refused the delete.
pub fn revoke_all(dir: &Path, store: &dyn SecretStore) -> Result<(), String> {
    let _ = std::fs::remove_file(pending_file(dir));
    store.delete(ACCOUNT).map(drop).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;

    #[test]
    fn a_code_pairs_once_and_keys_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let (code, _) = new_code(dir.path()).unwrap();
        assert_eq!(code.len(), 9);
        assert!(code_pending(dir.path()));
        let key = redeem(dir.path(), &code.to_lowercase().replace('-', " "), &store).unwrap();
        assert!(known_key(&store, &key));
        assert!(!known_key(&store, &"0".repeat(64)));
        assert!(!known_key(&store, "short"));
        // One-time.
        assert!(redeem(dir.path(), &code, &store).is_err());
        assert!(!code_pending(dir.path()));
        assert_eq!(paired_count(&store), 1);
        revoke_all(dir.path(), &store).unwrap();
        assert!(!known_key(&store, &key));
    }

    #[test]
    fn wrong_codes_burn_the_code_after_five_tries() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let (code, _) = new_code(dir.path()).unwrap();
        for _ in 0..MAX_TRIES {
            assert!(redeem(dir.path(), "AAAA-AAAA", &store).is_err());
        }
        assert!(redeem(dir.path(), &code, &store).is_err(), "code is gone");
    }

    #[test]
    fn expired_codes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let (code, _) = new_code(dir.path()).unwrap();
        let mut p: Pending =
            serde_json::from_slice(&std::fs::read(pending_file(dir.path())).unwrap()).unwrap();
        p.expires = now() - 1;
        write_pending(dir.path(), &p).unwrap();
        assert!(redeem(dir.path(), &code, &store).is_err());
    }
}
