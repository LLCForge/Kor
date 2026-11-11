//! `kor bench` (spec §5.1, §15.12, §14 Faz 2; ADR 0015): race models on the user's own test suite.
//!
//! A bench spec (`.kor/bench.toml`) lists tasks: a prompt, an optional setup command and the test
//! command that decides pass/fail. Every (task, model) job runs in a fresh copy of the project in
//! the temp folder, so the user's working tree is never touched. Each row records pass/fail, wall
//! time, provider-reported tokens, cost from catalog prices (`≈` when anything was estimated) and
//! tool-call outcomes from the repair layer (native or text-emulated).
//!
//! Router comparison: the same tasks run once on a single baseline model and once with each task's
//! category routed to its `[routing]` model; the report gives the cost and pass-rate deltas
//! against the Faz 2 acceptance check (at least 40% cheaper, at most 5 points fewer passes).

use kor_platform::NoWindow as _;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest};
use kor_core::{Cost, PermissionMode};
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use kor_providers::model_catalog::{self as mc, ModelData};
use serde::{Deserialize, Serialize};

/// Default spec location under the project root.
pub const SPEC_FILE: &str = ".kor/bench.toml";
/// `--max-cost` default: a run estimated above this needs confirmation.
pub const DEFAULT_MAX_COST_USD: f64 = 0.50;
/// Tokens assumed per job for the pre-run estimate (an agent task reads files and iterates).
pub const EST_INPUT_TOKENS: u64 = 40_000;
pub const EST_OUTPUT_TOKENS: u64 = 4_000;
/// Acceptance check of Faz 2: routed cost at most 60% of the baseline, pass rate at most 5 points lower.
pub const MAX_COST_RATIO: f64 = 0.60;
pub const MAX_PASS_DROP_PP: f64 = 5.0;

const MAX_TASKS: usize = 200;
const MAX_TEXT: usize = 16 * 1024;
const CMD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

// ---------------------------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spec {
    /// Models raced when the command line names none (`provider/model`).
    pub models: Vec<String>,
    /// Default setup command, run in the copy before the agent (e.g. `npm ci`).
    pub setup: Option<String>,
    /// Default test command; exit code 0 = pass.
    pub test: Option<String>,
    /// Category → model for router comparison when the command line gives no `--route`.
    pub routing: BTreeMap<String, String>,
    #[serde(rename = "task")]
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub name: String,
    /// Routing category (`simple_edit`, `complex_refactor`, `test_generation`, `docs`, `review`,
    /// `planning`, ...).
    #[serde(default = "default_category")]
    pub category: String,
    pub prompt: String,
    pub setup: Option<String>,
    pub test: Option<String>,
}

fn default_category() -> String {
    "simple_edit".into()
}

fn ident(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl Spec {
    /// Parse and validate a spec.
    ///
    /// # Errors
    /// Invalid TOML or a rule broken (no tasks, duplicate names, missing test command, too long).
    pub fn parse(text: &str) -> Result<Self, String> {
        let s: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if s.tasks.is_empty() {
            return Err("no [[task]] entries".into());
        }
        if s.tasks.len() > MAX_TASKS {
            return Err(format!("more than {MAX_TASKS} tasks"));
        }
        let mut names = BTreeSet::new();
        for t in &s.tasks {
            if !ident(&t.name) || !names.insert(t.name.as_str()) {
                return Err(format!(
                    "task name `{}` is empty, invalid or repeated",
                    t.name
                ));
            }
            if !ident(&t.category) {
                return Err(format!("task `{}`: invalid category", t.name));
            }
            if t.prompt.trim().is_empty() || t.prompt.len() > MAX_TEXT {
                return Err(format!("task `{}`: prompt is empty or too long", t.name));
            }
            if s.test_for(t).is_none() {
                return Err(format!("task `{}`: no test command", t.name));
            }
        }
        Ok(s)
    }

    /// `<root>/.kor/bench.toml` or an explicit file.
    ///
    /// # Errors
    /// Unreadable or invalid file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn test_for<'a>(&'a self, t: &'a Task) -> Option<&'a str> {
        t.test
            .as_deref()
            .or(self.test.as_deref())
            .filter(|c| !c.trim().is_empty())
    }

    fn setup_for<'a>(&'a self, t: &'a Task) -> Option<&'a str> {
        t.setup
            .as_deref()
            .or(self.setup.as_deref())
            .filter(|c| !c.trim().is_empty())
    }
}

// ---------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------

/// Which run a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arm {
    /// Plain model race.
    Race,
    /// Router off: every task on the baseline model.
    Baseline,
    /// Router on: each task on its category's model.
    Routed,
    /// `--graph`: graph tools removed (spec §7.5).
    #[serde(rename = "graph-off")]
    GraphOff,
    /// `--graph`: graph tools on.
    #[serde(rename = "graph-on")]
    GraphOn,
}

impl Arm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Race => "race",
            Self::Baseline => "baseline",
            Self::Routed => "routed",
            Self::GraphOff => "graph-off",
            Self::GraphOn => "graph-on",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Index into [`Spec::tasks`].
    pub task: usize,
    pub model: String,
    pub arm: Arm,
}

/// Every task on every model.
pub fn race_jobs(spec: &Spec, models: &[String]) -> Vec<Job> {
    (0..spec.tasks.len())
        .flat_map(|task| {
            models.iter().map(move |m| Job {
                task,
                model: m.clone(),
                arm: Arm::Race,
            })
        })
        .collect()
}

/// Router comparison: each task on `baseline`, then on `routes[category]` (else
/// `routes["fallback"]`, else the baseline).
///
/// Hook-up point for the cost router (ADR 0013): `routes` is the per-run model override; the CLI
/// and desktop fill it from `--route` or the spec's `[routing]` table. Baseline jobs always run
/// with the router off (`Agent::cost = None`); routed jobs keep whatever router `prepare` attached.
pub fn router_jobs(spec: &Spec, baseline: &str, routes: &BTreeMap<String, String>) -> Vec<Job> {
    let mut jobs = Vec::new();
    for (task, t) in spec.tasks.iter().enumerate() {
        jobs.push(Job {
            task,
            model: baseline.to_owned(),
            arm: Arm::Baseline,
        });
        let target = routes
            .get(&t.category)
            .or_else(|| routes.get("fallback"))
            .map_or(baseline, String::as_str);
        jobs.push(Job {
            task,
            model: target.to_owned(),
            arm: Arm::Routed,
        });
    }
    jobs
}

/// `kor bench --graph`: each task on each model with the graph tools off and on (`on`/`off` alone
/// run just that arm).
pub fn graph_jobs(spec: &Spec, models: &[String], off: bool, on: bool) -> Vec<Job> {
    let arms: Vec<Arm> = [(off, Arm::GraphOff), (on, Arm::GraphOn)]
        .into_iter()
        .filter_map(|(want, a)| want.then_some(a))
        .collect();
    let mut jobs = Vec::new();
    for task in 0..spec.tasks.len() {
        for m in models {
            for arm in &arms {
                jobs.push(Job {
                    task,
                    model: m.clone(),
                    arm: *arm,
                });
            }
        }
    }
    jobs
}

// ---------------------------------------------------------------------------------------------
// Prices
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Price {
    /// Runs on this machine.
    Free,
    /// USD per million input / output tokens, from the model catalog.
    PerM {
        input: f64,
        output: f64,
    },
    Unknown,
}

/// Catalog prices (cache only, never fetched here) and the ids of local providers.
#[derive(Debug, Clone, Default)]
pub struct Prices {
    pub models: Vec<ModelData>,
    pub local_providers: BTreeSet<String>,
}

impl Prices {
    /// Cached catalog (`kor catalog` fills it) plus the registry's local providers.
    pub fn load(l: &Localizer, catalog_cache: Option<&Path>) -> Self {
        let models = catalog_cache
            .and_then(mc::read_cache)
            .map(|c| c.catalog.models)
            .unwrap_or_default();
        let local_providers = crate::registry(l, &mut Vec::new())
            .map(|r| {
                r.iter()
                    .filter(|m| crate::is_local_provider(m))
                    .map(|m| m.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            models,
            local_providers,
        }
    }

    pub fn price(&self, model: &str) -> Price {
        let (provider, rest) = model.split_once('/').unwrap_or(("", model));
        if self.local_providers.contains(provider) {
            return Price::Free;
        }
        let found = mc::lookup(&self.models, model).or_else(|| mc::lookup(&self.models, rest));
        match found.map(|m| (m.input_price_per_m, m.output_price_per_m)) {
            Some((Some(i), Some(o))) if i >= 0.0 && o >= 0.0 && i.is_finite() && o.is_finite() => {
                Price::PerM {
                    input: i,
                    output: o,
                }
            }
            _ => Price::Unknown,
        }
    }

    /// Cost of a token count; `None` when the price is unknown.
    pub fn cost(&self, model: &str, input: u64, output: u64) -> Option<Cost> {
        match self.price(model) {
            Price::Free => Some(Cost::default()),
            Price::PerM {
                input: pi,
                output: po,
            } => {
                let mut c = Cost::for_tokens(input, micro(pi));
                c += Cost::for_tokens(output, micro(po));
                Some(c)
            }
            Price::Unknown => None,
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // checked finite and >= 0
fn micro(usd_per_m: f64) -> u64 {
    (usd_per_m * 1_000_000.0).round() as u64
}

/// Pre-run cost estimate (always `≈`): [`EST_INPUT_TOKENS`]/[`EST_OUTPUT_TOKENS`] per job.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Estimate {
    pub usd: f64,
    /// Remote models without a catalog price: their cost cannot be bounded.
    pub unknown: Vec<String>,
}

pub fn estimate(jobs: &[Job], prices: &Prices) -> Estimate {
    let mut total = Cost::default();
    let mut unknown = BTreeSet::new();
    for j in jobs {
        match prices.cost(&j.model, EST_INPUT_TOKENS, EST_OUTPUT_TOKENS) {
            Some(c) => total += c,
            None => {
                unknown.insert(j.model.clone());
            }
        }
    }
    Estimate {
        usd: total.usd(),
        unknown: unknown.into_iter().collect(),
    }
}

/// Budget guard: `true` when the run may start without confirmation.
pub fn within_budget(e: &Estimate, max_usd: f64) -> bool {
    e.unknown.is_empty() && e.usd <= max_usd
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub task: String,
    pub category: String,
    pub model: String,
    pub arm: Arm,
    pub passed: bool,
    /// Agent plus test command, in milliseconds.
    pub wall_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The provider reported no usage; tokens were estimated from the transcript.
    pub tokens_estimated: bool,
    /// `None` when the model's price is unknown.
    pub cost_usd: Option<f64>,
    /// Shown with `≈`: tokens estimated, or cache tokens priced at the input rate.
    pub cost_estimated: bool,
    /// `native` or `text` (emulated tool calling), as the agent ended.
    pub tool_mode: String,
    pub tool_calls_ok: u64,
    pub tool_calls_repaired: u64,
    pub tool_calls_failed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Progress {
    Start {
        index: usize,
        total: usize,
        task: String,
        model: String,
        arm: Arm,
    },
    Done {
        index: usize,
        total: usize,
        row: Row,
    },
}

/// Builds an agent for `model` rooted at the given copy of the project.
pub type MakeAgent<'a> = dyn Fn(&Path, &str) -> Result<Agent, String> + Send + Sync + 'a;

struct DenyAll;

#[async_trait]
impl Approver for DenyAll {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

/// A throwaway copy of the project; removed on drop together with the per-run undo journal and
/// memory folder Kor keeps for that path.
#[derive(Debug)]
pub struct Workdir {
    pub id: String,
    pub path: PathBuf,
}

impl Workdir {
    /// Copy `root` (respecting `.gitignore`, without `.git` and links) into the temp folder.
    ///
    /// # Errors
    /// The copy failed.
    pub fn copy(root: &Path) -> std::io::Result<Self> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("bench-{}-{nanos}-{n}", std::process::id());
        let path = std::env::temp_dir().join("kor-bench").join(&id);
        let w = Self { id, path };
        std::fs::create_dir_all(&w.path)?;
        let walk = ignore::WalkBuilder::new(root)
            .hidden(false)
            .require_git(false)
            .filter_entry(|e| e.file_name() != ".git")
            .build();
        for e in walk {
            let e = e.map_err(std::io::Error::other)?;
            let Ok(rel) = e.path().strip_prefix(root) else {
                continue;
            };
            let dst = w.path.join(rel);
            match e.file_type() {
                Some(t) if t.is_dir() => std::fs::create_dir_all(&dst)?,
                Some(t) if t.is_file() => {
                    std::fs::copy(e.path(), &dst)?;
                }
                _ => {}
            }
        }
        Ok(w)
    }

    /// Canonical root for the agent.
    ///
    /// # Errors
    /// The copy vanished.
    pub fn root(&self) -> std::io::Result<PathBuf> {
        Ok(kor_tools::without_verbatim(&self.path.canonicalize()?))
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        if let Some(home) = crate::kor_home() {
            let _ = std::fs::remove_dir_all(home.join("undo").join(&self.id));
            if let Ok(root) = self.root() {
                let lay = kor_agent::memory::layout(&home, &root);
                let _ = std::fs::remove_dir_all(lay.project_dir);
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Run a shell command in `dir` without inherited secrets; `Ok(true)` on exit code 0.
/// With `confine`, the command runs in that session's sandbox (ADR 0033): the test runs code
/// the model wrote. The spec's own setup command runs unconfined, like the user's shell.
async fn shell(
    dir: &Path,
    cmd: &str,
    scrub: &[String],
    cancel: &CancellationToken,
    confine: Option<&kor_tools::ToolCtx>,
) -> Result<bool, String> {
    let kind = confine.map_or(kor_sandbox::SandboxKind::None, |c| c.sandbox);
    let policy = confine.map_or_else(
        || kor_sandbox::Policy::new(dir),
        kor_tools::ToolCtx::sandbox_policy,
    );
    let sh = if kind == kor_sandbox::SandboxKind::Docker {
        kor_tools::Shell::container()
    } else {
        kor_tools::Shell::detect()
    };
    let mut c = tokio::process::Command::new(&sh.program);
    c.no_window();
    c.args(&sh.prefix).arg(cmd).current_dir(dir);
    // Model-written code runs in the test: provider keys stay out of its environment.
    for k in scrub {
        c.env_remove(k);
    }
    let child = kor_sandbox::spawn(kind, &policy, &c).map_err(|e| e.to_string())?;
    tokio::select! {
        r = tokio::time::timeout(CMD_TIMEOUT, child.wait_with_output()) => match r {
            Ok(Ok(s)) => Ok(s.status.success()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(format!("timed out after {} s", CMD_TIMEOUT.as_secs())),
        },
        () = cancel.cancelled() => Err("cancelled".into()),
    }
}

fn row_for(spec: &Spec, job: &Job) -> Row {
    let t = &spec.tasks[job.task];
    Row {
        task: t.name.clone(),
        category: t.category.clone(),
        model: job.model.clone(),
        arm: job.arm,
        passed: false,
        wall_ms: 0,
        input_tokens: 0,
        output_tokens: 0,
        tokens_estimated: false,
        cost_usd: None,
        cost_estimated: false,
        tool_mode: String::new(),
        tool_calls_ok: 0,
        tool_calls_repaired: 0,
        tool_calls_failed: 0,
        error: None,
    }
}

/// One job in a fresh copy. Errors land in [`Row::error`], never abort the run.
async fn run_job(
    l: &Localizer,
    root: &Path,
    spec: &Spec,
    job: &Job,
    make: &MakeAgent<'_>,
    prices: &Prices,
    cancel: &CancellationToken,
) -> Row {
    let mut row = row_for(spec, job);
    let t = &spec.tasks[job.task];
    let err = |key: &str, e: String| l.t_args(key, &args!("error" => e));
    let work = match Workdir::copy(root) {
        Ok(w) => w,
        Err(e) => {
            row.error = Some(err("bench-err-copy", e.to_string()));
            return row;
        }
    };
    let mut agent = match work
        .root()
        .map_err(|e| e.to_string())
        .and_then(|r| make(&r, &job.model))
    {
        Ok(a) => a,
        Err(e) => {
            row.error = Some(err("bench-err-agent", e));
            return row;
        }
    };
    // Only the routed arm may let the cost router (ADR 0013) switch models; a race or baseline
    // row must measure exactly the model it names.
    if job.arm != Arm::Routed {
        agent.cost = None;
    }
    match job.arm {
        Arm::GraphOff => kor_agent::graph::strip(&mut agent),
        Arm::GraphOn => kor_agent::graph::ensure(
            &mut agent,
            kor_agent::graph::Options {
                watch: true,
                ..kor_agent::graph::Options::default()
            },
        ),
        _ => {}
    }
    let scrub = agent.ctx.scrub_env.clone();
    if let Some(setup) = spec.setup_for(t) {
        match shell(&work.path, setup, &scrub, cancel, None).await {
            Ok(true) => {}
            Ok(false) => {
                row.error = Some(err("bench-err-setup", "exit code".into()));
                return row;
            }
            Err(e) => {
                row.error = Some(err("bench-err-setup", e));
                return row;
            }
        }
    }
    let started = Instant::now();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    let sent = agent
        .send(t.prompt.clone(), &tx, &DenyAll, cancel.clone())
        .await;
    if let Err(e) = sent {
        row.error = Some(err("bench-err-agent", e.to_string()));
    }
    if let Some(test) = spec.test_for(t) {
        match shell(&work.path, test, &scrub, cancel, Some(&agent.ctx)).await {
            Ok(ok) => row.passed = ok && row.error.is_none(),
            Err(e) => row.error = Some(err("bench-err-test", e)),
        }
    }
    row.wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let u = agent.usage;
    let cached = u.cache_read.saturating_add(u.cache_write);
    if u.input == 0 && u.output == 0 && cached == 0 {
        // No usage from the provider: estimate from the transcript, marked ≈.
        let assistant: Vec<_> = agent
            .messages
            .iter()
            .filter(|m| m.role == kor_core::Role::Assistant)
            .cloned()
            .collect();
        row.output_tokens = kor_agent::compact::estimate_tokens(&assistant);
        row.input_tokens = kor_agent::compact::estimate_tokens(&agent.messages);
        row.tokens_estimated = true;
    } else {
        row.input_tokens = u.input.saturating_add(cached);
        row.output_tokens = u.output;
    }
    row.cost_usd = prices
        .cost(&job.model, row.input_tokens, row.output_tokens)
        .map(Cost::usd);
    row.cost_estimated = row.tokens_estimated || (cached > 0 && row.cost_usd.unwrap_or(0.0) > 0.0);
    row.tool_mode = match agent.tool_mode {
        kor_agent::toolcall::ToolMode::Native => "native",
        kor_agent::toolcall::ToolMode::Text => "text",
    }
    .into();
    row.tool_calls_ok = agent.stats.tool_calls_ok;
    row.tool_calls_repaired = agent.stats.tool_calls_repaired;
    row.tool_calls_failed = agent.stats.tool_calls_failed;
    row
}

/// Run `jobs` one after another (sequential, so wall times are comparable). A cancelled run
/// returns the rows finished so far.
#[allow(clippy::too_many_arguments)] // each is a separate input the CLI and desktop pass through
pub async fn run(
    l: &Localizer,
    root: &Path,
    spec: &Spec,
    jobs: &[Job],
    make: &MakeAgent<'_>,
    prices: &Prices,
    progress: &(dyn Fn(Progress) + Send + Sync),
    cancel: &CancellationToken,
) -> Vec<Row> {
    let total = jobs.len();
    let mut rows = Vec::with_capacity(total);
    for (index, job) in jobs.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        progress(Progress::Start {
            index,
            total,
            task: spec.tasks[job.task].name.clone(),
            model: job.model.clone(),
            arm: job.arm,
        });
        let row = run_job(l, root, spec, job, make, prices, cancel).await;
        progress(Progress::Done {
            index,
            total,
            row: row.clone(),
        });
        rows.push(row);
    }
    rows
}

/// The real agent factory: the user's config, accept-edits mode (commands still follow the
/// user's own allow rules; nothing asks), a fresh session id per copy.
pub fn runtime_agent<'a>(
    l: &'a Localizer,
    loaded: &'a kor_config::Loaded,
) -> impl Fn(&Path, &str) -> Result<Agent, String> + Send + Sync + 'a {
    move |root: &Path, model: &str| {
        let id = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| crate::valid_session_id(s));
        let opts = crate::Options {
            root: root.to_owned(),
            model: Some(model.to_owned()),
            mode: Some(PermissionMode::AcceptEdits),
            resume: None,
            new_id: id,
            surface: kor_agent::prompt::Surface::default(),
        };
        // No knowledge base recall/learning: runs must not see or record each other (§31.3).
        crate::prepare(l, loaded, &opts, &mut Vec::new()).map(|mut p| {
            p.agent.context.recall = None;
            p.agent
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub model: String,
    pub arm: Option<Arm>,
    pub tasks: u64,
    pub passed: u64,
    pub pass_rate: f64,
    pub wall_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// `None` when any row's price is unknown.
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
    pub tool_calls_ok: u64,
    pub tool_calls_repaired: u64,
    pub tool_calls_failed: u64,
    /// (ok + repaired) / all calls; `None` without calls.
    pub tool_success_rate: Option<f64>,
    /// Jobs that ended on text-emulated tool calling.
    pub emulated: u64,
}

#[allow(clippy::cast_precision_loss)] // counts far below 2^52
fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

fn summarize_rows<'a>(
    model: &str,
    arm: Option<Arm>,
    rows: impl Iterator<Item = &'a Row>,
) -> Summary {
    let mut s = Summary {
        model: model.to_owned(),
        arm,
        cost_usd: Some(0.0),
        ..Summary::default()
    };
    for r in rows {
        s.tasks += 1;
        s.passed += u64::from(r.passed);
        s.wall_ms = s.wall_ms.saturating_add(r.wall_ms);
        s.input_tokens = s.input_tokens.saturating_add(r.input_tokens);
        s.output_tokens = s.output_tokens.saturating_add(r.output_tokens);
        s.cost_usd = s.cost_usd.zip(r.cost_usd).map(|(a, b)| a + b);
        s.cost_estimated |= r.cost_estimated;
        s.tool_calls_ok += r.tool_calls_ok;
        s.tool_calls_repaired += r.tool_calls_repaired;
        s.tool_calls_failed += r.tool_calls_failed;
        s.emulated += u64::from(r.tool_mode == "text");
    }
    s.pass_rate = ratio(s.passed, s.tasks);
    let calls = s.tool_calls_ok + s.tool_calls_repaired + s.tool_calls_failed;
    s.tool_success_rate =
        (calls > 0).then(|| ratio(s.tool_calls_ok + s.tool_calls_repaired, calls));
    s
}

/// One summary per (arm, model).
pub fn summarize(rows: &[Row]) -> Vec<Summary> {
    let keys: BTreeSet<(Arm, &str)> = rows.iter().map(|r| (r.arm, r.model.as_str())).collect();
    keys.into_iter()
        .map(|(arm, model)| {
            summarize_rows(
                model,
                Some(arm),
                rows.iter().filter(|r| r.arm == arm && r.model == model),
            )
        })
        .collect()
}

/// Cheapest model per category among those within [`MAX_PASS_DROP_PP`] of the category's best
/// pass rate (ties: faster); `fallback` the same over all tasks. Categories nobody passed get none.
pub fn suggest_routing(rows: &[Row]) -> BTreeMap<String, String> {
    let pick = |rows: &[&Row]| -> Option<String> {
        let models: BTreeSet<&str> = rows.iter().map(|r| r.model.as_str()).collect();
        let sums: Vec<Summary> = models
            .into_iter()
            .map(|m| summarize_rows(m, None, rows.iter().copied().filter(|r| r.model == m)))
            .collect();
        let best = sums.iter().map(|s| s.pass_rate).fold(0.0, f64::max);
        if best <= 0.0 {
            return None;
        }
        let per_task = |s: &Summary| s.cost_usd.map_or(f64::INFINITY, |c| c * ratio(1, s.tasks));
        sums.iter()
            .filter(|s| s.pass_rate * 100.0 >= best * 100.0 - MAX_PASS_DROP_PP)
            .min_by(|a, b| {
                per_task(a)
                    .total_cmp(&per_task(b))
                    .then(b.pass_rate.total_cmp(&a.pass_rate))
                    .then(a.wall_ms.cmp(&b.wall_ms))
            })
            .map(|s| s.model.clone())
    };
    let race: Vec<&Row> = rows.iter().filter(|r| r.arm == Arm::Race).collect();
    let cats: BTreeSet<&str> = race.iter().map(|r| r.category.as_str()).collect();
    let mut out = BTreeMap::new();
    for c in cats {
        let in_cat: Vec<&Row> = race.iter().copied().filter(|r| r.category == c).collect();
        if let Some(m) = pick(&in_cat) {
            out.insert(c.to_owned(), m);
        }
    }
    if let Some(m) = pick(&race) {
        out.insert("fallback".into(), m);
    }
    out
}

/// `[routing]` TOML in the spec §5.1 format. `fallback` is the bench's best overall model; in the
/// config the key is a list (`fallback = ["..."]`), written that way here.
pub fn routing_toml(routes: &BTreeMap<String, String>) -> String {
    let mut s = String::from("[routing]\n");
    for (k, v) in routes {
        let v = toml::Value::String(v.clone());
        let _ = if k == "fallback" {
            writeln!(s, "fallback = [{v}]")
        } else {
            writeln!(s, "{k} = {v}")
        };
    }
    s
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    pub baseline: Summary,
    pub routed: Summary,
    /// (routed − baseline) / baseline × 100; negative = cheaper. `None` if a cost is unknown or zero.
    pub cost_delta_pct: Option<f64>,
    /// Routed pass rate minus baseline pass rate, in percentage points.
    pub pass_delta_pp: f64,
    /// At least 40% cheaper and at most 5 points fewer passes; `None` when cost is unknown.
    pub meets_acceptance: Option<bool>,
}

pub fn compare(rows: &[Row]) -> Option<Comparison> {
    let side = |arm| {
        let rs: Vec<&Row> = rows.iter().filter(|r| r.arm == arm).collect();
        (!rs.is_empty()).then(|| summarize_rows(arm.as_str(), Some(arm), rs.into_iter()))
    };
    let (baseline, routed) = (side(Arm::Baseline)?, side(Arm::Routed)?);
    let cost_delta_pct = baseline
        .cost_usd
        .zip(routed.cost_usd)
        .filter(|(b, _)| *b > 0.0)
        .map(|(b, r)| (r - b) / b * 100.0);
    let pass_delta_pp = (routed.pass_rate - baseline.pass_rate) * 100.0;
    let meets_acceptance = cost_delta_pct
        .map(|d| d <= (MAX_COST_RATIO - 1.0) * 100.0 && pass_delta_pp >= -MAX_PASS_DROP_PP);
    Some(Comparison {
        baseline,
        routed,
        cost_delta_pct,
        pass_delta_pp,
        meets_acceptance,
    })
}

/// `--graph` comparison (spec §7.5, §14 Faz 3: at least 50% fewer tokens with the graph).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphComparison {
    pub off: Summary,
    pub on: Summary,
    /// (on − off) / off × 100 of input + output tokens; negative = fewer.
    pub token_delta_pct: Option<f64>,
    pub cost_delta_pct: Option<f64>,
    pub pass_delta_pp: f64,
    /// At least 50% fewer tokens and at most 5 points fewer passes.
    pub meets_acceptance: Option<bool>,
}

/// Minimum token cut of the graph arm (spec §14 Faz 3).
pub const GRAPH_MIN_TOKEN_CUT_PCT: f64 = 50.0;

pub fn compare_graph(rows: &[Row]) -> Option<GraphComparison> {
    let side = |arm| {
        let rs: Vec<&Row> = rows.iter().filter(|r| r.arm == arm).collect();
        (!rs.is_empty()).then(|| summarize_rows(arm.as_str(), Some(arm), rs.into_iter()))
    };
    let (off, on) = (side(Arm::GraphOff)?, side(Arm::GraphOn)?);
    let pct = |b: f64, a: f64| (b > 0.0).then(|| (a - b) / b * 100.0);
    #[allow(clippy::cast_precision_loss)] // token counts far below 2^52
    let tokens = |s: &Summary| (s.input_tokens + s.output_tokens) as f64;
    let token_delta_pct = pct(tokens(&off), tokens(&on));
    let cost_delta_pct = off.cost_usd.zip(on.cost_usd).and_then(|(b, a)| pct(b, a));
    let pass_delta_pp = (on.pass_rate - off.pass_rate) * 100.0;
    let meets_acceptance = token_delta_pct
        .map(|d| d <= -GRAPH_MIN_TOKEN_CUT_PCT && pass_delta_pp >= -MAX_PASS_DROP_PP);
    Some(GraphComparison {
        off,
        on,
        token_delta_pct,
        cost_delta_pct,
        pass_delta_pp,
        meets_acceptance,
    })
}

/// Everything the CLI table, the JSON export and the desktop show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub summary: Vec<Summary>,
    pub routing: BTreeMap<String, String>,
    pub routing_toml: String,
    pub comparison: Option<Comparison>,
    /// `--graph` arms side by side.
    pub graph: Option<GraphComparison>,
}

pub fn report(rows: Vec<Row>) -> Report {
    let routing = suggest_routing(&rows);
    Report {
        summary: summarize(&rows),
        routing_toml: if routing.is_empty() {
            String::new()
        } else {
            routing_toml(&routing)
        },
        routing,
        comparison: compare(&rows),
        graph: compare_graph(&rows),
        rows,
    }
}

/// `$0.0123`, `≈$0.0123` or `?`.
pub fn money(usd: Option<f64>, estimated: bool) -> String {
    match usd {
        Some(v) => format!("{}${v:.4}", if estimated { "≈" } else { "" }),
        None => "?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
test = "exit 0"
models = ["local/a"]
[[task]]
name = "one"
prompt = "do it"
[[task]]
name = "two"
category = "docs"
prompt = "write docs"
test = "exit 1"
"#;

    fn row(task: &str, cat: &str, model: &str, arm: Arm, passed: bool, cost: Option<f64>) -> Row {
        Row {
            task: task.into(),
            category: cat.into(),
            model: model.into(),
            arm,
            passed,
            wall_ms: 10,
            input_tokens: 100,
            output_tokens: 10,
            tokens_estimated: false,
            cost_usd: cost,
            cost_estimated: false,
            tool_mode: "native".into(),
            tool_calls_ok: 1,
            tool_calls_repaired: 1,
            tool_calls_failed: 0,
            error: None,
        }
    }

    #[test]
    fn spec_validates() {
        let s = Spec::parse(SPEC).unwrap();
        assert_eq!(s.tasks[0].category, "simple_edit");
        assert_eq!(s.test_for(&s.tasks[1]), Some("exit 1"));
        assert!(Spec::parse("").is_err(), "no tasks");
        assert!(
            Spec::parse("[[task]]\nname='a'\nprompt='p'").is_err(),
            "no test"
        );
        let dup = "test='t'\n[[task]]\nname='a'\nprompt='p'\n[[task]]\nname='a'\nprompt='q'";
        assert!(Spec::parse(dup).is_err());
        assert!(Spec::parse("test='t'\n[[task]]\nname='../x'\nprompt='p'").is_err());
        assert!(Spec::parse("bogus=1\ntest='t'\n[[task]]\nname='a'\nprompt='p'").is_err());
    }

    #[test]
    fn jobs_cover_models_and_routes() {
        let s = Spec::parse(SPEC).unwrap();
        let models = vec!["a/x".to_owned(), "b/y".to_owned()];
        assert_eq!(race_jobs(&s, &models).len(), 4);
        let routes = BTreeMap::from([("docs".to_owned(), "cheap/m".to_owned())]);
        let j = router_jobs(&s, "big/m", &routes);
        let picked: Vec<_> = j
            .iter()
            .filter(|j| j.arm == Arm::Routed)
            .map(|j| j.model.as_str())
            .collect();
        assert_eq!(picked, ["big/m", "cheap/m"]);
    }

    #[test]
    fn prices_local_known_unknown_and_budget() {
        let p = Prices {
            models: vec![ModelData {
                model_id: "acme/big".into(),
                input_price_per_m: Some(3.0),
                output_price_per_m: Some(15.0),
                ..ModelData::default()
            }],
            local_providers: BTreeSet::from(["local".to_owned()]),
        };
        assert_eq!(p.price("local/qwen"), Price::Free);
        // OpenRouter-style ids resolve by the model part.
        assert!(matches!(p.price("openrouter/acme/big"), Price::PerM { .. }));
        assert_eq!(p.price("other/x"), Price::Unknown);
        assert_eq!(
            p.cost("acme/big", 1_000_000, 100_000).unwrap().micro_usd,
            4_500_000
        );
        let jobs = |m: &str| {
            vec![Job {
                task: 0,
                model: m.into(),
                arm: Arm::Race,
            }]
        };
        assert!(
            within_budget(&estimate(&jobs("local/qwen"), &p), 0.0),
            "local is free"
        );
        let e = estimate(&jobs("acme/big"), &p);
        assert!((e.usd - 0.18).abs() < 1e-9);
        assert!(!within_budget(&e, DEFAULT_MAX_COST_USD - 0.4));
        assert!(
            !within_budget(&estimate(&jobs("other/x"), &p), 100.0),
            "unknown price"
        );
    }

    #[test]
    fn suggestion_prefers_cheapest_within_tolerance() {
        let rows = vec![
            row("a", "simple_edit", "big/m", Arm::Race, true, Some(1.0)),
            row("a", "simple_edit", "cheap/m", Arm::Race, true, Some(0.1)),
            row("b", "docs", "big/m", Arm::Race, true, Some(1.0)),
            row("b", "docs", "cheap/m", Arm::Race, false, Some(0.1)),
        ];
        let r = suggest_routing(&rows);
        assert_eq!(r["simple_edit"], "cheap/m");
        assert_eq!(r["docs"], "big/m");
        assert_eq!(r["fallback"], "big/m");
        let toml = routing_toml(&r);
        assert!(
            toml.contains("simple_edit = \"cheap/m\"") && toml.contains("fallback = [\"big/m\"]")
        );
        let parsed: toml::Table = toml::from_str(&toml).unwrap();
        assert!(parsed["routing"]["docs"].is_str());
    }

    #[test]
    fn comparison_checks_acceptance() {
        let rows = vec![
            row("a", "x", "big/m", Arm::Baseline, true, Some(1.0)),
            row("b", "x", "big/m", Arm::Baseline, true, Some(1.0)),
            row("a", "x", "cheap/m", Arm::Routed, true, Some(0.5)),
            row("b", "x", "cheap/m", Arm::Routed, true, Some(0.5)),
        ];
        let c = compare(&rows).unwrap();
        assert!((c.cost_delta_pct.unwrap() + 50.0).abs() < 1e-9);
        assert!(c.pass_delta_pp.abs() < 1e-9);
        assert_eq!(c.meets_acceptance, Some(true));
        let mut worse = rows.clone();
        worse[3].passed = false;
        assert_eq!(compare(&worse).unwrap().meets_acceptance, Some(false));
        worse[3].cost_usd = None;
        assert_eq!(compare(&worse).unwrap().meets_acceptance, None);
        assert!(compare(&rows[..2]).is_none());
        let s = &summarize(&rows)[0];
        assert_eq!((s.tasks, s.passed, s.tool_success_rate), (2, 2, Some(1.0)));
        assert_eq!(money(Some(0.5), true), "≈$0.5000");
        assert_eq!(money(None, false), "?");
    }

    #[test]
    fn workdir_copies_without_git_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "x").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("big"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("a.txt"), "hi").unwrap();
        let w = Workdir::copy(dir.path()).unwrap();
        let p = w.path.clone();
        assert_eq!(
            std::fs::read_to_string(p.join("src").join("a.txt")).unwrap(),
            "hi"
        );
        assert!(p.join(".gitignore").is_file());
        assert!(!p.join(".git").exists() && !p.join("target").exists());
        drop(w);
        assert!(!p.exists());
    }
}
