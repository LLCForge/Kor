//! Time and cost estimate before a task runs (Master Build Prompt §27.16, ADR 0063).
//!
//! Inputs: the task's category (the §5 router's keyword classifier), the history of similar
//! finished tasks on this machine (`<kor home>/estimates.jsonl`), the model's catalog price and
//! speed (§4), and the current context size. Every figure is an estimate and shown with `≈`; the
//! method says where it came from. When a task finishes, its actual duration, cost and touched
//! files are appended to the history, and the ratio actual/estimate of the last runs corrects
//! later estimates (local statistics only, nothing is sent anywhere).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};

use crate::cost::{Category, Prices, classify};

/// History lines read at most.
const MAX_RUNS: usize = 5_000;
/// Similar runs needed before history replaces the priors.
pub const MIN_SAMPLES: usize = 3;
const MAX_SIMILAR: usize = 15;
/// Runs used for the self-correction ratio.
const CORRECTION_RUNS: usize = 20;
/// Tokens the conversation grows per agent turn (tool results, the model's own output).
const GROWTH_PER_TURN: u64 = 2_500;
const OUTPUT_PER_TURN: u64 = 600;
/// Seconds per turn spent in tools (reads, edits, commands), on top of model time.
const TOOL_SECS_PER_TURN: f64 = 3.0;
const DEFAULT_TPS: f64 = 60.0;
const LOCAL_TPS: f64 = 20.0;
const DEFAULT_TTFT_MS: f64 = 1_500.0;
/// Tokens of the system prompt and tool definitions when a front end estimates without a live
/// session (the agent's own breakdown replaces it once a session runs).
pub const BASE_CONTEXT: u64 = 6_000;

/// Context tokens of a saved session: its transcript plus [`BASE_CONTEXT`].
pub fn session_context(transcript: &Path) -> u64 {
    let bytes =
        std::fs::metadata(transcript).map_or(0, |m| usize::try_from(m.len()).unwrap_or(usize::MAX));
    BASE_CONTEXT + kor_agent::context::tokens(bytes)
}

/// `<kor home>/estimates.jsonl`.
pub fn file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("estimates.jsonl"))
}

/// Where an estimate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Median of similar finished tasks on this machine.
    History,
    /// Category priors (no or too few similar tasks yet).
    Prior,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub secs: u64,
    /// `None` when the model has no known price.
    pub usd: Option<f64>,
    pub files: u32,
    pub turns: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub method: Method,
    pub samples: usize,
    pub category: Option<Category>,
    pub model: String,
    pub local: bool,
    /// Actual/estimate ratio applied from earlier runs (1.0 without history).
    pub correction: f64,
}

/// One finished task.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Run {
    pub at: u64,
    pub category: Option<Category>,
    /// Distinct words of the task (lowercase), for similarity. Stays on this machine.
    pub words: Vec<String>,
    pub model: String,
    pub est_secs: u64,
    pub est_usd: Option<f64>,
    pub secs: u64,
    pub usd: f64,
    /// `usd` comes (partly) from catalog prices, not the provider's own figure.
    pub usd_estimated: bool,
    pub files: u32,
    pub prompt: u64,
    pub completion: u64,
    pub turns: u32,
    pub ok: bool,
}

/// Distinct content words of a task (3+ letters, at most 40).
pub fn words(task: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for w in task.split(|c: char| !c.is_alphanumeric()) {
        let w = w.to_lowercase();
        if w.chars().count() >= 3 {
            set.insert(w);
        }
        if set.len() >= 40 {
            break;
        }
    }
    set.into_iter().collect()
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    let (sa, sb): (BTreeSet<&String>, BTreeSet<&String>) = (a.iter().collect(), b.iter().collect());
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        #[allow(clippy::cast_precision_loss)] // word counts are tiny
        let r = inter as f64 / union as f64;
        r
    }
}

/// Finished runs, oldest first.
pub fn read(path: &Path) -> Vec<Run> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(MAX_RUNS);
    lines[skip..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Append one finished run.
///
/// # Errors
/// When the history cannot be written.
pub fn append(path: &Path, r: &Run) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let line = serde_json::to_string(r).map_err(std::io::Error::other)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

/// (turns, files) a task of `cat` usually takes before there is history.
fn prior(cat: Option<Category>) -> (u32, u32) {
    match cat {
        Some(Category::SimpleEdit) => (4, 1),
        Some(Category::ComplexRefactor) => (14, 8),
        Some(Category::TestGeneration) => (8, 3),
        Some(Category::Docs) => (5, 2),
        Some(Category::Review) => (4, 0),
        Some(Category::Planning) => (3, 0),
        None => (6, 3),
    }
}

fn median<T: Copy + Ord>(mut v: Vec<T>) -> Option<T> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Prompt tokens of `turns` turns starting from `context` tokens (the history grows each turn).
fn prompt_tokens(context: u64, turns: u32) -> u64 {
    let t = u64::from(turns);
    t * context + GROWTH_PER_TURN * t * t.saturating_sub(1) / 2
}

fn is_local(model: &str) -> bool {
    let p = model.split('/').next().unwrap_or_default();
    p == "local" || p.starts_with("local-") || p == "ollama"
}

/// A run that measured a real task: at least one model turn and more than a second. Turns that
/// ended at once (no key, refused, stopped) would drag estimates down to "≈ 1 s".
fn real(r: &Run) -> bool {
    r.ok && r.turns > 0 && r.secs > 1
}

/// Median actual/estimate duration ratio of the last runs that carry both, clamped to 0.25–4.
fn correction(runs: &[Run]) -> f64 {
    let mut ratios: Vec<u64> = runs
        .iter()
        .rev()
        .filter(|r| real(r) && r.est_secs > 0)
        .take(CORRECTION_RUNS)
        .map(|r| r.secs * 1000 / r.est_secs)
        .collect();
    if ratios.len() < MIN_SAMPLES {
        return 1.0;
    }
    ratios.sort_unstable();
    #[allow(clippy::cast_precision_loss)] // ratios in permille, small
    let r = ratios[ratios.len() / 2] as f64 / 1000.0;
    r.clamp(0.25, 4.0)
}

/// The estimate for `task` on `model` with `context` tokens already in the conversation.
pub fn estimate(task: &str, model: &str, context: u64, prices: &Prices, runs: &[Run]) -> Estimate {
    let cat = classify(task);
    let w = words(task);
    let local = is_local(model);
    let mut similar: Vec<(&Run, f64)> = runs
        .iter()
        .filter(|r| real(r) && r.category == cat)
        .map(|r| (r, jaccard(&w, &r.words)))
        .collect();
    similar.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.at.cmp(&a.0.at)));
    similar.truncate(MAX_SIMILAR);
    let (tps, ttft_ms) = prices
        .speed(model)
        .unwrap_or((if local { LOCAL_TPS } else { DEFAULT_TPS }, DEFAULT_TTFT_MS));
    let tps = if tps > 0.0 { tps } else { DEFAULT_TPS };
    let corr = correction(runs);
    let (method, turns, files, prompt, completion, raw_secs) = if similar.len() >= MIN_SAMPLES {
        let pick =
            |f: fn(&Run) -> u64| median(similar.iter().map(|(r, _)| f(r)).collect()).unwrap_or(0);
        let turns = u32::try_from(pick(|r| u64::from(r.turns)))
            .unwrap_or(u32::MAX)
            .max(1);
        let files = u32::try_from(pick(|r| u64::from(r.files))).unwrap_or(u32::MAX);
        // History of the same kind of task, but a bigger context now costs more per turn.
        let prompt = pick(|r| r.prompt).max(prompt_tokens(context, turns));
        let completion = pick(|r| r.completion);
        #[allow(clippy::cast_precision_loss)]
        let secs = pick(|r| r.secs) as f64;
        (Method::History, turns, files, prompt, completion, secs)
    } else {
        let (turns, files) = prior(cat);
        let completion = u64::from(turns) * OUTPUT_PER_TURN;
        #[allow(clippy::cast_precision_loss)]
        let secs = f64::from(turns)
            * (ttft_ms / 1000.0 + OUTPUT_PER_TURN as f64 / tps + TOOL_SECS_PER_TURN);
        (
            Method::Prior,
            turns,
            files,
            prompt_tokens(context, turns),
            completion,
            secs,
        )
    };
    let usd = prices.get(model).map(|(i, o)| {
        #[allow(clippy::cast_precision_loss)] // token counts far below 2^52
        let c = (prompt as f64 * i + completion as f64 * o) / 1_000_000.0;
        c * if method == Method::Prior { corr } else { 1.0 }
    });
    // Never less than one model turn of this model.
    #[allow(clippy::cast_precision_loss)]
    let one_turn = ttft_ms / 1000.0 + OUTPUT_PER_TURN as f64 / tps;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // positive, clamped seconds
    let secs = (raw_secs * corr).max(one_turn).round().clamp(1.0, 86_400.0) as u64;
    Estimate {
        secs,
        usd: if local { Some(0.0) } else { usd },
        files,
        turns,
        prompt_tokens: prompt,
        completion_tokens: completion,
        method,
        samples: similar.len(),
        category: cat,
        model: model.to_owned(),
        local,
        correction: corr,
    }
}

/// `45s`, `4m`, `1m12s`, `1h5m`.
pub fn fmt_dur(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 && s % 60 == 0 => format!("{}m", s / 60),
        s if s < 3600 => format!("{}m{}s", s / 60, s % 60),
        s => format!("{}h{}m", s / 3600, s % 3600 / 60),
    }
}

/// `$0.03`, `$1.20`, `<$0.01`.
pub fn fmt_usd(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 {
        "<$0.01".into()
    } else if usd < 10.0 {
        format!("${usd:.2}")
    } else {
        format!("${usd:.0}")
    }
}

/// `≈ 4m · ≈ $0.03 · ~12 files` (localized) with the method in a second line.
pub fn line(l: &Localizer, e: &Estimate) -> String {
    let usd = match e.usd {
        Some(u) => fmt_usd(u),
        None => l.t("estimate-no-price"),
    };
    l.t_args(
        "estimate-line",
        &args!("time" => fmt_dur(e.secs), "usd" => usd, "files" => u64::from(e.files)),
    )
}

/// How the estimate was made, localized.
pub fn method_line(l: &Localizer, e: &Estimate) -> String {
    match e.method {
        Method::History => l.t_args("estimate-method-history", &args!("samples" => e.samples)),
        Method::Prior => l.t_args(
            "estimate-method-prior",
            &args!("category" => e.category.map_or("general", Category::key)),
        ),
    }
}

/// At the end of a task: actual time and cost next to the estimate.
pub fn actual_line(l: &Localizer, r: &Run) -> String {
    let est_usd = r.est_usd.map_or_else(|| l.t("estimate-no-price"), fmt_usd);
    l.t_args(
        "estimate-actual",
        &args!("time" => fmt_dur(r.secs), "usd" => format!("{}{}", if r.usd_estimated { "≈" } else { "" }, fmt_usd(r.usd)), "est_time" => fmt_dur(r.est_secs), "est_usd" => est_usd, "files" => u64::from(r.files)),
    )
}

/// Spinner text while a task runs: `(1m12s / ≈4m · $0.011 / ≈$0.03)`.
pub fn progress(elapsed_ms: u64, spent_usd: f64, e: &Estimate) -> String {
    let est = e
        .usd
        .map_or_else(String::new, |u| format!(" / ≈{}", fmt_usd(u)));
    format!(
        "({} / ≈{} · {}{est})",
        fmt_dur(elapsed_ms / 1000),
        fmt_dur(e.secs),
        if spent_usd > 0.0 {
            format!("${spent_usd:.3}")
        } else {
            "$0".into()
        }
    )
}

/// True when the run is far off its estimate (more than twice the time or cost): front ends
/// colour the line.
pub fn drifting(elapsed_ms: u64, spent_usd: f64, e: &Estimate) -> bool {
    elapsed_ms / 1000 > e.secs.saturating_mul(2)
        || e.usd.is_some_and(|u| u > 0.0 && spent_usd > u * 2.0)
}

/// The message to ask with when `est` would cross a budget limit (daily/monthly), else `None`.
pub fn budget_warning(
    l: &Localizer,
    e: &Estimate,
    budget: &kor_config::Budget,
    today: f64,
    month: f64,
) -> Option<String> {
    let usd = e.usd.filter(|u| *u > 0.0)?;
    let crosses = |limit: f64, spent: f64| limit > 0.0 && spent < limit && spent + usd > limit;
    let (limit, spent, which) = if crosses(budget.daily_limit_usd, today) {
        (budget.daily_limit_usd, today, l.t("estimate-limit-daily"))
    } else if crosses(budget.monthly_limit_usd, month) {
        (
            budget.monthly_limit_usd,
            month,
            l.t("estimate-limit-monthly"),
        )
    } else {
        return None;
    };
    Some(l.t_args(
        "estimate-over-budget",
        &args!("usd" => fmt_usd(usd), "spent" => fmt_usd(spent), "limit" => fmt_usd(limit), "which" => which),
    ))
}

/// A task the cost control is tracking, from route to `task_done`.
#[derive(Debug, Clone)]
pub struct Tracking {
    pub estimate: Estimate,
    pub words: Vec<String>,
    pub started: u64,
    pub usd: f64,
    pub usd_estimated: bool,
    pub prompt: u64,
    pub completion: u64,
    pub turns: u32,
    pub files: BTreeSet<String>,
}

impl Tracking {
    pub fn new(estimate: Estimate, task: &str, now_ms: u64) -> Self {
        Self {
            estimate,
            words: words(task),
            started: now_ms,
            usd: 0.0,
            usd_estimated: false,
            prompt: 0,
            completion: 0,
            turns: 0,
            files: BTreeSet::new(),
        }
    }

    /// The finished run for the history.
    pub fn finish(&self, now_ms: u64, ok: bool) -> Run {
        Run {
            at: now_ms,
            category: self.estimate.category,
            words: self.words.clone(),
            model: self.estimate.model.clone(),
            est_secs: self.estimate.secs,
            est_usd: self.estimate.usd,
            secs: now_ms.saturating_sub(self.started) / 1000,
            usd: self.usd,
            usd_estimated: self.usd_estimated,
            files: u32::try_from(self.files.len()).unwrap_or(u32::MAX),
            prompt: self.prompt,
            completion: self.completion,
            turns: self.turns,
            ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_providers::model_catalog::ModelData;

    fn prices() -> Prices {
        Prices::from_models(vec![ModelData {
            model_id: "m1".into(),
            input_price_per_m: Some(3.0),
            output_price_per_m: Some(15.0),
            speed_tps: Some(100.0),
            ttft_ms: Some(1000.0),
            ..ModelData::default()
        }])
    }

    fn run(task: &str, secs: u64, files: u32, est_secs: u64) -> Run {
        Run {
            at: 1,
            category: classify(task),
            words: words(task),
            model: "x/m1".into(),
            est_secs,
            secs,
            usd: 0.05,
            files,
            prompt: 40_000,
            completion: 3_000,
            turns: 6,
            ok: true,
            ..Run::default()
        }
    }

    #[test]
    fn priors_without_history_and_local_models_cost_nothing() {
        // Not "README": that word classifies as Docs (cost::KEYWORDS).
        let e = estimate("fix the typo in the config", "x/m1", 10_000, &prices(), &[]);
        assert_eq!(e.method, Method::Prior);
        assert_eq!(e.category, Some(Category::SimpleEdit));
        assert_eq!((e.turns, e.files), (4, 1));
        // 4 turns × (1 s first token + 600/100 s output + 3 s tools) = 40 s.
        assert_eq!(e.secs, 40);
        assert_eq!(e.prompt_tokens, 4 * 10_000 + 2_500 * 6);
        let usd = e.usd.unwrap();
        assert!((usd - (55_000.0 * 3.0 + 2_400.0 * 15.0) / 1e6).abs() < 1e-9);
        let l = estimate("fix the typo", "ollama/qwen", 1_000, &prices(), &[]);
        assert_eq!(l.usd, Some(0.0));
        assert!(l.local);
        assert!(
            estimate("x", "unknown/model", 0, &prices(), &[])
                .usd
                .is_none()
        );
    }

    #[test]
    fn runs_that_ended_at_once_do_not_shrink_the_estimate() {
        let mut quick = vec![
            run("fix typo in docs page", 1, 0, 40),
            run("fix typo in the config", 0, 0, 40),
            run("fix the typo here", 1, 0, 40),
        ];
        for r in &mut quick {
            r.turns = 0;
        }
        let e = estimate("fix a typo in the config", "x/m1", 1_000, &prices(), &quick);
        assert_eq!(e.method, Method::Prior, "instant runs are not history");
        assert_eq!(e.secs, 40);
        // Even real history never goes below one turn (1 s first token + 600/100 s output).
        let fast: Vec<Run> = [
            "fix typo in docs page",
            "fix typo in the config",
            "fix the typo here",
        ]
        .iter()
        .map(|t| run(t, 2, 0, 0))
        .collect();
        assert_eq!(
            estimate("fix a typo in the config", "x/m1", 1_000, &prices(), &fast).secs,
            7
        );
    }

    #[test]
    fn similar_history_wins_and_corrects() {
        let runs: Vec<Run> = vec![
            run("fix typo in docs page", 30, 1, 20),
            run("fix typo in the config", 50, 2, 25),
            run("fix the typo here", 40, 1, 20),
            run("refactor the whole parser", 900, 12, 300),
        ];
        let e = estimate("fix a typo in the config", "x/m1", 1_000, &prices(), &runs);
        assert_eq!(e.method, Method::History);
        assert_eq!(e.samples, 3);
        assert_eq!(e.files, 1);
        // Median 40 s × correction (median of 1.5, 2.0, 2.0, 3.0 = 2.0).
        assert!((e.correction - 2.0).abs() < 1e-9);
        assert_eq!(e.secs, 80);
        assert!(drifting(200_000, 0.0, &e));
        assert!(!drifting(10_000, 0.0, &e));
    }

    #[test]
    fn budget_warning_only_when_the_estimate_crosses_a_limit() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let e = Estimate {
            usd: Some(0.5),
            ..estimate("fix", "x/m1", 0, &prices(), &[])
        };
        let b = kor_config::Budget {
            daily_limit_usd: 1.0,
            ..kor_config::Budget::default()
        };
        assert!(budget_warning(&l, &e, &b, 0.2, 0.2).is_none());
        let w = budget_warning(&l, &e, &b, 0.8, 0.8).unwrap();
        assert!(w.contains("$0.50"), "{w}");
        assert!(
            budget_warning(&l, &e, &b, 1.2, 1.2).is_none(),
            "already over: the budget check handles it"
        );
        assert_eq!(
            progress(
                72_000,
                0.011,
                &Estimate {
                    secs: 240,
                    usd: Some(0.03),
                    ..e
                }
            ),
            "(1m12s / ≈4m · $0.011 / ≈$0.03)"
        );
    }

    #[test]
    #[allow(clippy::many_single_char_names)] // short names of the math or the protocol
    fn tracking_finishes_into_a_run() {
        let e = estimate("write tests for parser", "x/m1", 0, &prices(), &[]);
        let mut t = Tracking::new(e, "write tests for parser", 1_000);
        t.files.insert("a.rs".into());
        t.usd = 0.1;
        let r = t.finish(61_000, true);
        assert_eq!((r.secs, r.files, r.ok), (60, 1, true));
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.jsonl");
        append(&p, &r).unwrap();
        assert_eq!(read(&p), vec![r]);
    }
}
