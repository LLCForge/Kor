//! Token usage ledger shared by the CLI and the desktop (`kor cost`, the desktop cost screens).
//!
//! One JSON line per finished turn in `<kor home>/usage.jsonl`: time, `provider/model`, prompt and
//! completion tokens as the provider reported them. Nothing is estimated here; prices are applied
//! by the reader where a price is actually known.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Lines kept when the ledger is read (about a year of heavy use).
const MAX_LINES: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix milliseconds.
    pub at: u64,
    pub model: String,
    pub prompt: u64,
    pub completion: u64,
    /// Cost the provider itself reported for this call (`OpenRouter` `usage.cost`), in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// File the session touched last before this call (absolute), for the per-package
    /// breakdown (`kor packages cost`, spec §15.15). Local only, like the rest of the ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Who spent it in a multi-agent task (`kor task`, §8.2): `task:<id>/<role>`,
    /// `task:<id>/orchestrator`, `task:<id>/reviewer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// A paid image, video or audio generation, not a model call (`kor media`, ADR 0060):
    /// the cost dashboard shows media spending on its own line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub media: bool,
    /// `cost_usd` was estimated from list prices (shown with "≈").
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
}

pub fn file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("usage.jsonl"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Append one turn. Zero-token turns are skipped.
///
/// # Errors
/// If the ledger cannot be written.
pub fn record(path: &Path, model: &str, prompt: u64, completion: u64) -> std::io::Result<()> {
    record_call(path, model, prompt, completion, None)
}

/// [`record`] with the cost the provider reported, when it did.
///
/// # Errors
/// If the ledger cannot be written.
pub fn record_call(
    path: &Path,
    model: &str,
    prompt: u64,
    completion: u64,
    cost_usd: Option<f64>,
) -> std::io::Result<()> {
    append(
        path,
        &Entry {
            at: now_ms(),
            model: model.to_owned(),
            prompt,
            completion,
            cost_usd: cost_usd.filter(|c| c.is_finite() && *c >= 0.0),
            path: None,
            role: None,
            media: false,
            estimated: false,
        },
    )
}

/// Append one entry as given. Zero-token entries without a cost are skipped.
///
/// # Errors
/// If the ledger cannot be written.
pub fn append(path: &Path, e: &Entry) -> std::io::Result<()> {
    if e.prompt == 0 && e.completion == 0 && e.cost_usd.is_none() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(e).map_err(std::io::Error::other)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

/// [`record`] into the default ledger; errors are ignored (usage must never break a turn).
pub fn record_default(model: &str, prompt: u64, completion: u64) {
    if let Some(p) = file() {
        let _ = record(&p, model, prompt, completion);
    }
}

pub fn read(path: &Path) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(MAX_LINES);
    lines[skip..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Per model since `since_ms`: (prompt tokens, completion tokens, turns).
pub fn by_model(entries: &[Entry], since_ms: u64) -> BTreeMap<String, (u64, u64, u64)> {
    let mut out: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.at >= since_ms) {
        let s = out.entry(e.model.clone()).or_default();
        s.0 += e.prompt;
        s.1 += e.completion;
        s.2 += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_groups_by_model() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("usage.jsonl");
        record(&p, "venice/a", 100, 10).unwrap();
        record(&p, "venice/a", 50, 5).unwrap();
        record(&p, "local/b", 0, 0).unwrap();
        record(&p, "local/b", 7, 1).unwrap();
        let all = read(&p);
        assert_eq!(all.len(), 3, "zero-token turn skipped");
        let g = by_model(&all, 0);
        assert_eq!(g["venice/a"], (150, 15, 2));
        assert_eq!(g["local/b"], (7, 1, 1));
        assert!(by_model(&all, u64::MAX).is_empty());
    }

    #[test]
    fn reads_media_lines_written_by_the_gallery() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("usage.jsonl");
        kor_agent::gallery::charge(Some(&p), "fal/fal-ai/x", Some(&(0.4, "USD".into(), true)));
        let all = read(&p);
        assert_eq!(all.len(), 1);
        assert!(all[0].media && all[0].estimated);
        assert_eq!(all[0].cost_usd, Some(0.4));
    }
}
