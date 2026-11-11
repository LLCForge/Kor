//! Per-key quota (Master Build Prompt §15.10): the rate-limit headers of every response
//! (`x-ratelimit-*`, `anthropic-ratelimit-*`), kept per provider and key so `kor quota` can show
//! what is left on each one. Keys are stored only as their last four characters.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use kor_secrets::{ExposeSecret, SecretString};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::health::{Quota, quota_from_headers};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Used share at which a key counts as nearly exhausted.
pub const LOW_SHARE: f64 = 0.9;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyQuota {
    pub provider: String,
    /// `…abcd`; empty for a keyless provider.
    pub key: String,
    pub quota: Quota,
    /// Unix seconds the headers were seen.
    pub at: u64,
}

impl KeyQuota {
    /// Largest used share of the request or token limit, when the provider sent both numbers.
    pub fn used_share(&self) -> Option<f64> {
        let q = &self.quota;
        let share = |rem: Option<u64>, lim: Option<u64>| match (rem, lim) {
            #[allow(clippy::cast_precision_loss)] // a ratio for display and a threshold
            (Some(r), Some(l)) if l > 0 => Some(1.0 - (r.min(l) as f64 / l as f64)),
            _ => None,
        };
        let a = share(q.remaining_requests, q.limit_requests);
        let b = share(q.remaining_tokens, q.limit_tokens);
        match (a, b) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// Nearly exhausted: at least [`LOW_SHARE`] used, or nothing left.
    pub fn low(&self) -> bool {
        self.quota.remaining_requests == Some(0)
            || self.quota.remaining_tokens == Some(0)
            || self.used_share().is_some_and(|s| s >= LOW_SHARE)
    }

    /// Unix seconds the limit resets, when the hint is a number of seconds or a duration like
    /// `6m0s` (`OpenAI`, Groq). Other forms (RFC 3339 from Anthropic) stay in `quota.reset`.
    pub fn reset_at(&self) -> Option<u64> {
        let secs = parse_duration(self.quota.reset.as_deref()?)?;
        // Non-negative and finite (checked by the parser); a year is plenty for a reset.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let secs = secs.ceil().min(31_536_000.0) as u64;
        Some(self.at.saturating_add(secs))
    }
}

/// `20`, `1.5`, `6m0s`, `1h2m3.5s`, `250ms` → seconds.
fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<f64>() {
        return (n.is_finite() && n >= 0.0).then_some(n);
    }
    let (mut total, mut num) = (0.0, String::new());
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        let unit = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += n * unit;
        any = true;
    }
    (any && num.is_empty()).then_some(total)
}

static MAP: Mutex<BTreeMap<String, KeyQuota>> = Mutex::new(BTreeMap::new());
static STORE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Persist quotas to `path` from now on (after merging what it already holds).
pub fn set_store(path: PathBuf) {
    let mut s = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    if s.as_ref() != Some(&path) {
        load(&path);
        *s = Some(path);
    }
}

fn hint(key: Option<&SecretString>) -> String {
    let Some(k) = key else {
        return String::new();
    };
    let chars: Vec<char> = k.expose_secret().chars().collect();
    if chars.len() < 12 {
        return "••••".into();
    }
    format!("…{}", chars[chars.len() - 4..].iter().collect::<String>())
}

/// Rate-limit headers of one response from `provider` called with `key`.
pub(crate) fn record(provider: &str, key: Option<&SecretString>, h: &HeaderMap) {
    let quota = quota_from_headers(h);
    if quota.is_empty() {
        return;
    }
    let key = hint(key);
    let entry = KeyQuota {
        provider: provider.to_owned(),
        key: key.clone(),
        quota,
        at: crate::catalog::now(),
    };
    MAP.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(format!("{provider} {key}"), entry);
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(p) = store.as_ref() {
        // Best effort: quota history is a convenience, never a reason to fail a call.
        let _ = save(p);
    }
}

/// Every key seen, sorted by provider then key.
pub fn snapshot() -> Vec<KeyQuota> {
    MAP.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// Merge a saved file into memory; newer entries win. A missing or corrupt file is ignored.
pub fn load(path: &Path) {
    let mut text = String::new();
    if fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES).read_to_string(&mut text))
        .is_err()
    {
        return;
    }
    let Ok(saved) = serde_json::from_str::<BTreeMap<String, KeyQuota>>(&text) else {
        return;
    };
    let mut m = MAP.lock().unwrap_or_else(PoisonError::into_inner);
    for (k, v) in saved {
        if m.get(&k).is_none_or(|cur| cur.at < v.at) {
            m.insert(k, v);
        }
    }
}

fn save(path: &Path) -> std::io::Result<()> {
    let snap = MAP.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&snap)?)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn durations_and_reset() {
        assert_eq!(parse_duration("20"), Some(20.0));
        assert_eq!(parse_duration("6m0s"), Some(360.0));
        assert_eq!(parse_duration("1h2m3.5s"), Some(3723.5));
        assert_eq!(parse_duration("250ms"), Some(0.25));
        assert_eq!(parse_duration("2024-01-01T00:00:00Z"), None);
        assert_eq!(parse_duration("5x"), None);
        let k = KeyQuota {
            quota: Quota {
                reset: Some("1m".into()),
                ..Quota::default()
            },
            at: 100,
            ..KeyQuota::default()
        };
        assert_eq!(k.reset_at(), Some(160));
    }

    #[test]
    fn per_key_store_masks_keys_and_flags_low_quota() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-ratelimit-remaining-requests",
            HeaderValue::from_static("5"),
        );
        h.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("100"),
        );
        let key = SecretString::from("sk-test-quota-0000-wxyz".to_owned());
        record("quota-test", Some(&key), &h);
        record("quota-test", None, &HeaderMap::new());
        let rows: Vec<_> = snapshot()
            .into_iter()
            .filter(|k| k.provider == "quota-test")
            .collect();
        assert_eq!(rows.len(), 1, "empty headers are not stored");
        assert_eq!(rows[0].key, "…wxyz");
        assert!(rows[0].low());
        assert!((rows[0].used_share().unwrap() - 0.95).abs() < 1e-9);

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("q.json");
        save(&p).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("…wxyz") && !text.contains("sk-test"));
        load(&p);
    }
}
