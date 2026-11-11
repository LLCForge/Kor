//! Smart cost router, spending limits and the cost dashboard (Master Build Prompt §5.1, §5.2).
//!
//! * Each task is classified by keyword heuristics (no model call) and sent to the model set in
//!   `[routing]` for its category, else to the cheapest known model whose catalog intelligence
//!   is close enough to the default model's, skipping models whose task history is poor.
//! * Every decision is appended to `<kor home>/routing.jsonl` with its reason (`kor cost why`).
//! * Every model call goes to the usage ledger with the provider-reported cost when there is one;
//!   otherwise catalog prices give an estimate, always shown with "≈".
//! * `[budget]` limits are checked before every model call (see `kor_agent::cost`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Datelike, Local, TimeZone};
use kor_agent::cost::{CostControl, Outcome, Plan, Route, Spend};
use kor_agent::toolcall::ToolMode;
use kor_config::{Budget, OnLimit, Routing};
use kor_core::Usage;
use kor_i18n::{Lang, Localizer, args};
use kor_providers::Provider;
use kor_providers::model_catalog::{ModelData, lookup, read_cache};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::usage::{self, Entry};

/// Decision lines kept when the routing log is read.
const MAX_DECISIONS: usize = 20_000;
/// Tasks of one category on one model before its success rate counts.
const MIN_HISTORY: u32 = 5;
/// Success rate below which a model is not chosen for a category.
const MIN_SUCCESS: f64 = 0.6;
/// The ledger is re-read at most this often for the budget (other sessions spend too).
const REFRESH_MS: u64 = 60_000;
const DAY_MS: u64 = 86_400_000;

// ---------------------------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    SimpleEdit,
    ComplexRefactor,
    TestGeneration,
    Docs,
    Review,
    Planning,
}

impl Category {
    pub fn key(self) -> &'static str {
        match self {
            Self::SimpleEdit => "simple_edit",
            Self::ComplexRefactor => "complex_refactor",
            Self::TestGeneration => "test_generation",
            Self::Docs => "docs",
            Self::Review => "review",
            Self::Planning => "planning",
        }
    }

    fn configured(self, r: &Routing) -> &str {
        match self {
            Self::SimpleEdit => &r.simple_edit,
            Self::ComplexRefactor => &r.complex_refactor,
            Self::TestGeneration => &r.test_generation,
            Self::Docs => &r.docs,
            Self::Review => &r.review,
            Self::Planning => &r.planning,
        }
        .trim()
    }

    /// Share of the default model's intelligence index a cheaper model needs for this category.
    fn bar(self) -> f64 {
        match self {
            Self::SimpleEdit | Self::Docs => 0.6,
            Self::TestGeneration => 0.75,
            Self::Review | Self::Planning => 0.9,
            Self::ComplexRefactor => 0.95,
        }
    }
}

/// Keywords per category (English, Turkish, French), checked in this order.
const KEYWORDS: &[(Category, &[&str])] = &[
    (
        Category::Review,
        &[
            "review",
            "code review",
            "gözden geçir",
            "incele",
            "denetle",
            "relis",
            "revue de",
            "audit",
        ],
    ),
    (
        Category::Planning,
        &[
            "plan",
            "design",
            "architecture",
            "roadmap",
            "how should",
            "approach",
            "mimari",
            "tasarla",
            "yol haritası",
            "nasıl yapmalı",
            "conception",
            "concevoir",
        ],
    ),
    (
        Category::TestGeneration,
        &[
            "write test",
            "write a test",
            "add test",
            "add a test",
            "generate test",
            "unit test",
            "tests for",
            "test coverage",
            "test yaz",
            "test ekle",
            "test üret",
            "testlerini yaz",
            "écris des tests",
            "ajoute des tests",
            "tests unitaires",
        ],
    ),
    (
        Category::Docs,
        &[
            "readme",
            "docstring",
            "documentation",
            "document ",
            "changelog",
            "jsdoc",
            "rustdoc",
            "dokümantasyon",
            "belgele",
            "yorum ekle",
            "açıklama ekle",
            "documente",
        ],
    ),
    (
        Category::ComplexRefactor,
        &[
            "refactor",
            "restructure",
            "rewrite",
            "migrate",
            "across the",
            "entire",
            "all files",
            "yeniden yapılandır",
            "baştan yaz",
            "taşı",
            "tüm dosya",
            "refactoris",
            "réécri",
            "restructur",
        ],
    ),
    (
        Category::SimpleEdit,
        &[
            "rename",
            "typo",
            "fix",
            "change",
            "update",
            "add ",
            "remove",
            "delete",
            "bump",
            "düzelt",
            "değiştir",
            "güncelle",
            "ekle",
            "kaldır",
            "sil",
            "yeniden adlandır",
            "corrige",
            "renomme",
            "modifie",
            "ajoute",
            "supprime",
        ],
    ),
];

/// A long request that asks for an edit is treated as a complex one.
const LONG_TASK_CHARS: usize = 1500;

/// Category of a task, from keywords alone; `None` when nothing matches.
pub fn classify(task: &str) -> Option<Category> {
    let t = task.to_lowercase();
    let cat = KEYWORDS
        .iter()
        .find(|(_, words)| words.iter().any(|w| t.contains(w)))
        .map(|(c, _)| *c)?;
    if cat == Category::SimpleEdit && task.chars().count() > LONG_TASK_CHARS {
        return Some(Category::ComplexRefactor);
    }
    Some(cat)
}

// ---------------------------------------------------------------------------------------------
// Prices
// ---------------------------------------------------------------------------------------------

/// Catalog prices (USD per million tokens) and intelligence, from the cached model catalog.
#[derive(Debug, Clone, Default)]
pub struct Prices {
    models: Vec<ModelData>,
}

impl Prices {
    /// The cached catalog (`kor catalog` / the desktop refresh it); never touches the network.
    pub fn load() -> Self {
        let models = kor_platform::cache_dir()
            .and_then(|d| read_cache(&d.join("model-catalog.json")))
            .map(|c| c.catalog.models)
            .unwrap_or_default();
        Self { models }
    }

    pub fn from_models(models: Vec<ModelData>) -> Self {
        Self { models }
    }

    fn data(&self, model_ref: &str) -> Option<&ModelData> {
        let rest = model_ref.split_once('/').map_or(model_ref, |(_, m)| m);
        self.models
            .iter()
            .find(|m| m.model_id == model_ref || m.model_id == rest)
            .or_else(|| lookup(&self.models, rest))
    }

    /// (input, output) USD per million tokens. Local models cost nothing.
    pub fn get(&self, model_ref: &str) -> Option<(f64, f64)> {
        if is_local_ref(model_ref) {
            return Some((0.0, 0.0));
        }
        let m = self.data(model_ref)?;
        Some((m.input_price_per_m?, m.output_price_per_m?))
    }

    /// (output tokens per second, time to first token in ms) from the catalog (§27.16 estimates).
    pub fn speed(&self, model_ref: &str) -> Option<(f64, f64)> {
        let m = self.data(model_ref)?;
        Some((m.speed_tps?, m.ttft_ms.unwrap_or(1_500.0)))
    }

    pub fn intelligence(&self, model_ref: &str) -> Option<f64> {
        let m = self.data(model_ref)?;
        m.aa_intelligence_index.or(m.aa_coding_index)
    }
}

fn is_local_ref(model_ref: &str) -> bool {
    let p = model_ref.split('/').next().unwrap_or_default();
    p == "local" || p.starts_with("local-") || p == "ollama"
}

/// Weighted for agent work, which reads far more than it writes.
fn blended((i, o): (f64, f64)) -> f64 {
    (3.0 * i + o) / 4.0
}

/// Cost of one ledger entry: the provider's figure (exact) or the catalog price (estimate).
/// `None` when neither is known.
pub fn entry_cost(e: &Entry, prices: &Prices) -> Option<(f64, bool)> {
    if let Some(c) = e.cost_usd {
        return Some((c, e.estimated));
    }
    let (i, o) = prices.get(&e.model)?;
    let (read, write) = cache_rates(&e.model);
    let cached = e.cache_read.saturating_add(e.cache_write).min(e.prompt);
    #[allow(clippy::cast_precision_loss)] // token counts far below 2^52
    let usd = ((e.prompt - cached) as f64 * i
        + e.cache_read.min(e.prompt) as f64 * i * read
        + e.cache_write.min(e.prompt - e.cache_read.min(e.prompt)) as f64 * i * write
        + e.completion as f64 * o)
        / 1_000_000.0;
    Some((usd, !is_local_ref(&e.model)))
}

/// Prompt-cache price as a share of the input price: (read, write). Published by each
/// provider; unknown providers get no discount (1.0) rather than a guessed one.
/// shortcut: fixed per family; move into the catalogue when it carries cache prices.
fn cache_rates(model_ref: &str) -> (f64, f64) {
    let m = model_ref.to_ascii_lowercase();
    if m.contains("claude") || m.starts_with("anthropic/") {
        (0.1, 1.25)
    } else if m.contains("deepseek") {
        (0.1, 1.0)
    } else if m.contains("gemini") || m.starts_with("google/") {
        (0.25, 1.0)
    } else if m.starts_with("openai/")
        || m.contains("gpt-")
        || m.contains("/o3")
        || m.contains("/o4")
    {
        (0.5, 1.0)
    } else {
        (1.0, 1.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------------------------

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn local(ms: u64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(i64::try_from(ms).ok()?).single()
}

fn ms_of(t: Option<chrono::DateTime<Local>>) -> u64 {
    t.and_then(|t| u64::try_from(t.timestamp_millis()).ok())
        .unwrap_or(0)
}

/// Start of the local day and month containing `now`, and the length of that month in days.
fn periods(now: u64) -> (u64, u64, u32) {
    let Some(t) = local(now) else {
        return (now, now, 30);
    };
    let day = t.date_naive();
    let midnight = |d: chrono::NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|n| Local.from_local_datetime(&n).earliest())
    };
    let first = day.with_day(1).unwrap_or(day);
    let next = if first.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    let len = next
        .and_then(|n| u32::try_from((n - first).num_days()).ok())
        .unwrap_or(30);
    (ms_of(midnight(day)), ms_of(midnight(first)), len)
}

pub fn fmt_time(ms: u64) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
}

// ---------------------------------------------------------------------------------------------
// Report (`kor cost`, session summary, desktop)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ModelRow {
    pub model: String,
    pub requests: u64,
    pub prompt: u64,
    pub completion: u64,
    pub usd: f64,
    /// Part of `usd` comes from catalog prices, not from the provider.
    pub estimated: bool,
    /// Requests with no known price (left out of `usd`).
    pub unpriced: u64,
}

impl ModelRow {
    fn add(&mut self, e: &Entry, prices: &Prices) {
        self.requests += 1;
        self.prompt += e.prompt;
        self.completion += e.completion;
        match entry_cost(e, prices) {
            Some((usd, est)) => {
                self.usd += usd;
                self.estimated |= est;
            }
            None => self.unpriced += 1,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub days: u32,
    pub rows: Vec<ModelRow>,
    pub total: ModelRow,
    pub today: ModelRow,
    pub month: ModelRow,
    /// Image, video and audio generations in the window (ADR 0060), also part of `total`.
    pub media: ModelRow,
    /// Month total extrapolated to the whole month (always an estimate).
    pub projection_usd: f64,
    pub daily_limit_usd: f64,
    pub monthly_limit_usd: f64,
}

/// Per-model usage of the last `days` days, plus today, this month and a projection.
pub fn report(entries: &[Entry], prices: &Prices, now: u64, days: u32, budget: &Budget) -> Report {
    let since = now.saturating_sub(u64::from(days) * DAY_MS);
    let (day_start, month_start, month_len) = periods(now);
    let mut rows: BTreeMap<&str, ModelRow> = BTreeMap::new();
    let mut r = Report {
        days,
        daily_limit_usd: budget.daily_limit_usd,
        monthly_limit_usd: budget.monthly_limit_usd,
        ..Report::default()
    };
    for e in entries {
        if e.at >= since {
            rows.entry(&e.model).or_default().add(e, prices);
            r.total.add(e, prices);
            if e.media {
                r.media.add(e, prices);
            }
        }
        if e.at >= day_start {
            r.today.add(e, prices);
        }
        if e.at >= month_start {
            r.month.add(e, prices);
        }
    }
    r.rows = rows
        .into_iter()
        .map(|(m, mut row)| {
            m.clone_into(&mut row.model);
            row
        })
        .collect();
    r.rows
        .sort_by(|a, b| b.usd.total_cmp(&a.usd).then(a.model.cmp(&b.model)));
    #[allow(clippy::cast_precision_loss)] // milliseconds in a month
    let elapsed = (now.saturating_sub(month_start) as f64 / DAY_MS as f64).max(1.0);
    r.projection_usd = r.month.usd / elapsed * f64::from(month_len);
    r
}

/// Usage of the last `days` days grouped by `key` (for example the package of an entry's
/// `path`); each row's `model` field holds the key. Most expensive first.
pub fn rows_by(
    entries: &[Entry],
    prices: &Prices,
    now: u64,
    days: u32,
    key: impl Fn(&Entry) -> String,
) -> Vec<ModelRow> {
    let since = now.saturating_sub(u64::from(days) * DAY_MS);
    let mut rows: BTreeMap<String, ModelRow> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.at >= since) {
        let k = key(e);
        rows.entry(k.clone())
            .or_insert_with(|| ModelRow {
                model: k,
                ..ModelRow::default()
            })
            .add(e, prices);
    }
    let mut out: Vec<ModelRow> = rows.into_values().collect();
    out.sort_by(|a, b| b.usd.total_cmp(&a.usd).then(a.model.cmp(&b.model)));
    out
}

/// `$0.84`, `≈$0.84` for an estimate.
pub fn fmt_usd(usd: f64, estimated: bool) -> String {
    format!("{}${usd:.2}", if estimated { "≈" } else { "" })
}

fn fmt_tokens(n: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // display only
    let f = n as f64;
    if n >= 1_000_000 {
        format!("{:.1}M", f / 1e6)
    } else if n >= 1_000 {
        format!("{:.0}k", f / 1e3)
    } else {
        n.to_string()
    }
}

/// One dashboard row: label, cost (`?` when unpriced), requests, tokens.
pub fn row_line(l: &Localizer, label: &str, r: &ModelRow) -> String {
    let usd = if r.requests > 0 && r.unpriced == r.requests {
        "?".to_owned()
    } else {
        fmt_usd(r.usd, r.estimated)
    };
    let reqs = l.t_args("cost-requests", &args!("count" => r.requests));
    format!(
        "  {label:<40} {usd:>10}  {reqs:>12}  {}↑ {}↓",
        fmt_tokens(r.prompt),
        fmt_tokens(r.completion)
    )
}

fn month_line(l: &Localizer, r: &Report) -> String {
    let limit = if r.monthly_limit_usd > 0.0 {
        format!(" / {}", fmt_usd(r.monthly_limit_usd, false))
    } else {
        String::new()
    };
    l.t_args(
        "cost-month-line",
        &args!("spent" => format!("{}{limit}", fmt_usd(r.month.usd, r.month.estimated)),
               "projection" => fmt_usd(r.projection_usd, true)),
    )
}

/// The `kor cost` dashboard.
pub fn report_lines(l: &Localizer, r: &Report) -> Vec<String> {
    let mut out = vec![l.t_args("cost-header", &args!("days" => r.days))];
    if r.rows.is_empty() {
        out.push(format!("  {}", l.t("cost-none")));
    }
    for row in &r.rows {
        out.push(row_line(l, &row.model, row));
    }
    if !r.rows.is_empty() {
        out.push(row_line(l, &l.t("cost-total"), &r.total));
    }
    if r.media.requests > 0 {
        out.push(l.t_args(
            "cost-media-line",
            &args!("spent" => fmt_usd(r.media.usd, r.media.estimated), "count" => r.media.requests),
        ));
    }
    let today_limit = if r.daily_limit_usd > 0.0 {
        format!(" / {}", fmt_usd(r.daily_limit_usd, false))
    } else {
        String::new()
    };
    out.push(l.t_args(
        "cost-today-line",
        &args!("spent" => format!("{}{today_limit}", fmt_usd(r.today.usd, r.today.estimated))),
    ));
    out.push(month_line(l, r));
    if r.total.estimated || r.month.estimated {
        out.push(l.t("cost-estimate-note"));
    }
    if r.total.unpriced > 0 {
        out.push(l.t_args("cost-unpriced-note", &args!("count" => r.total.unpriced)));
    }
    out
}

/// CSV of the per-model rows (`kor cost --export csv`).
pub fn to_csv(r: &Report) -> String {
    let mut s =
        String::from("model,requests,prompt_tokens,completion_tokens,usd,estimated,unpriced\n");
    for row in &r.rows {
        // Model refs never contain quotes; quoting keeps commas safe.
        let _ = writeln!(
            s,
            "\"{}\",{},{},{},{:.6},{},{}",
            row.model.replace('"', ""),
            row.requests,
            row.prompt,
            row.completion,
            row.usd,
            row.estimated,
            row.unpriced
        );
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Routing decisions
// ---------------------------------------------------------------------------------------------

/// One line of `routing.jsonl`: a decision, or later the outcome of the same `id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decision {
    pub id: String,
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// First words of the task, so the user can tell decisions apart. Stays on this machine.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<bool>,
}

pub fn decisions_file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("routing.jsonl"))
}

fn append(path: &Path, d: &Decision) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(d).map_err(std::io::Error::other)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

/// Decisions, oldest first, with their outcomes merged in.
pub fn decisions(path: &Path) -> Vec<Decision> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(MAX_DECISIONS);
    let mut out: Vec<Decision> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for d in lines[skip..]
        .iter()
        .filter_map(|l| serde_json::from_str::<Decision>(l).ok())
    {
        if d.model.is_empty() {
            if let (Some(&i), Some(o)) = (index.get(&d.id), d.outcome) {
                out[i].outcome = Some(o);
            }
        } else {
            index.insert(d.id.clone(), out.len());
            out.push(d);
        }
    }
    out
}

/// (successes, finished tasks) of `model` on `cat`.
fn history(h: &[Decision], cat: Category, model: &str) -> (u32, u32) {
    h.iter()
        .filter(|d| d.category == Some(cat) && d.model == model)
        .filter_map(|d| d.outcome)
        .fold((0, 0), |(ok, n), o| (ok + u32::from(o), n + 1))
}

fn weak(h: &[Decision], cat: Category, model: &str) -> Option<(u32, u32)> {
    let (ok, n) = history(h, cat, model);
    (n >= MIN_HISTORY && f64::from(ok) / f64::from(n) < MIN_SUCCESS).then_some((ok, n))
}

/// Why a model was chosen; localized by [`Why::text`].
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    NoCategory,
    Configured(Category),
    /// The configured model's history is poor; the default model is used instead.
    ConfiguredWeak {
        cat: Category,
        model: String,
        ok: u32,
        n: u32,
    },
    Cheapest {
        cat: Category,
        usd_per_m: f64,
    },
    DefaultOnly(Category),
    Downgrade,
}

impl Why {
    pub fn text(&self, l: &Localizer) -> String {
        match self {
            Self::NoCategory => l.t("route-why-none"),
            Self::Configured(c) => l.t_args("route-why-configured", &args!("category" => c.key())),
            Self::ConfiguredWeak { cat, model, ok, n } => l.t_args(
                "route-why-weak",
                &args!("category" => cat.key(), "model" => model.clone(),
                       "ok" => *ok, "count" => *n),
            ),
            Self::Cheapest { cat, usd_per_m } => l.t_args(
                "route-why-cheapest",
                &args!("category" => cat.key(), "price" => format!("{usd_per_m:.2}")),
            ),
            Self::DefaultOnly(c) => l.t_args("route-why-default", &args!("category" => c.key())),
            Self::Downgrade => l.t("route-why-downgrade"),
        }
    }
}

/// Model for a task of category `cat`. `candidates` are the models the user configured (all of
/// them reachable with their keys); `default` is the session's model.
pub fn pick(
    cat: Option<Category>,
    routing: &Routing,
    default: &str,
    candidates: &[String],
    prices: &Prices,
    h: &[Decision],
) -> (String, Why) {
    let Some(cat) = cat else {
        return (default.to_owned(), Why::NoCategory);
    };
    let configured = cat.configured(routing);
    if !configured.is_empty() {
        return match weak(h, cat, configured) {
            Some((ok, n)) => (
                default.to_owned(),
                Why::ConfiguredWeak {
                    cat,
                    model: configured.to_owned(),
                    ok,
                    n,
                },
            ),
            None => (configured.to_owned(), Why::Configured(cat)),
        };
    }
    let (Some(base_price), Some(base_iq)) = (prices.get(default), prices.intelligence(default))
    else {
        return (default.to_owned(), Why::DefaultOnly(cat));
    };
    let best = candidates
        .iter()
        .filter(|m| m.as_str() != default && weak(h, cat, m).is_none())
        .filter(|m| {
            prices
                .intelligence(m)
                .is_some_and(|iq| iq >= base_iq * cat.bar())
        })
        .filter_map(|m| prices.get(m).map(|p| (m, blended(p))))
        .filter(|(_, p)| *p < blended(base_price))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match best {
        Some((m, p)) => (m.clone(), Why::Cheapest { cat, usd_per_m: p }),
        None => (default.to_owned(), Why::DefaultOnly(cat)),
    }
}

/// The cheapest priced model among `candidates`.
pub fn cheapest<'a>(candidates: &'a [String], prices: &Prices) -> Option<&'a String> {
    candidates
        .iter()
        .filter_map(|m| prices.get(m).map(|p| (m, blended(p))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(m, _)| m)
}

// ---------------------------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Totals {
    /// When the ledger was last read.
    at: u64,
    day_start: u64,
    today: f64,
    month: f64,
    estimated: bool,
}

#[derive(Debug, Default)]
struct BudgetFlags {
    alerted: bool,
    downgrade_reported: bool,
    allowed_over: bool,
}

/// Spend state from totals; updates `flags` so alerts and downgrades are reported once.
fn spend_state(
    l: &Localizer,
    b: &Budget,
    today: f64,
    month: f64,
    estimated: bool,
    flags: &mut BudgetFlags,
) -> Spend {
    let limits = [
        ("daily", b.daily_limit_usd, today),
        ("monthly", b.monthly_limit_usd, month),
    ];
    let mut worst: Option<(&str, f64, f64, f64)> = None;
    for (scope, limit, spent) in limits {
        if limit > 0.0 {
            let pct = spent / limit * 100.0;
            if worst.is_none_or(|w| pct > w.3) {
                worst = Some((scope, limit, spent, pct));
            }
        }
    }
    let Some((scope, limit, spent, pct)) = worst else {
        return Spend::Ok;
    };
    let action = match b.on_limit {
        OnLimit::Downgrade => kor_agent::cost::OnLimit::Downgrade,
        OnLimit::Pause => kor_agent::cost::OnLimit::Pause,
        OnLimit::Ask => kor_agent::cost::OnLimit::Ask,
    };
    let fields = |over: bool| {
        json!({"scope": scope, "spent_usd": spent, "limit_usd": limit, "percent": pct.round(),
               "estimated": estimated, "action": over.then(|| format!("{:?}", b.on_limit).to_lowercase())})
    };
    let money = format!("{} / {}", fmt_usd(spent, estimated), fmt_usd(limit, false));
    if pct >= 100.0 {
        if flags.allowed_over
            || (action == kor_agent::cost::OnLimit::Downgrade && flags.downgrade_reported)
        {
            return Spend::Ok;
        }
        flags.downgrade_reported |= action == kor_agent::cost::OnLimit::Downgrade;
        flags.alerted = true;
        let key = match action {
            kor_agent::cost::OnLimit::Downgrade => "budget-over-downgrade",
            kor_agent::cost::OnLimit::Pause => "budget-over-pause",
            kor_agent::cost::OnLimit::Ask => "budget-over-ask",
        };
        return Spend::Over {
            action,
            message: l.t_args(
                key,
                &args!("scope" => l.t(&format!("budget-{scope}")), "money" => money),
            ),
            fields: fields(true),
        };
    }
    if pct >= f64::from(b.alert_at_percent) && !flags.alerted {
        flags.alerted = true;
        return Spend::Alert {
            message: l.t_args(
                "budget-alert",
                &args!("scope" => l.t(&format!("budget-{scope}")), "money" => money,
                       "percent" => format!("{pct:.0}")),
            ),
            fields: fields(false),
        };
    }
    Spend::Ok
}

// ---------------------------------------------------------------------------------------------
// The control the agent loop calls
// ---------------------------------------------------------------------------------------------

/// A model the router may switch to.
#[derive(Clone)]
pub struct Target {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub tool_mode: ToolMode,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("provider", &self.provider.id())
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct State {
    session: BTreeMap<String, ModelRow>,
    totals: Totals,
    flags: BudgetFlags,
    history: Vec<Decision>,
    last: Option<String>,
    seq: u64,
    /// Last file the agent touched (for the ledger's per-package breakdown).
    touched: Option<String>,
    /// Ledger role tag of a multi-agent task member (`kor task`, ADR 0037).
    role: Option<String>,
    /// The running task's estimate and actuals (§27.16).
    tracking: Option<crate::estimate::Tracking>,
    /// The last finished task's actuals and estimate, until `finished` reads them.
    finished: Option<crate::estimate::Run>,
}

/// Router, ledger and budget for one session.
pub struct Control {
    l: Localizer,
    routing: Routing,
    budget: Budget,
    /// `provider/model` of the session's model.
    default: String,
    /// Every model the router may use, by `provider/model` (the default included).
    targets: BTreeMap<String, Target>,
    prices: Prices,
    ledger: Option<PathBuf>,
    log: Option<PathBuf>,
    /// `estimates.jsonl` next to the ledger (§27.16); `None` with the ledger off.
    estimates: Option<PathBuf>,
    started: u64,
    state: Mutex<State>,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("default", &self.default)
            .field("targets", &self.targets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Where a [`Control`] reads and writes; `None` turns that file off (tests).
#[derive(Debug, Clone, Default)]
pub struct Files {
    pub ledger: Option<PathBuf>,
    pub log: Option<PathBuf>,
}

impl Files {
    pub fn default_paths() -> Self {
        Self {
            ledger: usage::file(),
            log: decisions_file(),
        }
    }
}

impl Control {
    pub fn new(
        lang: Lang,
        routing: Routing,
        budget: Budget,
        default: Target,
        others: Vec<Target>,
        prices: Prices,
        files: Files,
    ) -> Self {
        let default_ref = format!("{}/{}", default.provider.id(), default.model);
        let mut targets: BTreeMap<String, Target> = others
            .into_iter()
            .map(|t| (format!("{}/{}", t.provider.id(), t.model), t))
            .collect();
        targets.insert(default_ref.clone(), default);
        let history = files.log.as_deref().map(decisions).unwrap_or_default();
        Self {
            l: Localizer::new(lang),
            routing,
            budget,
            default: default_ref,
            targets,
            prices,
            estimates: files
                .ledger
                .as_deref()
                .and_then(Path::parent)
                .map(|d| d.join("estimates.jsonl")),
            ledger: files.ledger,
            log: files.log,
            started: now_ms(),
            state: Mutex::new(State {
                history,
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn candidates(&self) -> Vec<String> {
        self.targets.keys().cloned().collect()
    }

    fn route_to(&self, model_ref: &str, reason: String) -> Option<Route> {
        let t = self.targets.get(model_ref)?;
        Some(Route {
            provider: t.provider.clone(),
            model: t.model.clone(),
            tool_mode: t.tool_mode,
            reason,
        })
    }

    /// Re-read today's and this month's spend from the ledger when it is stale.
    fn refresh(&self, s: &mut State, now: u64) {
        let (day_start, month_start, _) = periods(now);
        if now.saturating_sub(s.totals.at) < REFRESH_MS && s.totals.day_start == day_start {
            return;
        }
        let entries = self.ledger.as_deref().map(usage::read).unwrap_or_default();
        let mut t = Totals {
            at: now,
            day_start,
            ..Totals::default()
        };
        for e in entries.iter().filter(|e| e.at >= month_start) {
            if let Some((usd, est)) = entry_cost(e, &self.prices) {
                t.month += usd;
                t.estimated |= est;
                if e.at >= day_start {
                    t.today += usd;
                }
            }
        }
        s.totals = t;
    }

    fn over_downgrading(&self, s: &mut State) -> bool {
        if self.budget.on_limit != OnLimit::Downgrade || s.flags.allowed_over {
            return false;
        }
        self.refresh(s, now_ms());
        let over = |limit: f64, spent: f64| limit > 0.0 && spent >= limit;
        over(self.budget.daily_limit_usd, s.totals.today)
            || over(self.budget.monthly_limit_usd, s.totals.month)
    }

    /// Tag every later ledger entry of this session with `role` (`task:<id>/<role>`).
    pub fn set_role(&self, role: impl Into<String>) {
        self.lock().role = Some(role.into());
    }

    /// This session's spend in USD and whether any of it is estimated from catalog prices.
    pub fn session_usd(&self) -> (f64, bool) {
        self.lock()
            .session
            .values()
            .fold((0.0, false), |(usd, est), r| {
                (usd + r.usd, est || r.estimated)
            })
    }

    /// This session's spend per model, for the summary at the end.
    pub fn session_rows(&self) -> Vec<ModelRow> {
        self.lock().session.values().cloned().collect()
    }

    /// Session summary (§5.2): duration, per-model cost, requests and tokens, month and projection.
    pub fn summary_lines(&self, l: &Localizer) -> Vec<String> {
        let rows = self.session_rows();
        if rows.is_empty() {
            return Vec::new();
        }
        let minutes = now_ms().saturating_sub(self.started) / 60_000;
        let mut out = vec![l.t_args("cost-session-head", &args!("minutes" => minutes))];
        let mut total = ModelRow::default();
        for r in &rows {
            out.push(row_line(l, &r.model, r));
            total.requests += r.requests;
            total.prompt += r.prompt;
            total.completion += r.completion;
            total.usd += r.usd;
            total.estimated |= r.estimated;
            total.unpriced += r.unpriced;
        }
        out.push(row_line(l, &l.t("cost-total"), &total));
        let entries = self.ledger.as_deref().map(usage::read).unwrap_or_default();
        out.push(month_line(
            l,
            &report(&entries, &self.prices, now_ms(), 30, &self.budget),
        ));
        out
    }
}

impl Control {
    /// The time and cost estimate of `task` on the session's model with `context` tokens in the
    /// conversation (front ends show it in the composer before anything is sent).
    pub fn estimate(&self, task: &str, context: u64) -> crate::estimate::Estimate {
        let runs = self
            .estimates
            .as_deref()
            .map(crate::estimate::read)
            .unwrap_or_default();
        crate::estimate::estimate(task, &self.default, context, &self.prices, &runs)
    }

    /// Estimate of the running task, if one is tracked.
    pub fn current_estimate(&self) -> Option<crate::estimate::Estimate> {
        self.lock().tracking.as_ref().map(|t| t.estimate.clone())
    }

    fn route_plan(&self, task: &str) -> Option<Plan> {
        let mut s = self.lock();
        let downgrade = self.over_downgrading(&mut s);
        if !self.routing.enabled && !downgrade {
            return None;
        }
        let cat = classify(task);
        let candidates = self.candidates();
        let (model, why) = if downgrade {
            let m = cheapest(&candidates, &self.prices)
                .map_or_else(|| self.default.clone(), Clone::clone);
            (m, Why::Downgrade)
        } else {
            pick(
                cat,
                &self.routing,
                &self.default,
                &candidates,
                &self.prices,
                &s.history,
            )
        };
        let reason = why.text(&self.l);
        let main = self
            .route_to(&model, reason.clone())
            .or_else(|| self.route_to(&self.default, reason.clone()))?;
        let verify_ref = self.routing.verify_with.trim();
        let verify = (self.routing.enabled && !verify_ref.is_empty() && verify_ref != model)
            .then(|| self.route_to(verify_ref, self.l.t("route-why-verify")))
            .flatten();
        s.seq += 1;
        let d = Decision {
            id: format!("{}-{}", now_ms(), s.seq),
            at: now_ms(),
            category: cat,
            model: model.clone(),
            reason,
            task: task
                .split_whitespace()
                .take(12)
                .collect::<Vec<_>>()
                .join(" "),
            outcome: None,
        };
        if let Some(p) = &self.log {
            // Best effort: the log explains decisions; it must never stop a task.
            let _ = append(p, &d);
        }
        s.last = Some(d.id.clone());
        s.history.push(d);
        Some(Plan { main, verify })
    }
}

impl CostControl for Control {
    fn route(&self, task: &str) -> Option<Plan> {
        let plan = self.route_plan(task);
        let model = plan.as_ref().map_or_else(
            || self.default.clone(),
            |p| format!("{}/{}", p.main.provider.id(), p.main.model),
        );
        let runs = self
            .estimates
            .as_deref()
            .map(crate::estimate::read)
            .unwrap_or_default();
        let e = crate::estimate::estimate(task, &model, 0, &self.prices, &runs);
        self.lock().tracking = Some(crate::estimate::Tracking::new(e, task, now_ms()));
        plan
    }

    fn finished(&self) -> Option<String> {
        let r = self.lock().finished.take()?;
        Some(crate::estimate::actual_line(&self.l, &r))
    }

    fn preflight(&self) -> Option<kor_agent::cost::Preflight> {
        let e = self.current_estimate()?;
        let ask = if self.budget.on_limit == OnLimit::Ask {
            let mut s = self.lock();
            self.refresh(&mut s, now_ms());
            let (today, month) = (s.totals.today, s.totals.month);
            drop(s);
            crate::estimate::budget_warning(&self.l, &e, &self.budget, today, month)
        } else {
            None
        };
        Some(kor_agent::cost::Preflight {
            line: crate::estimate::line(&self.l, &e),
            method: crate::estimate::method_line(&self.l, &e),
            secs: e.secs,
            usd: e.usd,
            files: e.files,
            ask,
            price: self.prices.get(&e.model),
        })
    }

    fn record(&self, model: &str, u: &Usage, cost_usd: Option<f64>) -> Option<(f64, bool)> {
        let (path, role) = {
            let s = self.lock();
            (s.touched.clone(), s.role.clone())
        };
        let e = Entry {
            at: now_ms(),
            model: model.to_owned(),
            prompt: u.input,
            completion: u.output,
            cost_usd: cost_usd.filter(|c| c.is_finite() && *c >= 0.0),
            path,
            role,
            media: false,
            estimated: false,
            cache_read: u.cache_read,
            cache_write: u.cache_write,
        };
        if let Some(p) = &self.ledger {
            let _ = usage::append(p, &e);
        }
        let mut s = self.lock();
        let row = s
            .session
            .entry(model.to_owned())
            .or_insert_with(|| ModelRow {
                model: model.to_owned(),
                ..ModelRow::default()
            });
        row.add(&e, &self.prices);
        let cost = entry_cost(&e, &self.prices);
        if let Some(t) = s.tracking.as_mut() {
            t.prompt += u.input;
            t.completion += u.output;
            t.turns += 1;
            t.usd += cost.map_or(0.0, |c| c.0);
            t.usd_estimated |= cost.is_some_and(|c| c.1);
        }
        if let Some((usd, est)) = cost {
            s.totals.today += usd;
            s.totals.month += usd;
            s.totals.estimated |= est;
        }
        cost
    }

    fn touched(&self, path: &std::path::Path) {
        let mut s = self.lock();
        let p = path.to_string_lossy().into_owned();
        if let Some(t) = s.tracking.as_mut() {
            t.files.insert(p.clone());
        }
        s.touched = Some(p);
    }

    fn task_done(&self, outcome: Outcome) {
        let mut s = self.lock();
        // §27.16: the actuals of a finished task correct later estimates (local history only).
        if let Some(t) = s.tracking.take()
            && outcome != Outcome::Unknown
        {
            let run = t.finish(now_ms(), outcome == Outcome::Ok);
            if let Some(p) = &self.estimates {
                let _ = crate::estimate::append(p, &run);
            }
            s.finished = Some(run);
        }
        let Some(id) = s.last.take() else { return };
        let ok = match outcome {
            Outcome::Ok => true,
            Outcome::Failed => false,
            Outcome::Unknown => return,
        };
        if let Some(d) = s.history.iter_mut().rev().find(|d| d.id == id) {
            d.outcome = Some(ok);
        }
        if let Some(p) = &self.log {
            let _ = append(
                p,
                &Decision {
                    id,
                    at: now_ms(),
                    outcome: Some(ok),
                    ..Decision::default()
                },
            );
        }
    }

    fn spend(&self) -> Spend {
        if self.budget.daily_limit_usd <= 0.0 && self.budget.monthly_limit_usd <= 0.0 {
            return Spend::Ok;
        }
        let mut s = self.lock();
        self.refresh(&mut s, now_ms());
        let State { totals, flags, .. } = &mut *s;
        spend_state(
            &self.l,
            &self.budget,
            totals.today,
            totals.month,
            totals.estimated,
            flags,
        )
    }

    fn downgrade(&self) -> Option<Route> {
        let m = cheapest(&self.candidates(), &self.prices)?.clone();
        self.route_to(&m, Why::Downgrade.text(&self.l))
    }

    fn allow_over(&self) {
        self.lock().flags.allowed_over = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(id: &str, i: f64, o: f64, iq: Option<f64>) -> ModelData {
        ModelData {
            model_id: id.into(),
            input_price_per_m: Some(i),
            output_price_per_m: Some(o),
            aa_intelligence_index: iq,
            ..ModelData::default()
        }
    }

    fn prices() -> Prices {
        Prices::from_models(vec![
            md("anthropic/claude-sonnet", 3.0, 15.0, Some(60.0)),
            md("deepseek/deepseek-v3", 0.3, 1.1, Some(50.0)),
            md("meta/llama-3.3-70b", 0.6, 0.6, Some(40.0)),
        ])
    }

    #[test]
    fn classifies_by_keywords_in_three_languages() {
        assert_eq!(classify("Please review this PR"), Some(Category::Review));
        assert_eq!(classify("bu modülü gözden geçir"), Some(Category::Review));
        assert_eq!(
            classify("write tests for the parser"),
            Some(Category::TestGeneration)
        );
        assert_eq!(
            classify("parser için test yaz"),
            Some(Category::TestGeneration)
        );
        assert_eq!(classify("update the README"), Some(Category::Docs));
        assert_eq!(
            classify("refactor the auth system"),
            Some(Category::ComplexRefactor)
        );
        assert_eq!(
            classify("corrige la faute de frappe"),
            Some(Category::SimpleEdit)
        );
        assert_eq!(
            classify("fix the typo in main.rs"),
            Some(Category::SimpleEdit)
        );
        assert_eq!(classify("mimariyi tasarla"), Some(Category::Planning));
        assert_eq!(classify("hello there"), None);
        let long = format!("fix {}", "x ".repeat(1000));
        assert_eq!(classify(&long), Some(Category::ComplexRefactor));
    }

    #[test]
    fn cached_prompt_tokens_are_billed_at_the_cache_price() {
        let p = prices();
        let e = |model: &str, prompt: u64, read: u64, write: u64| Entry {
            at: 0,
            model: model.into(),
            prompt,
            completion: 0,
            cost_usd: None,
            path: None,
            role: None,
            media: false,
            estimated: false,
            cache_read: read,
            cache_write: write,
        };
        let usd = |x: &Entry| entry_cost(x, &p).unwrap().0;
        // Claude at $3/M: 1M prompt, 600k read (0.1x), 100k written (1.25x), 300k plain.
        let claude = usd(&e("anthropic/claude-sonnet", 1_000_000, 600_000, 100_000));
        assert!(
            (claude - (0.3 * 3.0 + 0.6 * 0.3 + 0.1 * 3.75)).abs() < 1e-9,
            "{claude}"
        );
        // No cache: the plain input price.
        assert!((usd(&e("anthropic/claude-sonnet", 1_000_000, 0, 0)) - 3.0).abs() < 1e-9);
        // An unknown family gets no discount; DeepSeek reads at 0.1x.
        assert!((usd(&e("meta/llama-3.3-70b", 1_000_000, 1_000_000, 0)) - 0.6).abs() < 1e-9);
        assert!((usd(&e("deepseek/deepseek-v3", 1_000_000, 1_000_000, 0)) - 0.03).abs() < 1e-9);
        // Counts larger than the prompt never make the cost negative.
        assert!(usd(&e("anthropic/claude-sonnet", 10, 50, 50)) >= 0.0);
    }

    #[test]
    fn prices_match_catalog_ids_and_local_is_free() {
        let p = prices();
        assert_eq!(p.get("anthropic/claude-sonnet"), Some((3.0, 15.0)));
        assert_eq!(p.get("openrouter/deepseek/deepseek-v3"), Some((0.3, 1.1)));
        assert_eq!(p.get("local/qwen2.5-coder:7b"), Some((0.0, 0.0)));
        assert_eq!(p.get("nobody/unknown"), None);
        let e = Entry {
            at: 0,
            model: "anthropic/claude-sonnet".into(),
            prompt: 1_000_000,
            completion: 100_000,
            cost_usd: None,
            path: None,
            role: None,
            media: false,
            estimated: false,
            cache_read: 0,
            cache_write: 0,
        };
        assert_eq!(entry_cost(&e, &p), Some((4.5, true)));
        let exact = Entry {
            cost_usd: Some(0.01),
            ..e
        };
        assert_eq!(entry_cost(&exact, &p), Some((0.01, false)));
    }

    #[test]
    fn picks_configured_then_cheapest_good_enough_and_skips_weak_history() {
        let p = prices();
        let default = "anthropic/claude-sonnet";
        let cands: Vec<String> = [default, "deepseek/deepseek-v3", "meta/llama-3.3-70b"]
            .map(str::to_owned)
            .to_vec();
        let mut r = Routing::default();
        // Docs bar = 0.6 × 60 = 36: llama (40) qualifies but deepseek is cheaper overall.
        let (m, why) = pick(Some(Category::Docs), &r, default, &cands, &p, &[]);
        assert_eq!(m, "deepseek/deepseek-v3");
        assert!(matches!(why, Why::Cheapest { .. }));
        // Refactor bar = 0.95 × 60 = 57: nothing cheaper qualifies.
        let (m, why) = pick(
            Some(Category::ComplexRefactor),
            &r,
            default,
            &cands,
            &p,
            &[],
        );
        assert_eq!(
            (m.as_str(), why),
            (default, Why::DefaultOnly(Category::ComplexRefactor))
        );
        assert_eq!(pick(None, &r, default, &cands, &p, &[]).1, Why::NoCategory);

        r.docs = "meta/llama-3.3-70b".into();
        let (m, why) = pick(Some(Category::Docs), &r, default, &cands, &p, &[]);
        assert_eq!(
            (m.as_str(), why),
            ("meta/llama-3.3-70b", Why::Configured(Category::Docs))
        );

        // Five failed docs tasks on llama: back to the default model, with the reason.
        let bad: Vec<Decision> = (0..5)
            .map(|i| Decision {
                id: i.to_string(),
                category: Some(Category::Docs),
                model: "meta/llama-3.3-70b".into(),
                outcome: Some(false),
                ..Decision::default()
            })
            .collect();
        let (m, why) = pick(Some(Category::Docs), &r, default, &cands, &p, &bad);
        assert_eq!(m, default);
        assert!(matches!(why, Why::ConfiguredWeak { ok: 0, n: 5, .. }));
        assert_eq!(
            cheapest(&cands, &p).map(String::as_str),
            Some("deepseek/deepseek-v3")
        );
    }

    #[test]
    fn decision_log_merges_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("routing.jsonl");
        let d = Decision {
            id: "a".into(),
            at: 1,
            category: Some(Category::Review),
            model: "x/y".into(),
            reason: "r".into(),
            ..Decision::default()
        };
        append(&f, &d).unwrap();
        append(
            &f,
            &Decision {
                id: "a".into(),
                outcome: Some(true),
                ..Decision::default()
            },
        )
        .unwrap();
        let all = decisions(&f);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].outcome, Some(true));
        assert_eq!(history(&all, Category::Review, "x/y"), (1, 1));
    }

    #[test]
    fn budget_alerts_once_then_acts_on_the_limit() {
        let l = Localizer::new(Lang::En);
        let mut b = Budget {
            daily_limit_usd: 5.0,
            ..Budget::default()
        };
        let mut f = BudgetFlags::default();
        assert_eq!(spend_state(&l, &b, 1.0, 1.0, false, &mut f), Spend::Ok);
        let Spend::Alert { fields, .. } = spend_state(&l, &b, 4.2, 4.2, true, &mut f) else {
            panic!("expected an alert at 84%");
        };
        assert_eq!(fields["scope"], "daily");
        assert_eq!(
            spend_state(&l, &b, 4.5, 4.5, true, &mut f),
            Spend::Ok,
            "alert fires once"
        );
        let Spend::Over { action, .. } = spend_state(&l, &b, 5.0, 5.0, false, &mut f) else {
            panic!("expected the limit");
        };
        assert_eq!(action, kor_agent::cost::OnLimit::Ask);
        f.allowed_over = true;
        assert_eq!(spend_state(&l, &b, 6.0, 6.0, false, &mut f), Spend::Ok);

        b.on_limit = OnLimit::Downgrade;
        let mut f = BudgetFlags::default();
        assert!(matches!(
            spend_state(&l, &b, 6.0, 6.0, false, &mut f),
            Spend::Over {
                action: kor_agent::cost::OnLimit::Downgrade,
                ..
            }
        ));
        assert_eq!(spend_state(&l, &b, 6.0, 6.0, false, &mut f), Spend::Ok);
        b.on_limit = OnLimit::Pause;
        let mut f = BudgetFlags::default();
        for _ in 0..2 {
            assert!(matches!(
                spend_state(&l, &b, 6.0, 6.0, false, &mut f),
                Spend::Over { .. }
            ));
        }
        assert_eq!(
            spend_state(&l, &Budget::default(), 99.0, 99.0, false, &mut f),
            Spend::Ok,
            "no limits, no checks"
        );
    }

    #[test]
    fn rows_by_groups_on_any_key() {
        let p = prices();
        let now = now_ms();
        let e = |path: Option<&str>, cost: f64| Entry {
            at: now,
            model: "x/y".into(),
            prompt: 1,
            completion: 1,
            cost_usd: Some(cost),
            path: path.map(str::to_owned),
            role: None,
            media: false,
            estimated: false,
            cache_read: 0,
            cache_write: 0,
        };
        let entries = [e(Some("a/1"), 0.5), e(Some("a/2"), 0.25), e(None, 1.0)];
        let rows = rows_by(&entries, &p, now, 30, |e| {
            e.path.as_deref().map_or("-", |p| &p[..1]).to_owned()
        });
        let got: Vec<(&str, f64, u64)> = rows
            .iter()
            .map(|r| (r.model.as_str(), r.usd, r.requests))
            .collect();
        assert_eq!(got, [("-", 1.0, 1), ("a", 0.75, 2)]);
    }

    #[test]
    fn report_groups_and_projects_and_exports() {
        let p = prices();
        let now = now_ms();
        let e = |model: &str, cost: Option<f64>| Entry {
            at: now,
            model: model.into(),
            prompt: 1_000_000,
            completion: 0,
            cost_usd: cost,
            path: None,
            role: None,
            media: false,
            estimated: false,
            cache_read: 0,
            cache_write: 0,
        };
        let entries = vec![
            e("anthropic/claude-sonnet", None),
            e("openrouter/deepseek/deepseek-v3", Some(0.25)),
            e("local/qwen", None),
            e("x/unknown", None),
        ];
        let r = report(&entries, &p, now, 30, &Budget::default());
        assert_eq!(r.rows.len(), 4);
        assert_eq!(r.rows[0].model, "anthropic/claude-sonnet");
        assert!((r.total.usd - 3.25).abs() < 1e-9);
        assert!(r.total.estimated);
        assert_eq!(r.total.unpriced, 1);
        assert!((r.today.usd - 3.25).abs() < 1e-9);
        assert!(r.projection_usd >= r.month.usd);
        let csv = to_csv(&r);
        assert!(csv.starts_with("model,requests"));
        assert!(csv.contains("\"local/qwen\",1,1000000,0,0.000000,false,0"));
        let l = Localizer::new(Lang::En);
        let lines = report_lines(&l, &r);
        assert!(lines.iter().any(|x| x.contains("≈$3.00")));
        assert!(
            lines
                .iter()
                .any(|x| x.contains("$0.25") && !x.contains("≈$0.25"))
        );
        assert_eq!(fmt_usd(1.5, true), "≈$1.50");
    }

    #[test]
    fn media_spending_has_its_own_line() {
        let p = prices();
        let now = now_ms();
        let media = Entry {
            at: now,
            model: "fal/fal-ai/flux/schnell".into(),
            prompt: 0,
            completion: 0,
            cost_usd: Some(0.4),
            path: None,
            role: None,
            media: true,
            estimated: true,
            cache_read: 0,
            cache_write: 0,
        };
        let r = report(
            std::slice::from_ref(&media),
            &p,
            now,
            30,
            &Budget::default(),
        );
        assert_eq!(r.media.requests, 1);
        assert!(r.media.estimated, "an estimated list price stays marked");
        let lines = report_lines(&Localizer::new(Lang::En), &r);
        assert!(
            lines
                .iter()
                .any(|x| x.contains("≈$0.40") && x.to_lowercase().contains("media")),
            "{lines:?}"
        );
    }

    #[test]
    fn month_periods_are_ordered() {
        let now = now_ms();
        let (day, month, len) = periods(now);
        assert!(month <= day && day <= now);
        assert!((28..=31).contains(&len));
    }
}
