//! The performance contract (§32, ADR 0062): the budget table, `bench/history.json`, the 5%
//! regression gate, the weekly `PERFORMANCE.md`, the local `perf.log` and low-power mode.
//!
//! Measuring lives where the code is (`kor bench --ui --cli --memory` in kor-cli, the desktop's
//! own bench run, `kor_memory::bench`); this module only knows numbers.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A measurement more than this much slower than the baseline blocks the PR (§32.3).
pub const REGRESSION: f64 = 0.05;
/// Below this share of the budget a difference is noise, never a regression (a 0.004 ms key read
/// moving to 0.0045 ms is +12% and means nothing).
pub const NOISE_SHARE_OF_BUDGET: f64 = 0.02;
/// Budgets are relaxed by this factor on the weak CI worker (2 cores / 4 GB, §32.3).
pub const WEAK_FACTOR: f64 = 2.0;
/// `bench/history.json` keeps this many runs.
const MAX_RUNS: usize = 1000;
/// UI frames slower than this are written to `perf.log` (§32.3).
pub const SLOW_FRAME_MS: f64 = 50.0;
/// `perf.log` is rotated to `perf.log.1` past this size.
const PERF_LOG_MAX: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Ms,
    Mb,
    Percent,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::Mb => "MB",
            Self::Percent => "%",
        }
    }
}

/// Which `kor bench` flag measures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Cli,
    Ui,
    Memory,
}

impl Group {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Ui => "ui",
            Self::Memory => "memory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub id: &'static str,
    pub group: Group,
    /// The p95 must stay under this.
    pub target: f64,
    pub unit: Unit,
    /// Spec wording (English), for reports.
    pub what: &'static str,
}

const fn b(id: &'static str, group: Group, target: f64, unit: Unit, what: &'static str) -> Budget {
    Budget {
        id,
        group,
        target,
        unit,
        what,
    }
}

/// §32.1 and §31.3.1, in the spec's order.
pub const BUDGETS: &[Budget] = &[
    b("cli.version", Group::Cli, 30.0, Unit::Ms, "`kor --version`"),
    b("cli.help", Group::Cli, 30.0, Unit::Ms, "`kor help`"),
    b(
        "cli.ready_cold",
        Group::Cli,
        150.0,
        Unit::Ms,
        "`kor` ready for the first command (cold)",
    ),
    b(
        "cli.token_path",
        Group::Cli,
        16.0,
        Unit::Ms,
        "model token → screen: stream path overhead (local SSE → text event)",
    ),
    b(
        "cli.memory_idle",
        Group::Cli,
        30.0,
        Unit::Mb,
        "CLI memory at idle",
    ),
    b(
        "cli.cpu_idle",
        Group::Cli,
        0.5,
        Unit::Percent,
        "CLI CPU at idle (no polling)",
    ),
    b("cli.binary", Group::Cli, 40.0, Unit::Mb, "CLI binary size"),
    b(
        "cli.leak",
        Group::Cli,
        10.0,
        Unit::Percent,
        "memory growth over a long session (accelerated soak)",
    ),
    b(
        "tui.first_draw",
        Group::Ui,
        200.0,
        Unit::Ms,
        "TUI open, first draw",
    ),
    b(
        "tui.keypress",
        Group::Ui,
        16.0,
        Unit::Ms,
        "TUI key press → frame (10k-line thread)",
    ),
    b(
        "tui.frame",
        Group::Ui,
        16.0,
        Unit::Ms,
        "TUI frame while a token streams (10k-line thread)",
    ),
    b(
        "tui.scroll",
        Group::Ui,
        16.0,
        Unit::Ms,
        "TUI scroll frame (10k-line thread)",
    ),
    b(
        "ui.startup_cold",
        Group::Ui,
        1000.0,
        Unit::Ms,
        "desktop window visible and interactive (cold)",
    ),
    b(
        "ui.startup_warm",
        Group::Ui,
        400.0,
        Unit::Ms,
        "desktop window visible and interactive (warm)",
    ),
    b(
        "ui.keypress",
        Group::Ui,
        16.0,
        Unit::Ms,
        "desktop composer key press → paint",
    ),
    b(
        "ui.token_render",
        Group::Ui,
        16.0,
        Unit::Ms,
        "desktop token → paint",
    ),
    b(
        "ui.panel_open",
        Group::Ui,
        100.0,
        Unit::Ms,
        "desktop panel / tab switch",
    ),
    b(
        "ui.thread_open",
        Group::Ui,
        100.0,
        Unit::Ms,
        "desktop open a 10k-message session",
    ),
    b(
        "ui.scroll",
        Group::Ui,
        16.0,
        Unit::Ms,
        "desktop scroll frame, 10k-message session",
    ),
    b(
        "ui.file_search",
        Group::Ui,
        50.0,
        Unit::Ms,
        "fuzzy search over 50k files",
    ),
    b(
        "ui.memory_idle",
        Group::Ui,
        200.0,
        Unit::Mb,
        "desktop memory at idle",
    ),
    b(
        "ui.cpu_idle",
        Group::Ui,
        0.5,
        Unit::Percent,
        "desktop CPU at idle",
    ),
    b(
        "ui.binary",
        Group::Ui,
        80.0,
        Unit::Mb,
        "desktop binary size",
    ),
    b(
        "ui.leak",
        Group::Ui,
        10.0,
        Unit::Percent,
        "desktop memory growth over a long session (accelerated soak)",
    ),
    b(
        "memory.write",
        Group::Memory,
        1.0,
        Unit::Ms,
        "knowledge: single write",
    ),
    b(
        "memory.batch",
        Group::Memory,
        50.0,
        Unit::Ms,
        "knowledge: batch of 1000 records (one transaction)",
    ),
    b(
        "memory.indexed",
        Group::Memory,
        100.0,
        Unit::Ms,
        "knowledge: batch of 1000 records searchable (indexed in the background)",
    ),
    b(
        "memory.batch_queued",
        Group::Memory,
        50.0,
        Unit::Ms,
        "knowledge: 1000 queued single writes (group commits)",
    ),
    b(
        "memory.read",
        Group::Memory,
        0.1,
        Unit::Ms,
        "knowledge: key read",
    ),
    b(
        "memory.fts",
        Group::Memory,
        20.0,
        Unit::Ms,
        "knowledge: full-text search",
    ),
    b(
        "memory.fts_common",
        Group::Memory,
        20.0,
        Unit::Ms,
        "knowledge: full-text search, frequent words",
    ),
    b(
        "memory.fts_dirty",
        Group::Memory,
        20.0,
        Unit::Ms,
        "knowledge: full-text search during a 60k-write burst",
    ),
    b(
        "memory.vector",
        Group::Memory,
        30.0,
        Unit::Ms,
        "knowledge: vector top-50 over 1M embeddings",
    ),
    b(
        "memory.inject",
        Group::Memory,
        100.0,
        Unit::Ms,
        "knowledge: context injection query",
    ),
    b(
        "memory.cold_open",
        Group::Memory,
        300.0,
        Unit::Ms,
        "knowledge: cold open + first search",
    ),
    b(
        "memory.disk",
        Group::Memory,
        2000.0,
        Unit::Mb,
        "knowledge: disk size",
    ),
];

pub fn budget(id: &str) -> Option<&'static Budget> {
    BUDGETS.iter().find(|b| b.id == id)
}

/// One measured number: the p95 is what budgets and the gate look at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    pub p50: f64,
    pub p95: f64,
    /// Samples behind it (1 for a size).
    pub n: usize,
}

impl Stat {
    pub fn single(v: f64) -> Self {
        Self {
            p50: v,
            p95: v,
            n: 1,
        }
    }

    /// p50 / p95 of durations in milliseconds; `None` when empty.
    pub fn of_durations(samples: &[Duration]) -> Option<Self> {
        Self::of(
            &samples
                .iter()
                .map(|d| d.as_secs_f64() * 1000.0)
                .collect::<Vec<_>>(),
        )
    }

    /// p50 / p95 (nearest rank) of plain numbers; `None` when empty.
    pub fn of(samples: &[f64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut v = samples.to_vec();
        v.sort_by(f64::total_cmp);
        let at = |q: f64| {
            // Nearest rank: ceil(q·n) - 1, clamped.
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let i = ((q * v.len() as f64).ceil() as usize)
                .saturating_sub(1)
                .min(v.len() - 1);
            v[i]
        };
        Some(Self {
            p50: at(0.50),
            p95: at(0.95),
            n: v.len(),
        })
    }
}

/// One `kor bench` run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// RFC 3339, UTC.
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub machine: String,
    #[serde(default)]
    pub weak: bool,
    pub results: BTreeMap<String, Stat>,
}

/// `bench/history.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    #[serde(default = "one")]
    pub version: u32,
    /// p95 per measurement, per machine (only runs on the same kind of machine compare).
    #[serde(default)]
    pub baseline: BTreeMap<String, BTreeMap<String, f64>>,
    #[serde(default)]
    pub runs: Vec<Run>,
}

fn one() -> u32 {
    1
}

impl History {
    /// A missing file is an empty history.
    ///
    /// # Errors
    /// When the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                version: 1,
                ..Self::default()
            }),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// # Errors
    /// When the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())? + "\n";
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn push(&mut self, run: Run) {
        self.runs.push(run);
        let extra = self.runs.len().saturating_sub(MAX_RUNS);
        self.runs.drain(..extra);
    }

    /// Make `run`'s numbers the baseline of its machine (merged: measurements not in `run` keep
    /// their old baseline).
    pub fn set_baseline(&mut self, run: &Run) {
        let base = self.baseline.entry(run.machine.clone()).or_default();
        for (id, s) in &run.results {
            base.insert(id.clone(), s.p95);
        }
    }
}

/// `windows-x86_64-16c` (`-weak` appended on the weak worker): runs only compare within one key.
pub fn machine_key(weak: bool) -> String {
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let mut k = format!(
        "{}-{}-{cores}c",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if weak {
        k.push_str("-weak");
    }
    k
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Regression {
    pub id: String,
    pub baseline: f64,
    pub now: f64,
    /// `now / baseline - 1`, e.g. `0.08` for +8%.
    pub change: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverBudget {
    pub id: String,
    pub p95: f64,
    pub target: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Verdict {
    pub regressions: Vec<Regression>,
    pub over_budget: Vec<OverBudget>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.regressions.is_empty() && self.over_budget.is_empty()
    }
}

/// The gate (§32.3): every budget must hold (relaxed by [`WEAK_FACTOR`] on the weak worker) and no
/// p95 may be more than [`REGRESSION`] over its machine's baseline, past the noise floor.
pub fn check(history: &History, run: &Run) -> Verdict {
    let base = history.baseline.get(&run.machine);
    let mut v = Verdict::default();
    for (id, s) in &run.results {
        let Some(budget) = budget(id) else { continue };
        let target = if run.weak {
            budget.target * WEAK_FACTOR
        } else {
            budget.target
        };
        if s.p95 > target {
            v.over_budget.push(OverBudget {
                id: id.clone(),
                p95: s.p95,
                target,
            });
        }
        if let Some(&old) = base.and_then(|b| b.get(id))
            && old > 0.0
            && s.p95 > old * (1.0 + REGRESSION)
            && s.p95 - old > budget.target * NOISE_SHARE_OF_BUDGET
        {
            v.regressions.push(Regression {
                id: id.clone(),
                baseline: old,
                now: s.p95,
                change: s.p95 / old - 1.0,
            });
        }
    }
    v
}

/// ISO week label of an RFC 3339 time, `2026-W45`.
fn week(ts: &str) -> Option<String> {
    use chrono::Datelike as _;
    let t = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    let w = t.iso_week();
    Some(format!("{}-W{:02}", w.year(), w.week()))
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => f64::midpoint(v[n / 2 - 1], v[n / 2]),
    }
}

fn num(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

/// `PERFORMANCE.md` (§32.3): the latest run of each machine against the budgets and the weekly
/// trend (median p95 per ISO week) as Mermaid charts GitHub renders, with the budget as a line.
#[allow(clippy::too_many_lines)] // one linear document: header, latest table, weekly charts
pub fn performance_md(history: &History) -> String {
    let mut out = String::from(
        "# Performance\n\nGenerated by `kor bench --performance-md` from `bench/history.json` (spec §32, ADR 0062). \
         Do not edit by hand.\n\nEvery number is a p95 measured by `kor bench --ui --cli --memory`; a PR that makes \
         any of them more than 5% slower than its machine's baseline fails CI.\n",
    );
    let mut machines: Vec<&str> = history.runs.iter().map(|r| r.machine.as_str()).collect();
    machines.sort_unstable();
    machines.dedup();
    if machines.is_empty() {
        out.push_str("\nNo runs recorded yet.\n");
        return out;
    }
    for m in machines {
        let runs: Vec<&Run> = history.runs.iter().filter(|r| r.machine == m).collect();
        let Some(last) = runs.last() else { continue };
        let _ = write!(
            out,
            "\n## {m}\n\nLatest run: {}{}.\n\n",
            last.ts,
            last.commit
                .as_deref()
                .map(|c| format!(" (`{c}`)"))
                .unwrap_or_default()
        );
        out.push_str("| Measurement | p95 | p50 | Budget | Baseline | |\n| --- | ---: | ---: | ---: | ---: | --- |\n");
        let base = history.baseline.get(m);
        for b in BUDGETS {
            let Some(s) = last.results.get(b.id) else {
                continue;
            };
            let target = if last.weak {
                b.target * WEAK_FACTOR
            } else {
                b.target
            };
            let baseline = base
                .and_then(|x| x.get(b.id))
                .map_or_else(|| "–".to_owned(), |v| num(*v));
            let ok = if s.p95 <= target { "ok" } else { "**over**" };
            let u = b.unit.as_str();
            let _ = writeln!(
                out,
                "| {} | {} {u} | {} {u} | < {} {u} | {baseline} | {ok} |",
                b.what,
                num(s.p95),
                num(s.p50),
                num(target)
            );
        }
        // Weekly medians.
        let mut weeks: BTreeMap<String, BTreeMap<&str, Vec<f64>>> = BTreeMap::new();
        for r in &runs {
            let Some(w) = week(&r.ts) else { continue };
            for (id, s) in &r.results {
                weeks
                    .entry(w.clone())
                    .or_default()
                    .entry(id.as_str())
                    .or_default()
                    .push(s.p95);
            }
        }
        if weeks.len() < 2 {
            out.push_str("\nWeekly charts appear once two weeks of runs exist.\n");
            continue;
        }
        let labels: Vec<&String> = weeks.keys().collect();
        out.push_str("\n### Weekly trend (median p95)\n");
        for b in BUDGETS {
            let series: Vec<Option<f64>> = weeks
                .values()
                .map(|ids| ids.get(b.id).map(|v| median(v.clone())))
                .collect();
            if series.iter().flatten().count() < 2 {
                continue;
            }
            // Weeks without a number repeat the previous one (Mermaid lines need every point).
            let mut last = series.iter().flatten().next().copied().unwrap_or(0.0);
            let points: Vec<String> = series
                .iter()
                .map(|v| {
                    if let Some(v) = v {
                        last = *v;
                    }
                    num(last)
                })
                .collect();
            let top = series.iter().flatten().copied().fold(b.target, f64::max) * 1.1;
            let _ = write!(
                out,
                "\n#### {}\n\n```mermaid\nxychart-beta\n    title \"{} (p95, {})\"\n    x-axis [{}]\n    y-axis \"{}\" 0 --> {}\n    line [{}]\n    line [{}]\n```\n",
                b.what.replace('`', ""),
                b.id,
                b.unit.as_str(),
                labels
                    .iter()
                    .map(|l| format!("\"{l}\""))
                    .collect::<Vec<_>>()
                    .join(", "),
                b.unit.as_str(),
                num(top),
                points.join(", "),
                vec![num(b.target); labels.len()].join(", "),
            );
        }
    }
    out
}

/// One `perf.log` line: a slow frame or call and what caused it (§32.3; local, never sent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfEntry {
    /// RFC 3339.
    pub ts: String,
    /// `desktop`, `tui`, `cli`, `plugin`.
    pub source: String,
    /// `frame`, `ipc`, `startup`, `hook`, …
    pub kind: String,
    /// The cause: a panel, an IPC method, a plugin.
    pub what: String,
    pub ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl PerfEntry {
    pub fn now(source: &str, kind: &str, what: &str, ms: f64, detail: Option<String>) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            source: source.chars().take(32).collect(),
            kind: kind.chars().take(32).collect(),
            what: what.chars().take(200).collect(),
            ms,
            detail: detail.map(|d| d.chars().take(500).collect()),
        }
    }
}

/// `<kor home>/perf.log`.
pub fn perf_log_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("perf.log"))
}

/// Append one entry (JSON line); past 4 MiB the log moves to `perf.log.1` first.
///
/// # Errors
/// When the file cannot be written.
pub fn append_to(path: &Path, e: &PerfEntry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > PERF_LOG_MAX) {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(e).map_err(std::io::Error::other)?;
    writeln!(f, "{line}")
}

/// Append to `perf.log` when `[perf] log` is on (no-op otherwise, errors ignored: logging must
/// never slow or break the thing it measures).
pub fn log(cfg: &kor_config::Config, e: &PerfEntry) {
    if cfg.perf.log
        && let Some(p) = perf_log_path()
    {
        let _ = append_to(&p, e);
    }
}

/// Entries of `perf.log` and its rotated predecessor, oldest first (bad lines skipped).
pub fn read_log(path: &Path) -> Vec<PerfEntry> {
    let mut out = Vec::new();
    for p in [path.with_extension("log.1"), path.to_owned()] {
        if let Ok(t) = std::fs::read_to_string(&p) {
            out.extend(t.lines().filter_map(|l| serde_json::from_str(l).ok()));
        }
    }
    out
}

/// One cause of slowness in `perf.log`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cause {
    pub source: String,
    pub kind: String,
    pub what: String,
    pub count: usize,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
}

/// `kor doctor --perf`: causes by total time lost, worst first.
pub fn causes(entries: &[PerfEntry]) -> Vec<Cause> {
    let mut by: BTreeMap<(&str, &str, &str), Vec<f64>> = BTreeMap::new();
    for e in entries {
        by.entry((&e.source, &e.kind, &e.what))
            .or_default()
            .push(e.ms);
    }
    let mut out: Vec<Cause> = by
        .into_iter()
        .map(|((source, kind, what), v)| {
            let s = Stat::of(&v).unwrap_or(Stat::single(0.0));
            Cause {
                source: source.into(),
                kind: kind.into(),
                what: what.into(),
                count: v.len(),
                p95_ms: s.p95,
                max_ms: v.iter().copied().fold(0.0, f64::max),
                total_ms: v.iter().sum(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms));
    out
}

/// Low-power mode (§32.3): animations off, background indexing slowed. `auto` turns it on on
/// battery below 20% or on a machine with 2 or fewer cores.
pub fn low_power(cfg: &kor_config::Config) -> bool {
    match cfg.perf.low_power {
        kor_config::LowPower::On => true,
        kor_config::LowPower::Off => false,
        kor_config::LowPower::Auto => auto_low_power(
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            kor_platform::desktop_probe::battery(),
        ),
    }
}

/// Apply low-power mode to the background indexers (graph watcher, knowledge embedding).
/// Returns whether it is on.
pub fn apply_low_power(cfg: &kor_config::Config) -> bool {
    let on = low_power(cfg);
    kor_graph::watch::set_low_power(on);
    kor_memory::kb::set_low_power(on);
    on
}

fn auto_low_power(cores: usize, battery: Option<kor_platform::desktop_probe::Battery>) -> bool {
    cores <= 2 || battery.is_some_and(|b| !b.charging && b.percent < 20)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_platform::desktop_probe::Battery;

    fn run(ts: &str, results: &[(&str, f64)]) -> Run {
        Run {
            ts: ts.into(),
            commit: None,
            machine: "m".into(),
            weak: false,
            results: results
                .iter()
                .map(|(k, v)| ((*k).to_owned(), Stat::single(*v)))
                .collect(),
        }
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = Stat::of(&v).unwrap();
        assert_eq!((s.p50, s.p95, s.n), (50.0, 95.0, 100));
        assert!((Stat::of(&[7.0]).unwrap().p95 - 7.0).abs() < f64::EPSILON);
        assert!(Stat::of(&[]).is_none());
        let d = Stat::of_durations(&[Duration::from_millis(2), Duration::from_millis(4)]).unwrap();
        assert_eq!((d.p50, d.p95), (2.0, 4.0));
    }

    #[test]
    fn every_budget_id_is_unique_and_grouped() {
        for (i, b) in BUDGETS.iter().enumerate() {
            assert!(BUDGETS[i + 1..].iter().all(|o| o.id != b.id), "{}", b.id);
            assert!(
                b.id.starts_with(match b.group {
                    Group::Cli => "cli.",
                    Group::Ui => "tui.",
                    Group::Memory => "memory.",
                }) || (b.group == Group::Ui && b.id.starts_with("ui.")),
                "{}",
                b.id
            );
            assert!(b.target > 0.0);
        }
    }

    #[test]
    fn gate_blocks_five_percent_regressions_past_the_noise_floor() {
        let mut h = History::default();
        h.set_baseline(&run(
            "2026-11-02T10:00:00Z",
            &[
                ("cli.version", 20.0),
                ("memory.read", 0.004),
                ("memory.batch", 40.0),
            ],
        ));
        // +10% on 20 ms = 2 ms > 2% of the 30 ms budget (0.6 ms): blocked. +25% on a 4 µs read is
        // noise. +4% on the batch is under 5%.
        let v = check(
            &h,
            &run(
                "2026-11-03T10:00:00Z",
                &[
                    ("cli.version", 22.0),
                    ("memory.read", 0.005),
                    ("memory.batch", 41.6),
                ],
            ),
        );
        assert_eq!(v.regressions.len(), 1, "{v:?}");
        assert_eq!(v.regressions[0].id, "cli.version");
        assert!((v.regressions[0].change - 0.1).abs() < 1e-9);
        assert!(v.over_budget.is_empty());
        // Over budget, no baseline needed; the weak worker gets twice the budget.
        let v = check(&h, &run("x", &[("cli.help", 45.0)]));
        assert_eq!(
            v.over_budget,
            [OverBudget {
                id: "cli.help".into(),
                p95: 45.0,
                target: 30.0
            }]
        );
        let mut weak = run("x", &[("cli.help", 45.0)]);
        weak.weak = true;
        assert!(check(&h, &weak).passed());
        // Another machine's baseline is never used.
        let mut other = run("x", &[("cli.version", 29.0)]);
        other.machine = "n".into();
        assert!(check(&h, &other).passed());
    }

    #[test]
    fn history_round_trips_and_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bench/history.json");
        let mut h = History::load(&p).unwrap();
        assert_eq!(h.version, 1);
        for i in 0..(MAX_RUNS + 5) {
            h.push(run(&format!("t{i}"), &[("cli.help", 1.0)]));
        }
        assert_eq!(h.runs.len(), MAX_RUNS);
        assert_eq!(h.runs[0].ts, "t5");
        h.save(&p).unwrap();
        assert_eq!(History::load(&p).unwrap(), h);
        std::fs::write(&p, "{").unwrap();
        assert!(History::load(&p).is_err());
    }

    #[test]
    fn performance_md_has_latest_table_and_weekly_charts() {
        let mut h = History::default();
        assert!(performance_md(&h).contains("No runs recorded yet"));
        h.push(run("2026-11-02T10:00:00Z", &[("cli.version", 20.0)]));
        h.push(run("2026-11-03T10:00:00Z", &[("cli.version", 22.0)]));
        h.push(run(
            "2026-11-10T10:00:00Z",
            &[("cli.version", 18.0), ("cli.help", 40.0)],
        ));
        h.set_baseline(&h.runs[0].clone());
        let md = performance_md(&h);
        assert!(
            md.contains("| `kor --version` | 18.0 ms | 18.0 ms | < 30.0 ms | 20.0 | ok |"),
            "{md}"
        );
        assert!(
            md.contains("| `kor help` | 40.0 ms | 40.0 ms | < 30.0 ms | – | **over** |"),
            "{md}"
        );
        assert!(md.contains("x-axis [\"2026-W45\", \"2026-W46\"]"), "{md}");
        // Week 45 median of 20 and 22 is 21.
        assert!(md.contains("line [21.0, 18.0]"), "{md}");
        assert!(md.contains("line [30.0, 30.0]"), "{md}");
        // `kor help` has one week only: no chart.
        assert!(!md.contains("title \"cli.help"), "{md}");
    }

    #[test]
    fn perf_log_appends_rotates_and_summarizes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("perf.log");
        for ms in [120.0, 80.0, 60.0] {
            append_to(
                &p,
                &PerfEntry::now("desktop", "frame", "panel:graph", ms, None),
            )
            .unwrap();
        }
        append_to(
            &p,
            &PerfEntry::now(
                "plugin",
                "hook",
                "lint-plugin",
                900.0,
                Some("PreToolUse".into()),
            ),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let entries = read_log(&p);
        assert_eq!(entries.len(), 4);
        let c = causes(&entries);
        assert_eq!(c[0].what, "lint-plugin");
        assert_eq!(
            (c[1].what.as_str(), c[1].count, c[1].max_ms, c[1].total_ms),
            ("panel:graph", 3, 120.0, 260.0)
        );
        // Rotation keeps the old lines readable.
        let big = "x".repeat(usize::try_from(PERF_LOG_MAX).unwrap() + 1);
        std::fs::write(&p, big).unwrap();
        append_to(&p, &PerfEntry::now("tui", "frame", "stream", 55.0, None)).unwrap();
        assert!(p.with_extension("log.1").exists());
        assert_eq!(read_log(&p).len(), 1);
    }

    #[test]
    fn low_power_on_small_machines_and_low_battery() {
        assert!(auto_low_power(2, None));
        assert!(!auto_low_power(8, None));
        assert!(auto_low_power(
            8,
            Some(Battery {
                percent: 15,
                charging: false
            })
        ));
        assert!(!auto_low_power(
            8,
            Some(Battery {
                percent: 15,
                charging: true
            })
        ));
        assert!(!auto_low_power(
            8,
            Some(Battery {
                percent: 60,
                charging: false
            })
        ));
        let mut cfg = kor_config::Config::default();
        cfg.perf.low_power = kor_config::LowPower::On;
        assert!(low_power(&cfg));
        cfg.perf.low_power = kor_config::LowPower::Off;
        assert!(!low_power(&cfg));
    }

    #[test]
    fn weeks_are_iso() {
        assert_eq!(week("2026-01-01T00:00:00Z").as_deref(), Some("2026-W01"));
        assert_eq!(
            week("2026-11-10T12:00:00+03:00").as_deref(),
            Some("2026-W46")
        );
        assert_eq!(week("nope"), None);
    }
}
