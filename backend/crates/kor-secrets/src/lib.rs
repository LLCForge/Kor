//! API key storage and output redaction.
//!
//! Lookup order for a provider key: environment variable, then OS keychain.
//! Keys are never written to config files. Secrets live in `SecretString`,
//! which zeroizes on drop and never prints its contents in `Debug`.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError, RwLock};

use regex::Regex;
pub use secrecy::{ExposeSecret, SecretString};

pub mod filestore;
pub mod vault;

/// Keychain service name under which every Kor entry is stored.
pub const SERVICE: &str = "kor";
/// Upper bound on accepted key length; real provider keys are far shorter.
pub const MAX_SECRET_LEN: usize = 8192;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("invalid provider id `{0}`")]
    InvalidProvider(String),
    #[error("secret is empty or longer than {MAX_SECRET_LEN} bytes")]
    InvalidSecret,
    #[error("invalid secret name `{0}` (letters, digits and _, not starting with a digit)")]
    InvalidName(String),
    #[error("OS keychain error: {0}")]
    Keychain(String),
    /// The encrypted vault file (`[vault] backend = "file"`): wrong passphrase, damaged or
    /// unreadable file.
    #[error("vault file: {0}")]
    VaultFile(String),
}

/// A place secrets can be stored. Implementations must not log secret values.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError>;
    fn set(&self, account: &str, secret: &SecretString) -> Result<(), SecretError>;
    /// Returns whether an entry existed.
    fn delete(&self, account: &str) -> Result<bool, SecretError>;
}

/// The platform credential store: Keychain, Credential Manager or Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainStore;

impl KeychainStore {
    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, account).map_err(|e| SecretError::Keychain(e.to_string()))
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        match Self::entry(account)?.get_password() {
            Ok(p) => Ok(Some(SecretString::from(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), SecretError> {
        Self::entry(account)?
            .set_password(secret.expose_secret())
            .map_err(|e| SecretError::Keychain(e.to_string()))
    }

    fn delete(&self, account: &str) -> Result<bool, SecretError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }
}

/// In-memory store for tests and headless fallbacks.
#[derive(Default)]
pub struct MemoryStore(Mutex<Vec<(String, SecretString)>>);

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryStore(..)")
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        let v = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(v.iter().find(|(a, _)| a == account).map(|(_, s)| s.clone()))
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), SecretError> {
        let mut v = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        v.retain(|(a, _)| a != account);
        v.push((account.to_owned(), secret.clone()));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<bool, SecretError> {
        let mut v = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let before = v.len();
        v.retain(|(a, _)| a != account);
        Ok(v.len() != before)
    }
}

/// Keychain account name for a provider's API key.
pub fn account_for(provider: &str) -> Result<String, SecretError> {
    if kor_core::is_valid_provider_id(provider) {
        Ok(format!("provider:{provider}"))
    } else {
        Err(SecretError::InvalidProvider(provider.to_owned()))
    }
}

/// Default env var for a provider: `open-router` → `OPEN_ROUTER_API_KEY`.
/// Provider manifests may override this (Phase 1).
pub fn env_var_for(provider: &str) -> String {
    format!(
        "{}_API_KEY",
        provider.to_ascii_uppercase().replace('-', "_")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Env(String),
    Keychain,
}

#[derive(Debug)]
pub struct ResolvedKey {
    pub secret: SecretString,
    pub source: KeySource,
}

/// Find a provider key: `env_var` first, then `store`. Empty env values are ignored.
pub fn resolve_key(
    provider: &str,
    env_var: &str,
    env: impl Fn(&str) -> Option<String>,
    store: &dyn SecretStore,
) -> Result<Option<ResolvedKey>, SecretError> {
    let account = account_for(provider)?;
    if let Some(v) = env(env_var).filter(|v| !v.trim().is_empty()) {
        return Ok(Some(ResolvedKey {
            secret: SecretString::from(v),
            source: KeySource::Env(env_var.to_owned()),
        }));
    }
    Ok(store.get(&account)?.map(|secret| ResolvedKey {
        secret,
        source: KeySource::Keychain,
    }))
}

/// Store a provider key after validating it.
pub fn store_key(
    provider: &str,
    secret: &SecretString,
    store: &dyn SecretStore,
) -> Result<(), SecretError> {
    let s = secret.expose_secret();
    if s.is_empty() || s.len() > MAX_SECRET_LEN || s.chars().any(char::is_control) {
        return Err(SecretError::InvalidSecret);
    }
    store.set(&account_for(provider)?, secret)
}

/// Keys per provider in the failover pool (keychain `provider:<id>`, `provider:<id>#2`…).
pub const MAX_POOL_KEYS: usize = 8;

/// Keychain account of pool slot `slot` (1-based; slot 1 is the main key).
pub fn pool_account(provider: &str, slot: usize) -> Result<String, SecretError> {
    let main = account_for(provider)?;
    Ok(if slot <= 1 {
        main
    } else {
        format!("{main}#{slot}")
    })
}

/// Every key of a provider, in failover order: the env var, then pool slots 1..=8.
/// Duplicate values are dropped.
pub fn resolve_keys(
    provider: &str,
    env_var: &str,
    env: impl Fn(&str) -> Option<String>,
    store: &dyn SecretStore,
) -> Result<Vec<ResolvedKey>, SecretError> {
    let mut out: Vec<ResolvedKey> = Vec::new();
    if let Some(v) = env(env_var).filter(|v| !v.trim().is_empty()) {
        out.push(ResolvedKey {
            secret: SecretString::from(v),
            source: KeySource::Env(env_var.to_owned()),
        });
    }
    for slot in 1..=MAX_POOL_KEYS {
        if let Some(secret) = store.get(&pool_account(provider, slot)?)?
            && !out
                .iter()
                .any(|k| k.secret.expose_secret() == secret.expose_secret())
        {
            out.push(ResolvedKey {
                secret,
                source: KeySource::Keychain,
            });
        }
    }
    Ok(out)
}

/// Keychain slots that hold a key, 1-based.
pub fn pool_slots(provider: &str, store: &dyn SecretStore) -> Result<Vec<usize>, SecretError> {
    let mut out = Vec::new();
    for slot in 1..=MAX_POOL_KEYS {
        if store.get(&pool_account(provider, slot)?)?.is_some() {
            out.push(slot);
        }
    }
    Ok(out)
}

/// Put a key in the first free pool slot and return it; `None` when all slots are used.
pub fn add_pool_key(
    provider: &str,
    secret: &SecretString,
    store: &dyn SecretStore,
) -> Result<Option<usize>, SecretError> {
    let s = secret.expose_secret();
    if s.is_empty() || s.len() > MAX_SECRET_LEN || s.chars().any(char::is_control) {
        return Err(SecretError::InvalidSecret);
    }
    let used = pool_slots(provider, store)?;
    let Some(slot) = (1..=MAX_POOL_KEYS).find(|s| !used.contains(s)) else {
        return Ok(None);
    };
    store.set(&pool_account(provider, slot)?, secret)?;
    Ok(Some(slot))
}

/// `sk-ant-abc...xyz9` → `sk-a…xyz9`. Short secrets are fully masked.
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() < 12 {
        return "•".repeat(chars.len().max(4));
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

const REDACTED: &str = "[REDACTED]";

/// Replaces secrets in text bound for logs, transcripts or tool output.
#[derive(Debug, Clone)]
pub struct Redactor {
    patterns: Vec<Regex>,
    candidate: Regex,
    literals: Vec<SecretString>,
}

impl Default for Redactor {
    fn default() -> Self {
        // Well-known key formats; order does not matter.
        let patterns = [
            r"sk-ant-[A-Za-z0-9_\-]{20,}",
            r"sk-(?:proj-|or-v1-)?[A-Za-z0-9_\-]{20,}",
            r"AIza[0-9A-Za-z_\-]{35}",
            r"xai-[A-Za-z0-9]{20,}",
            r"hf_[A-Za-z0-9]{30,}",
            r"gh[pousr]_[A-Za-z0-9]{36,}",
            r"github_pat_[A-Za-z0-9_]{40,}",
            r"(?:AKIA|ASIA)[0-9A-Z]{16}",
            r"nvapi-[A-Za-z0-9_\-]{20,}",
            r"pplx-[A-Za-z0-9]{20,}",
            r"(?i)bearer\s+[A-Za-z0-9._~+/\-]{16,}=*",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static pattern"))
        .collect();
        Self {
            patterns,
            candidate: Regex::new(r"[A-Za-z0-9_\-+/=]{32,}").expect("static pattern"),
            literals: Vec::new(),
        }
    }
}

/// `scheme://user:password@host` (spec §6.6 `postgres://user:pass@`): only the password goes.
static URL_CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([a-z][a-z0-9+.\-]*://[^\s:/@]+:)([^\s@/]+)@").expect("static pattern")
});

impl Redactor {
    /// Also redact this exact value wherever it appears (e.g. a key loaded at startup).
    /// Values shorter than 8 bytes are ignored: masking them would mangle ordinary text.
    pub fn register(&mut self, secret: SecretString) {
        let s = secret.expose_secret();
        if s.len() >= 8 && !self.literals.iter().any(|l| l.expose_secret() == s) {
            self.literals.push(secret);
            // Longest first, so a value that contains another one is masked whole.
            self.literals
                .sort_by_key(|l| std::cmp::Reverse(l.expose_secret().len()));
        }
    }

    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.apply(text, true).0
    }

    /// Mask every value registered here in model-bound content too ([`mask_outbound`]).
    pub fn share_outbound(&self) {
        for l in &self.literals {
            register_outbound(l.clone());
        }
    }

    /// For content bound for a model: registered values, known key formats and URL passwords,
    /// with how many were masked. The entropy guess is left out: tool-call ids, reasoning
    /// signatures and hashes must reach the provider unchanged.
    pub fn redact_outbound<'a>(&self, text: &'a str) -> (Cow<'a, str>, usize) {
        self.apply(text, false)
    }

    fn apply<'a>(&self, text: &'a str, guess: bool) -> (Cow<'a, str>, usize) {
        let mut out = Cow::Borrowed(text);
        let mut n = 0;
        for lit in &self.literals {
            let s = lit.expose_secret();
            if out.contains(s) {
                n += out.matches(s).count();
                out = Cow::Owned(out.replace(s, REDACTED));
            }
        }
        for re in &self.patterns {
            if re.is_match(&out) {
                n += re.find_iter(&out).count();
                out = Cow::Owned(re.replace_all(&out, REDACTED).into_owned());
            }
        }
        if URL_CREDENTIALS.is_match(&out) {
            let replaced = URL_CREDENTIALS.replace_all(&out, |c: &regex::Captures<'_>| {
                if &c[2] == REDACTED {
                    c[0].to_owned()
                } else {
                    n += 1;
                    format!("{}{REDACTED}@", &c[1])
                }
            });
            if let Cow::Owned(s) = replaced {
                out = Cow::Owned(s);
            }
        }
        if guess && self.candidate.is_match(&out) {
            let replaced = self.candidate.replace_all(&out, |c: &regex::Captures<'_>| {
                if looks_random(&c[0]) {
                    n += 1;
                    REDACTED.to_owned()
                } else {
                    c[0].to_owned()
                }
            });
            if let Cow::Owned(s) = replaced
                && s != *out
            {
                out = Cow::Owned(s);
            }
        }
        (out, n)
    }
}

/// Process-wide masker for everything sent to a model (§6.6). The provider request builder
/// (`kor_providers::http`) is the one choke point that calls [`mask_outbound`]; the runtime
/// registers provider keys and every vault value here at session start.
static OUTBOUND: LazyLock<RwLock<Redactor>> = LazyLock::new(|| RwLock::new(Redactor::default()));
static MASKED: AtomicU64 = AtomicU64::new(0);

/// Mask this exact value in all model-bound content from now on.
pub fn register_outbound(secret: SecretString) {
    OUTBOUND
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .register(secret);
}

/// `text` with secrets masked, for a model request.
pub fn mask_outbound(text: &str) -> Cow<'_, str> {
    let (out, n) = OUTBOUND
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .redact_outbound(text);
    if n > 0 {
        MASKED.fetch_add(n as u64, Ordering::Relaxed);
    }
    out
}

/// Secrets masked in model-bound content since the process started (for the audit log).
pub fn outbound_masked_total() -> u64 {
    MASKED.load(Ordering::Relaxed)
}

/// High Shannon entropy plus mixed character classes. Hex digests (git SHAs,
/// checksums) top out at 4 bits/char and are deliberately left alone.
fn looks_random(s: &str) -> bool {
    let has_upper = s.bytes().any(|b| b.is_ascii_uppercase());
    let has_lower = s.bytes().any(|b| b.is_ascii_lowercase());
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    if !(has_upper && has_lower && has_digit) {
        return false;
    }
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[usize::from(b)] += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    let len = s.len() as f64;
    let entropy: f64 = counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / len;
            -p * p.log2()
        })
        .sum();
    entropy > 4.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_wins_over_store_and_empty_env_is_ignored() {
        let store = MemoryStore::default();
        store_key("openai", &SecretString::from("from-store-123"), &store).unwrap();

        let r = resolve_key(
            "openai",
            "OPENAI_API_KEY",
            |_| Some("from-env".into()),
            &store,
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.source, KeySource::Env("OPENAI_API_KEY".into()));
        assert_eq!(r.secret.expose_secret(), "from-env");

        let r = resolve_key("openai", "OPENAI_API_KEY", |_| Some("  ".into()), &store)
            .unwrap()
            .unwrap();
        assert_eq!(r.source, KeySource::Keychain);

        assert!(store.delete(&account_for("openai").unwrap()).unwrap());
        assert!(
            resolve_key("openai", "X", |_| None, &store)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn key_pool_slots_and_order() {
        let store = MemoryStore::default();
        assert_eq!(pool_account("groq", 1).unwrap(), "provider:groq");
        assert_eq!(pool_account("groq", 3).unwrap(), "provider:groq#3");
        let k = |s: &str| SecretString::from(s.to_owned());
        assert_eq!(add_pool_key("groq", &k("k1"), &store).unwrap(), Some(1));
        assert_eq!(add_pool_key("groq", &k("k2"), &store).unwrap(), Some(2));
        assert_eq!(add_pool_key("groq", &k("k3"), &store).unwrap(), Some(3));
        store.delete(&pool_account("groq", 2).unwrap()).unwrap();
        // A freed slot is reused.
        assert_eq!(add_pool_key("groq", &k("k4"), &store).unwrap(), Some(2));
        let keys = resolve_keys("groq", "GROQ_API_KEY", |_| Some("k3".into()), &store).unwrap();
        let got: Vec<&str> = keys.iter().map(|r| r.secret.expose_secret()).collect();
        // Env first; the keychain copy of the same value is not repeated.
        assert_eq!(got, ["k3", "k1", "k4"]);
        for i in 4..=MAX_POOL_KEYS {
            add_pool_key("groq", &k(&format!("x{i}")), &store).unwrap();
        }
        assert_eq!(add_pool_key("groq", &k("full"), &store).unwrap(), None);
        assert!(add_pool_key("groq", &k(""), &store).is_err());
    }

    #[test]
    fn rejects_bad_ids_and_secrets() {
        let store = MemoryStore::default();
        assert!(matches!(
            store_key("../x", &SecretString::from("abc"), &store),
            Err(SecretError::InvalidProvider(_))
        ));
        for bad in ["", "a\nb"] {
            assert!(matches!(
                store_key("x", &SecretString::from(bad), &store),
                Err(SecretError::InvalidSecret)
            ));
        }
        let long = "a".repeat(MAX_SECRET_LEN + 1);
        assert!(store_key("x", &SecretString::from(long), &store).is_err());
    }

    #[test]
    fn env_var_names() {
        assert_eq!(env_var_for("openai"), "OPENAI_API_KEY");
        assert_eq!(env_var_for("open-router"), "OPEN_ROUTER_API_KEY");
    }

    #[test]
    fn masking() {
        assert_eq!(mask("sk-ant-api03-abcdefghijklmnop"), "sk-a…mnop");
        assert_eq!(mask("short"), "•••••");
        assert_eq!(mask(""), "••••");
    }

    #[test]
    fn redacts_known_formats_literals_and_random_tokens() {
        let mut r = Redactor::default();
        r.register(SecretString::from("my-custom-secret-value"));
        let input = "a=sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv b=my-custom-secret-value \
                     c=AKIAABCDEFGHIJKLMNOP auth: Bearer abcdefghijklmnop1234 \
                     d=Zx8Qw2Lm9Rt4Yu7Io1Pa5Sd3Fg6Hj0Kl";
        let out = r.redact(input);
        assert!(!out.contains("sk-ant"), "{out}");
        assert!(!out.contains("my-custom"), "{out}");
        assert!(!out.contains("AKIA"), "{out}");
        assert!(!out.contains("abcdefghijklmnop1234"), "{out}");
        assert!(!out.contains("Zx8Qw2"), "{out}");
    }

    #[test]
    fn outbound_masks_values_formats_and_url_passwords_but_keeps_ids() {
        let mut r = Redactor::default();
        r.register(SecretString::from("vault-value-1234"));
        r.register(SecretString::from("vault-value-1234-longer"));
        let input = "url=postgres://admin:hunter2pw@db.local:5432/app \
                     key=sk-proj-AbCdEfGhIjKlMnOpQrStUv v=vault-value-1234-longer \
                     id=call_Zx8Qw2Lm9Rt4Yu7Io1Pa5Sd3Fg6Hj0Kl";
        let (out, n) = r.redact_outbound(input);
        assert_eq!(n, 3, "{out}");
        assert!(
            out.contains("postgres://admin:[REDACTED]@db.local"),
            "{out}"
        );
        assert!(
            !out.contains("sk-proj") && !out.contains("vault-value"),
            "{out}"
        );
        assert!(
            !out.contains("-longer"),
            "the longer value is masked whole: {out}"
        );
        // No entropy guess outbound: ids and signatures round-trip.
        assert!(
            out.contains("call_Zx8Qw2Lm9Rt4Yu7Io1Pa5Sd3Fg6Hj0Kl"),
            "{out}"
        );
        // Already masked passwords are not counted twice.
        let (again, n) = r.redact_outbound(&out);
        assert_eq!((again.as_ref(), n), (out.as_ref(), 0));
        // A URL without a password is left alone.
        assert!(matches!(
            r.redact_outbound("see https://example.com/a:b@c").0,
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn global_outbound_masker_counts() {
        register_outbound(SecretString::from("global-vault-secret-xyz"));
        let before = outbound_masked_total();
        assert_eq!(
            mask_outbound("a global-vault-secret-xyz b"),
            "a [REDACTED] b"
        );
        assert!(outbound_masked_total() > before);
    }

    #[test]
    fn leaves_ordinary_text_and_hashes_alone() {
        let r = Redactor::default();
        let text = "commit 3f786850e387550fdab836ed7e6dc881de23001b in src/very_long_module_name_for_testing.rs";
        assert!(matches!(r.redact(text), Cow::Borrowed(_)));
    }

    #[test]
    fn secret_debug_does_not_leak() {
        let s = SecretString::from("hunter2-super-secret");
        assert!(!format!("{s:?}").contains("hunter2"));
    }
}
