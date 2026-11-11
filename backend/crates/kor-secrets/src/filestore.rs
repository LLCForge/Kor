//! Encrypted local vault (spec §6.6 "şifreli yerel kasa", ADR 0036): the opt-in fallback for
//! machines without an OS keychain (Linux without Secret Service), `[vault] backend = "file"`.
//!
//! One JSON file: `{ "v": 1, "kdf": { "alg": "argon2id", "m", "t", "p", "salt" }, "nonce", "data" }`.
//! `data` is the account → value map (JSON) sealed with XChaCha20-Poly1305 under a key derived
//! from the user's passphrase with Argon2id; the header is the associated data, so its KDF
//! parameters cannot be swapped. Every change re-encrypts the whole map with a fresh nonce and
//! replaces the file atomically (temporary file + rename). The passphrase is never stored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{ExposeSecret, SecretError, SecretStore, SecretString};

/// Argon2id cost: 64 MiB, 3 passes, 1 lane (above the OWASP minimum of 19 MiB / 2 passes).
pub const DEFAULT_KDF: Kdf = Kdf {
    m: 64 * 1024,
    t: 3,
    p: 1,
};
/// Larger files are refused before decryption (512 entries of at most 8 KB fit easily).
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// Argon2id parameters: memory in KiB, passes, lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kdf {
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

/// The encrypted vault file. Each operation re-reads the file, so changes made by another
/// process (the CLI while the desktop app runs) are seen.
pub struct FileStore {
    path: PathBuf,
    kdf: Kdf,
    salt: Vec<u8>,
    key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for FileStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn bad(msg: impl Into<String>) -> SecretError {
    SecretError::VaultFile(msg.into())
}

fn derive(pass: &SecretString, salt: &[u8], k: Kdf) -> Result<Zeroizing<[u8; 32]>, SecretError> {
    let params = Params::new(k.m, k.t, k.p, Some(32)).map_err(|e| bad(e.to_string()))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass.expose_secret().as_bytes(), salt, &mut *key)
        .map_err(|e| bad(e.to_string()))?;
    Ok(key)
}

fn aad(k: Kdf, salt: &str) -> String {
    format!("kor-vault/1/argon2id/{}/{}/{}/{salt}", k.m, k.t, k.p)
}

/// The parsed header and sealed payload of an existing file.
struct Sealed {
    kdf: Kdf,
    salt: Vec<u8>,
    nonce: Vec<u8>,
    data: Vec<u8>,
}

fn read_sealed(path: &Path) -> Result<Option<Sealed>, SecretError> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(bad(e.to_string())),
    };
    if meta.len() > MAX_FILE {
        return Err(bad("file too large"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| bad(e.to_string()))?;
    let v: Value = serde_json::from_str(&text).map_err(|_| bad("damaged file"))?;
    let num = |k: &str| {
        v["kdf"][k]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad("damaged file"))
    };
    let b64 = |s: &Value| {
        s.as_str()
            .and_then(|s| B64.decode(s).ok())
            .ok_or_else(|| bad("damaged file"))
    };
    if v["v"] != 1 || v["kdf"]["alg"] != "argon2id" {
        return Err(bad("unsupported file version"));
    }
    Ok(Some(Sealed {
        kdf: Kdf {
            m: num("m")?,
            t: num("t")?,
            p: num("p")?,
        },
        salt: b64(&v["kdf"]["salt"])?,
        nonce: b64(&v["nonce"])?,
        data: b64(&v["data"])?,
    }))
}

impl FileStore {
    /// Open `path` (created on the first write) with `passphrase`. A wrong passphrase or a
    /// damaged file is an error here, not on the first read.
    pub fn open(path: &Path, passphrase: &SecretString) -> Result<Self, SecretError> {
        Self::open_with(path, passphrase, DEFAULT_KDF)
    }

    /// Like [`FileStore::open`]; `kdf` applies only when the file does not exist yet.
    pub fn open_with(
        path: &Path,
        passphrase: &SecretString,
        kdf: Kdf,
    ) -> Result<Self, SecretError> {
        if passphrase.expose_secret().is_empty() {
            return Err(bad("empty passphrase"));
        }
        let (salt, kdf) = if let Some(s) = read_sealed(path)? {
            (s.salt, s.kdf)
        } else {
            let mut salt = vec![0u8; 16];
            getrandom::fill(&mut salt).map_err(|e| bad(e.to_string()))?;
            (salt, kdf)
        };
        let store = Self {
            path: path.to_owned(),
            key: derive(passphrase, &salt, kdf)?,
            kdf,
            salt,
        };
        store.load()?;
        Ok(store)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&*self.key).into())
    }

    fn load(&self) -> Result<BTreeMap<String, SecretString>, SecretError> {
        let Some(s) = read_sealed(&self.path)? else {
            return Ok(BTreeMap::new());
        };
        if s.salt != self.salt || s.kdf != self.kdf || s.nonce.len() != 24 {
            return Err(bad("the file was re-created with another passphrase"));
        }
        let plain = Zeroizing::new(
            self.cipher()
                .decrypt(
                    XNonce::from_slice(&s.nonce),
                    Payload {
                        msg: &s.data,
                        aad: aad(s.kdf, &B64.encode(&s.salt)).as_bytes(),
                    },
                )
                .map_err(|_| bad("wrong passphrase or damaged file"))?,
        );
        let map: BTreeMap<String, String> =
            serde_json::from_slice(&plain).map_err(|_| bad("damaged file"))?;
        Ok(map
            .into_iter()
            .map(|(k, v)| (k, SecretString::from(v)))
            .collect())
    }

    fn save(&self, map: &BTreeMap<String, SecretString>) -> Result<(), SecretError> {
        let plain: BTreeMap<&str, &str> = map
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose_secret()))
            .collect();
        let plain = Zeroizing::new(serde_json::to_vec(&plain).map_err(|e| bad(e.to_string()))?);
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|e| bad(e.to_string()))?;
        let salt = B64.encode(&self.salt);
        let data = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad(self.kdf, &salt).as_bytes(),
                },
            )
            .map_err(|_| bad("encryption failed"))?;
        let doc = json!({
            "v": 1,
            "kdf": { "alg": "argon2id", "m": self.kdf.m, "t": self.kdf.t, "p": self.kdf.p, "salt": salt },
            "nonce": B64.encode(nonce),
            "data": B64.encode(data),
        });
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| bad(e.to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, doc.to_string()).map_err(|e| bad(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| bad(e.to_string()))
    }
}

impl SecretStore for FileStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        Ok(self.load()?.remove(account))
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), SecretError> {
        let mut map = self.load()?;
        map.insert(account.to_owned(), secret.clone());
        self.save(&map)
    }

    fn delete(&self, account: &str) -> Result<bool, SecretError> {
        let mut map = self.load()?;
        let had = map.remove(account).is_some();
        if had {
            self.save(&map)?;
        }
        Ok(had)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault;

    const FAST: Kdf = Kdf { m: 8, t: 1, p: 1 };

    fn pass(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn round_trip_wrong_passphrase_and_tamper() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("vault.enc");
        let store = FileStore::open_with(&p, &pass("correct horse"), FAST).unwrap();
        assert!(!p.exists(), "nothing is written before the first secret");
        vault::add(&store, "DATABASE_URL", &pass("postgres://u:p4ssw0rd@h/db")).unwrap();
        vault::add(&store, "API_TOKEN", &pass("t0ken-value")).unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            !text.contains("p4ssw0rd") && !text.contains("DATABASE_URL"),
            "{text}"
        );

        let again = FileStore::open(&p, &pass("correct horse")).unwrap();
        assert_eq!(vault::names(&again).unwrap(), ["API_TOKEN", "DATABASE_URL"]);
        assert_eq!(
            again
                .get("vault:API_TOKEN")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "t0ken-value"
        );
        assert!(vault::remove(&again, "API_TOKEN").unwrap());
        assert_eq!(vault::names(&store).unwrap(), ["DATABASE_URL"]);

        assert!(matches!(
            FileStore::open(&p, &pass("wrong")),
            Err(SecretError::VaultFile(_))
        ));
        // Lowering the KDF cost in the header breaks the associated data.
        let tampered = std::fs::read_to_string(&p)
            .unwrap()
            .replace("\"t\":1", "\"t\":2");
        std::fs::write(&p, tampered).unwrap();
        assert!(FileStore::open(&p, &pass("correct horse")).is_err());
        assert!(FileStore::open_with(&dir.path().join("x"), &pass(""), FAST).is_err());
    }
}
