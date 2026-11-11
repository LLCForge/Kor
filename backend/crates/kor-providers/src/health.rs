//! Per-provider health (Master Build Prompt §4.5): latency, error rate and the remaining quota
//! the provider reports in its rate-limit headers. Kept in memory for the process; `save` and
//! `load` persist a small JSON file so `kor provider status` can show history across runs.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// Outcomes kept per provider for the error rate.
const WINDOW: usize = 50;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    pub remaining_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub limit_requests: Option<u64>,
    pub limit_tokens: Option<u64>,
    /// Raw reset hint as the provider sent it (seconds or RFC 3339).
    pub reset: Option<String>,
}

impl Quota {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Most recent outcomes, `true` = success; at most `WINDOW`.
    pub recent: Vec<bool>,
    /// Latencies (ms) of the most recent successful calls, at most `WINDOW`.
    pub latencies_ms: Vec<u64>,
    pub last_error: Option<String>,
    pub quota: Quota,
    /// Unix seconds of the last update.
    pub updated_at: u64,
}

impl Stats {
    /// Share of failed calls among the recent ones, 0.0 when nothing was recorded.
    pub fn error_rate(&self) -> f64 {
        if self.recent.is_empty() {
            return 0.0;
        }
        let failed = self.recent.iter().filter(|ok| !**ok).count();
        // Both counts are at most WINDOW, so the conversion is exact.
        f64::from(u32::try_from(failed).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(self.recent.len()).unwrap_or(u32::MAX))
    }

    /// Median latency of recent successes.
    pub fn latency_ms(&self) -> Option<u64> {
        let mut v = self.latencies_ms.clone();
        v.sort_unstable();
        v.get(v.len() / 2).copied()
    }
}

static STATS: Mutex<BTreeMap<String, Stats>> = Mutex::new(BTreeMap::new());
/// File every [`record`] persists to, once [`set_store`] was called.
static STORE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Persist stats to `path` from now on (after merging what it already holds).
pub fn set_store(path: PathBuf) {
    let mut s = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    if s.as_ref() != Some(&path) {
        load(&path);
        *s = Some(path);
    }
}

fn persist() {
    let s = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(p) = s.as_ref() {
        // Best effort: health history is a convenience, never a reason to fail a call.
        let _ = save(p);
    }
}

fn with<T>(f: impl FnOnce(&mut BTreeMap<String, Stats>) -> T) -> T {
    f(&mut STATS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn push<T>(v: &mut Vec<T>, x: T) {
    v.push(x);
    if v.len() > WINDOW {
        v.remove(0);
    }
}

/// Record one call: `Ok(latency_ms)` or `Err(message)` (already redacted).
pub fn record(provider: &str, outcome: Result<u64, &str>) {
    with(|m| {
        let s = m.entry(provider.to_owned()).or_default();
        match outcome {
            Ok(ms) => {
                push(&mut s.recent, true);
                push(&mut s.latencies_ms, ms);
            }
            Err(e) => {
                push(&mut s.recent, false);
                s.last_error = Some(e.chars().take(300).collect());
            }
        }
        s.updated_at = crate::catalog::now();
    });
    persist();
}

/// Rate-limit headers of any response (`x-ratelimit-*`, `anthropic-ratelimit-*`).
pub(crate) fn record_headers(provider: &str, h: &HeaderMap) {
    let q = quota_from_headers(h);
    if q.is_empty() {
        return;
    }
    with(|m| {
        let s = m.entry(provider.to_owned()).or_default();
        s.quota = q;
        s.updated_at = crate::catalog::now();
    });
}

pub fn quota_from_headers(h: &HeaderMap) -> Quota {
    let num = |names: &[&str]| {
        names.iter().find_map(|n| {
            h.get(*n)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
        })
    };
    let text = |names: &[&str]| {
        names.iter().find_map(|n| {
            h.get(*n)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.chars().take(64).collect::<String>())
        })
    };
    Quota {
        remaining_requests: num(&[
            "x-ratelimit-remaining-requests",
            "anthropic-ratelimit-requests-remaining",
            "x-ratelimit-remaining",
        ]),
        remaining_tokens: num(&[
            "x-ratelimit-remaining-tokens",
            "anthropic-ratelimit-tokens-remaining",
        ]),
        limit_requests: num(&[
            "x-ratelimit-limit-requests",
            "anthropic-ratelimit-requests-limit",
            "x-ratelimit-limit",
        ]),
        limit_tokens: num(&[
            "x-ratelimit-limit-tokens",
            "anthropic-ratelimit-tokens-limit",
        ]),
        reset: text(&[
            "x-ratelimit-reset-requests",
            "anthropic-ratelimit-requests-reset",
            "x-ratelimit-reset",
        ]),
    }
}

pub fn get(provider: &str) -> Option<Stats> {
    with(|m| m.get(provider).cloned())
}

pub fn snapshot() -> BTreeMap<String, Stats> {
    with(|m| m.clone())
}

/// Merge a saved file into memory; entries already in memory (this run) win.
/// A missing or corrupt file is ignored: it is only history.
pub fn load(path: &Path) {
    let mut text = String::new();
    if fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES).read_to_string(&mut text))
        .is_err()
    {
        return;
    }
    let Ok(saved) = serde_json::from_str::<BTreeMap<String, Stats>>(&text) else {
        return;
    };
    with(|m| {
        for (k, v) in saved {
            m.entry(k).or_insert(v);
        }
    });
}

/// Write the in-memory stats atomically (temp file + rename).
pub fn save(path: &Path) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&snapshot())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn error_rate_latency_and_window() {
        let id = "health-test-a";
        for i in 0..60 {
            record(id, Ok(i));
        }
        record(id, Err("boom"));
        let s = get(id).unwrap();
        assert_eq!(s.recent.len(), WINDOW);
        assert!((s.error_rate() - 1.0 / 50.0).abs() < 1e-9);
        assert_eq!(s.latency_ms(), Some(35));
        assert_eq!(s.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn quota_headers_openai_and_anthropic() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-ratelimit-remaining-requests",
            HeaderValue::from_static("42"),
        );
        h.insert("x-ratelimit-limit-tokens", HeaderValue::from_static("1000"));
        let q = quota_from_headers(&h);
        assert_eq!(
            (q.remaining_requests, q.limit_tokens),
            (Some(42), Some(1000))
        );
        let mut h = HeaderMap::new();
        h.insert(
            "anthropic-ratelimit-tokens-remaining",
            HeaderValue::from_static("9000"),
        );
        assert_eq!(quota_from_headers(&h).remaining_tokens, Some(9000));
        assert!(quota_from_headers(&HeaderMap::new()).is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h.json");
        record("health-test-b", Ok(7));
        save(&p).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("health-test-b"));
        load(&p);
        load(&dir.path().join("missing.json"));
    }
}
