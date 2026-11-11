//! Estimated finish time (§28.5): from the durations of earlier turns, never invented.
//!
//! The estimate is the first of the 50th / 75th / 90th percentile of similar past durations that
//! the running task has not passed yet: similar = the same project when it has at least
//! [`MIN_SAMPLES`] turns, otherwise all projects. Past the 90th percentile the task is "taking
//! longer than usual" (no time is shown). Every estimate is labelled `≈` with its method.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MIN_SAMPLES: usize = 3;
const MAX_SAMPLES: usize = 300;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sample {
    /// Project root (or trigger name for daemon jobs).
    pub key: String,
    pub secs: u64,
    pub at: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Same project (or trigger).
    #[default]
    Project,
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Eta {
    /// Unix ms; `None` once the task runs longer than the 90th percentile.
    pub finish_ms: Option<u64>,
    pub remaining_secs: Option<u64>,
    /// 50, 75 or 90.
    pub percentile: u8,
    pub scope: Scope,
    pub samples: usize,
    /// The task has passed the 90th percentile.
    pub overdue: bool,
}

/// `<presence>/durations.json` (desktop turn durations).
#[derive(Debug, Clone)]
pub struct History {
    path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    samples: Vec<Sample>,
}

impl History {
    pub fn standard() -> Option<Self> {
        super::dir().map(|d| Self::at(&d.join("durations.json")))
    }

    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    pub fn samples(&self) -> Vec<Sample> {
        super::read_json::<File>(&self.path).samples
    }

    /// Record one finished turn (shorter than a second is not a useful sample).
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn record(&self, key: &str, secs: u64) -> Result<(), String> {
        if secs == 0 {
            return Ok(());
        }
        super::edit_json(&self.path, |f: &mut File| {
            f.samples.push(Sample {
                key: key.to_owned(),
                secs,
                at: super::now_ms(),
            });
            let over = f.samples.len().saturating_sub(MAX_SAMPLES);
            f.samples.drain(..over);
        })
    }
}

/// Nearest-rank percentile of sorted values.
fn percentile(sorted: &[u64], p: u8) -> u64 {
    let n = sorted.len();
    let rank = (usize::from(p) * n).div_ceil(100).clamp(1, n);
    sorted[rank - 1]
}

/// Estimate for a task under `key` that started at `started_ms`.
pub fn estimate(samples: &[Sample], key: &str, started_ms: u64, now_ms: u64) -> Option<Eta> {
    let mut own: Vec<u64> = samples
        .iter()
        .filter(|s| s.key == key)
        .map(|s| s.secs)
        .collect();
    let (mut pool, scope) = if own.len() >= MIN_SAMPLES {
        (std::mem::take(&mut own), Scope::Project)
    } else {
        (samples.iter().map(|s| s.secs).collect(), Scope::All)
    };
    if pool.len() < MIN_SAMPLES {
        return None;
    }
    pool.sort_unstable();
    let elapsed = now_ms.saturating_sub(started_ms) / 1000;
    for p in [50u8, 75, 90] {
        let secs = percentile(&pool, p);
        if secs > elapsed {
            return Some(Eta {
                finish_ms: Some(started_ms + secs * 1000),
                remaining_secs: Some(secs - elapsed),
                percentile: p,
                scope,
                samples: pool.len(),
                overdue: false,
            });
        }
    }
    Some(Eta {
        finish_ms: None,
        remaining_secs: None,
        percentile: 90,
        scope,
        samples: pool.len(),
        overdue: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(key: &str, secs: u64) -> Sample {
        Sample {
            key: key.into(),
            secs,
            at: 0,
        }
    }

    #[test]
    fn needs_enough_history() {
        assert!(estimate(&[s("a", 10), s("a", 20)], "a", 0, 0).is_none());
    }

    #[test]
    fn uses_the_project_then_falls_back_to_all() {
        let mut v = vec![s("a", 10), s("a", 20), s("a", 30), s("b", 1000)];
        let e = estimate(&v, "a", 0, 0).unwrap();
        assert_eq!(e.scope, Scope::Project);
        assert_eq!(e.samples, 3);
        assert_eq!(e.percentile, 50);
        assert_eq!(e.finish_ms, Some(20_000));
        assert_eq!(e.remaining_secs, Some(20));
        v.retain(|x| x.key != "a" || x.secs == 10);
        v.push(s("c", 40));
        let e = estimate(&v, "a", 0, 0).unwrap();
        assert_eq!(e.scope, Scope::All);
        assert_eq!(e.samples, 3);
    }

    #[test]
    fn moves_to_higher_percentiles_then_overdue() {
        let v: Vec<Sample> = (1..=10).map(|i| s("a", i * 10)).collect();
        // p50 = 50 s, p75 = 80 s, p90 = 90 s.
        let at = |elapsed: u64| estimate(&v, "a", 1_000, 1_000 + elapsed * 1000).unwrap();
        assert_eq!(at(10).percentile, 50);
        assert_eq!(at(10).finish_ms, Some(1_000 + 50_000));
        assert_eq!(at(60).percentile, 75);
        assert_eq!(at(60).remaining_secs, Some(20));
        assert_eq!(at(85).percentile, 90);
        let late = at(95);
        assert!(late.overdue);
        assert_eq!(late.finish_ms, None);
    }

    #[test]
    fn history_round_trip_is_capped() {
        let d = tempfile::tempdir().unwrap();
        let h = History::at(&d.path().join("d.json"));
        h.record("a", 0).unwrap();
        assert!(h.samples().is_empty());
        for i in 1..=(MAX_SAMPLES as u64 + 3) {
            h.record("a", i).unwrap();
        }
        let got = h.samples();
        assert_eq!(got.len(), MAX_SAMPLES);
        assert_eq!(got[0].secs, 4);
    }
}
