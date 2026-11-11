//! Multi-agent tasks (`kor task "<goal>" --agents N`, spec §8.2, §6.2, ADR 0037).
//!
//! An orchestrator model splits the goal into sub-tasks with roles and owned files (structured
//! output, `kor-core/schemas/task-plan.v1.json`). Each sub-agent runs in its own git worktree on
//! its own branch (`.kor/worktrees/<id>/<n>-<role>`, `kor/task/<id>/<n>-<role>`) with its own
//! model and budget; without a pinned model the smart cost router picks one per sub-task.
//! Sub-agents run in parallel (bounded), commands sandboxed like any session. A reviewer model
//! checks each result (diff + test output) before it is merged into the task branch
//! (`kor/task/<id>/merged`) in plan order. A conflicting merge is aborted, so nothing is
//! half-merged; the conflict is kept for the user (`resolve`: re-plan that part or drop it).
//! The task is checkpointed after every step in `SQLite` (`<repo>/.kor/tasks/checkpoints.sqlite`,
//! WAL, one row per task holding its JSON state, each save one transaction), so `resume`
//! continues after a crash and two processes never see half a checkpoint. Checkpoints written by
//! older versions as `<id>.json` are imported once (renamed `<id>.json.imported`).

pub mod git;
mod spawn;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use kor_agent::cost::{CostControl, OnLimit, Outcome, Plan as RoutePlan, Route, Spend};
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest, StopReason};
use kor_core::{Message, Role, StreamEvent, ToolSpec, Usage};
use kor_i18n::{Lang, Localizer, args};
use kor_providers::{CancellationToken, ChatRequest, Provider};
use kor_tools::ToolCtx;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use git::ConflictFile;
pub use spawn::{RuntimeSpawner, models};

use crate::cost::{self, now_ms};

/// Checkpoint format version.
const VERSION: u32 = 1;
/// Characters of test output kept.
const MAX_TEST_OUTPUT: usize = 8_000;

// ---------------------------------------------------------------------------------------------
// State (the checkpoint)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Planning,
    Running,
    Merging,
    /// A merge conflicted; waits for `resolve`.
    Conflict,
    Done,
    /// Some sub-task failed or was rejected, or tests failed; `resume` retries.
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    Pending,
    Running,
    /// Reviewed and approved; waits for its merge.
    Approved,
    /// The reviewer refused it; `resume` runs it again with the reviewer's notes.
    Rejected,
    Failed,
    Merged,
    Conflict,
    Dropped,
    Cancelled,
}

impl SubStatus {
    /// Runs (again) on `resume`.
    fn runnable(self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Running | Self::Rejected | Self::Failed | Self::Cancelled
        )
    }
}

/// Settings fixed at task start (CLI flags over `[orchestrator]`), kept for `resume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub agents: usize,
    pub max_parallel: usize,
    /// Pinned `provider/model` per role.
    pub models: BTreeMap<String, String>,
    pub budget_usd: f64,
    pub budget_per_agent_usd: f64,
    pub test_command: String,
    pub max_turns: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_config(&kor_config::Orchestrator::default())
    }
}

impl Settings {
    pub fn from_config(c: &kor_config::Orchestrator) -> Self {
        Self {
            agents: usize::from(c.agents.clamp(1, 16)),
            max_parallel: usize::from(c.max_parallel.clamp(1, 16)),
            models: c.models.clone(),
            budget_usd: c.budget_usd.max(0.0),
            budget_per_agent_usd: c.budget_per_agent_usd.max(0.0),
            test_command: c.test_command.trim().to_owned(),
            max_turns: c.max_turns.max(1),
        }
    }

    /// Budget of one sub-agent; 0 = none.
    fn agent_budget(&self, subtasks: usize) -> f64 {
        if self.budget_per_agent_usd > 0.0 {
            self.budget_per_agent_usd
        } else if self.budget_usd > 0.0 && subtasks > 0 {
            // At most 16 sub-tasks: exact in f64.
            #[allow(clippy::cast_precision_loss)]
            let n = subtasks as f64;
            self.budget_usd / n
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestRun {
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubTask {
    pub index: usize,
    pub role: String,
    pub goal: String,
    /// Files and directories this sub-agent owns (declared by the orchestrator).
    pub files: Vec<String>,
    pub status: SubStatus,
    pub branch: String,
    /// Relative to the repository root.
    pub worktree: String,
    /// Pinned `provider/model`, if any.
    pub model: Option<String>,
    /// Models that actually answered (router choices included).
    #[serde(default)]
    pub models_used: Vec<String>,
    /// Commit the worktree starts from; `None` = the task's base. A re-planned sub-task starts
    /// from the task branch.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub changed: Vec<String>,
    /// Changed files outside `files` (shown to the reviewer).
    #[serde(default)]
    pub outside: Vec<String>,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub estimated: bool,
    #[serde(default)]
    pub test: Option<TestRun>,
    #[serde(default)]
    pub review: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Transcript of the sub-agent's session.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub index: usize,
    pub role: String,
    pub files: Vec<ConflictFile>,
    /// What `git merge` printed with conflict markers (cut).
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    pub version: u32,
    pub id: String,
    pub goal: String,
    #[serde(default)]
    pub summary: String,
    pub status: TaskStatus,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Commit the task started from.
    pub base: String,
    /// Integration branch the sub-agents are merged into.
    pub branch: String,
    pub settings: Settings,
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
    #[serde(default)]
    pub test: Option<TestRun>,
    #[serde(default)]
    pub orchestrator_usd: f64,
    #[serde(default)]
    pub reviewer_usd: f64,
    #[serde(default)]
    pub estimated: bool,
    #[serde(default)]
    pub forked_from: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl TaskState {
    pub fn total_usd(&self) -> f64 {
        self.orchestrator_usd
            + self.reviewer_usd
            + self.subtasks.iter().map(|s| s.cost_usd).sum::<f64>()
    }

    pub fn estimated(&self) -> bool {
        self.estimated || self.subtasks.iter().any(|s| s.estimated)
    }

    /// Finished share in percent (merged or dropped sub-tasks).
    pub fn percent(&self) -> u32 {
        if self.status == TaskStatus::Done {
            return 100;
        }
        let n = self.subtasks.len();
        if n == 0 {
            return 0;
        }
        let done = self
            .subtasks
            .iter()
            .filter(|s| matches!(s.status, SubStatus::Merged | SubStatus::Dropped))
            .count();
        u32::try_from(done * 100 / n).unwrap_or(100)
    }
}

fn integration_branch(id: &str) -> String {
    format!("kor/task/{id}/merged")
}

/// `backend`, `Front End!` → `front-end`; at most 32 characters, never empty.
pub fn slug(role: &str) -> String {
    let mut s = String::new();
    for c in role.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
        if s.len() >= 32 {
            break;
        }
    }
    let s = s.trim_end_matches('-').to_owned();
    if s.is_empty() { "agent".into() } else { s }
}

// ---------------------------------------------------------------------------------------------
// Store: <repo>/.kor/tasks/checkpoints.sqlite
// ---------------------------------------------------------------------------------------------

const DB_FILE: &str = "checkpoints.sqlite";

pub fn tasks_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("tasks")
}

fn worktrees_dir(root: &Path, id: &str) -> PathBuf {
    root.join(".kor").join("worktrees").join(id)
}

/// Task ids are the numbers `kor task list` prints.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Open (create) the checkpoint database and import checkpoints older versions left as JSON.
fn db(root: &Path, create: bool) -> Result<Option<rusqlite::Connection>, String> {
    let dir = tasks_dir(root);
    let file = dir.join(DB_FILE);
    let legacy: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_stem().and_then(|s| s.to_str()).is_some_and(valid_id)
        })
        .collect();
    if !create && !file.exists() && legacy.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let c = rusqlite::Connection::open(&file).map_err(|e| e.to_string())?;
    c.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    c.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, updated_ms INTEGER NOT NULL, state TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;
    // `<id>.json` checkpoints of older versions (the `TaskCreate` tool's own files do not parse
    // as a task state and stay untouched).
    for p in legacy {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(st) = serde_json::from_str::<TaskState>(&text) else {
            continue;
        };
        c.execute(
            "INSERT OR IGNORE INTO tasks (id, updated_ms, state) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                st.id,
                i64::try_from(st.updated_ms).unwrap_or(i64::MAX),
                text
            ],
        )
        .map_err(|e| e.to_string())?;
        let _ = std::fs::rename(&p, p.with_extension("json.imported"));
    }
    Ok(Some(c))
}

/// Write the checkpoint (one transaction: a crash never leaves half a checkpoint).
///
/// # Errors
/// If the database cannot be written.
pub fn save(root: &Path, state: &mut TaskState) -> Result<(), String> {
    state.updated_ms = now_ms();
    let text = serde_json::to_string(state).map_err(|e| e.to_string())?;
    let c = db(root, true)?.ok_or("checkpoint database unavailable")?;
    c.execute(
        "INSERT INTO tasks (id, updated_ms, state) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET updated_ms = excluded.updated_ms, state = excluded.state",
        rusqlite::params![
            state.id,
            i64::try_from(state.updated_ms).unwrap_or(i64::MAX),
            text
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// # Errors
/// Unknown id or an unreadable checkpoint.
pub fn load(root: &Path, id: &str) -> Result<TaskState, String> {
    let id = id.trim_start_matches('#');
    if !valid_id(id) {
        return Err(format!("invalid task id `{id}`"));
    }
    let missing = || format!("task #{id} not found");
    let c = db(root, false)?.ok_or_else(missing)?;
    let text: String = c
        .query_row("SELECT state FROM tasks WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|_| missing())?;
    serde_json::from_str(&text).map_err(|e| format!("task #{id}: {e}"))
}

/// Every task of this repository, most recently updated first.
pub fn list(root: &Path) -> Vec<TaskState> {
    let Ok(Some(c)) = db(root, false) else {
        return Vec::new();
    };
    let Ok(mut q) = c.prepare("SELECT state FROM tasks ORDER BY updated_ms DESC") else {
        return Vec::new();
    };
    q.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| {
            rows.flatten()
                .filter_map(|t| serde_json::from_str(&t).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn next_id(root: &Path) -> String {
    let max = list(root)
        .iter()
        .filter_map(|t| t.id.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    (max + 1).to_string()
}

/// A new task at `HEAD` of the repository containing `dir`, saved before any model call.
///
/// # Errors
/// Not a git repository, no commit yet, or the checkpoint cannot be written.
pub fn create(dir: &Path, goal: &str, settings: Settings) -> Result<(PathBuf, TaskState), String> {
    let goal = goal.trim();
    if goal.is_empty() {
        return Err("the task goal is empty".into());
    }
    let root = git::repo_root(dir)?;
    let base = git::head(&root)?;
    git::exclude_kor_dirs(&root)?;
    let id = next_id(&root);
    let now = now_ms();
    let mut state = TaskState {
        version: VERSION,
        branch: integration_branch(&id),
        id,
        goal: goal.to_owned(),
        summary: String::new(),
        status: TaskStatus::Planning,
        created_ms: now,
        updated_ms: now,
        base,
        settings,
        subtasks: Vec::new(),
        conflicts: Vec::new(),
        test: None,
        orchestrator_usd: 0.0,
        reviewer_usd: 0.0,
        estimated: false,
        forked_from: None,
        error: None,
    };
    save(&root, &mut state)?;
    Ok((root, state))
}

/// A copy of task `id` under a new id from the same point (`kor task fork`): its branches are
/// copied, worktrees are made again on `resume`.
///
/// # Errors
/// Unknown task, or git refuses a branch copy.
pub fn fork(root: &Path, id: &str) -> Result<TaskState, String> {
    let old = load(root, id)?;
    let new_id = next_id(root);
    let mut st = old.clone();
    st.id.clone_from(&new_id);
    st.forked_from = Some(old.id.clone());
    st.branch = integration_branch(&new_id);
    st.created_ms = now_ms();
    if git::branch_exists(root, &old.branch) {
        git::git(root, &["branch", &st.branch, &old.branch])?;
    }
    for s in &mut st.subtasks {
        let (branch, worktree) = sub_names(&new_id, s.index, &s.role);
        if git::branch_exists(root, &s.branch) {
            git::git(root, &["branch", &branch, &s.branch])?;
        }
        s.branch = branch;
        s.worktree = worktree;
        if s.status == SubStatus::Running {
            s.status = SubStatus::Pending;
        }
    }
    if matches!(
        st.status,
        TaskStatus::Running | TaskStatus::Planning | TaskStatus::Merging
    ) {
        st.status = TaskStatus::Failed;
    }
    save(root, &mut st)?;
    Ok(st)
}

/// Stop for good: worktrees and sub-agent branches are removed; the task branch stays.
///
/// # Errors
/// Unknown task or an unwritable checkpoint.
pub fn cancel(root: &Path, id: &str) -> Result<TaskState, String> {
    let mut st = load(root, id)?;
    for s in &st.subtasks {
        git::remove_worktree(root, &root.join(&s.worktree), Some(&s.branch));
    }
    git::remove_worktree(root, &worktrees_dir(root, &st.id).join("merged"), None);
    let _ = std::fs::remove_dir_all(worktrees_dir(root, &st.id));
    st.status = TaskStatus::Cancelled;
    save(root, &mut st)?;
    Ok(st)
}

fn sub_names(id: &str, index: usize, role: &str) -> (String, String) {
    let name = format!("{}-{}", index + 1, slug(role));
    (
        format!("kor/task/{id}/{name}"),
        format!(".kor/worktrees/{id}/{name}"),
    )
}

// ---------------------------------------------------------------------------------------------
// Models and sub-agents
// ---------------------------------------------------------------------------------------------

/// A model the orchestrator or reviewer talks to, with the ledger it reports to.
#[derive(Clone)]
pub struct Model {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub cost: Option<Arc<cost::Control>>,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("provider", &self.provider.id())
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// A sub-agent ready to run.
#[allow(missing_debug_implementations)] // `Agent` holds providers and tools
pub struct Spawned {
    pub agent: Agent,
    /// Its router, ledger and budget (also in `agent.cost`, replaced by the task's cap).
    pub cost: Arc<cost::Control>,
    pub session: Option<PathBuf>,
}

/// Builds sub-agents and command contexts. [`RuntimeSpawner`] uses the user's configuration;
/// tests use scripted providers.
pub trait Spawner: Send + Sync {
    /// A sub-agent rooted at `worktree`; `model` is pinned when given.
    ///
    /// # Errors
    /// The model or provider cannot be used.
    fn spawn(&self, worktree: &Path, role: &str, model: Option<&str>) -> Result<Spawned, String>;
    /// Where the test command runs (sandboxed like agent commands).
    ///
    /// # Errors
    /// `dir` is not usable.
    fn command_ctx(&self, dir: &Path) -> Result<ToolCtx, String>;
}

/// Answers every permission question with "no": sub-agents run unattended, so whatever the
/// permission mode and rules do not allow on their own is refused.
struct Unattended;

#[async_trait::async_trait]
impl Approver for Unattended {
    async fn approve(&self, _: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

/// The sub-agent's own cost control: pinned model (no routing), and a USD cap.
struct SubCost {
    inner: Arc<cost::Control>,
    pinned: bool,
    cap_usd: f64,
    l: Localizer,
}

impl CostControl for SubCost {
    fn route(&self, task: &str) -> Option<RoutePlan> {
        if self.pinned {
            None
        } else {
            self.inner.route(task)
        }
    }
    fn record(&self, model: &str, usage: &Usage, cost_usd: Option<f64>) -> Option<(f64, bool)> {
        self.inner.record(model, usage, cost_usd)
    }
    fn touched(&self, path: &Path) {
        self.inner.touched(path);
    }
    fn task_done(&self, outcome: Outcome) {
        self.inner.task_done(outcome);
    }
    fn spend(&self) -> Spend {
        let (spent, est) = self.inner.session_usd();
        if self.cap_usd > 0.0 && spent >= self.cap_usd {
            return Spend::Over {
                action: OnLimit::Pause,
                message: self.l.t_args(
                    "task-agent-budget",
                    &args!("money" => format!(
                        "{} / {}",
                        cost::fmt_usd(spent, est),
                        cost::fmt_usd(self.cap_usd, false)
                    )),
                ),
                fields: json!({"scope": "task-agent", "spent_usd": spent, "limit_usd": self.cap_usd}),
            };
        }
        self.inner.spend()
    }
    fn downgrade(&self) -> Option<Route> {
        self.inner.downgrade()
    }
    fn allow_over(&self) {
        self.inner.allow_over();
    }
}

// ---------------------------------------------------------------------------------------------
// Structured output (§15.13)
// ---------------------------------------------------------------------------------------------

/// The first `{ … }` block of a reply (models wrap JSON in prose or fences).
fn json_in(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start)
        .then(|| serde_json::from_str(&text[start..=end]).ok())
        .flatten()
}

/// One structured answer: a forced tool call where the provider has native tools, JSON in the
/// text otherwise; coerced and validated against `schema`, with one repair round (§15.12).
/// Returns the value and its cost (USD, estimated).
async fn structured(
    m: &Model,
    system: &str,
    prompt: &str,
    tool: &str,
    schema: &str,
    cancel: &CancellationToken,
) -> Result<(Value, f64, bool), String> {
    let schema: Value = serde_json::from_str(schema).map_err(|e| e.to_string())?;
    let native = m.provider.manifest().tool_calling;
    let ask = format!(
        "{prompt}\n\nAnswer {} with exactly this JSON Schema:\n{schema}",
        if native {
            format!("by calling the `{tool}` tool once")
        } else {
            "with only one JSON object (no other text)".to_owned()
        }
    );
    let mut messages = vec![Message::text(Role::User, ask)];
    let (mut usd, mut est) = (0.0, false);
    let mut last_err = String::new();
    for _ in 0..2 {
        let req = ChatRequest {
            model: m.model.clone(),
            system: Some(system.to_owned()),
            messages: messages.clone(),
            tools: if native {
                vec![ToolSpec {
                    name: tool.to_owned(),
                    description: format!("Submit the {tool} result."),
                    input_schema: schema.clone(),
                }]
            } else {
                Vec::new()
            },
            max_tokens: Some(8_000),
            ..ChatRequest::default()
        };
        let mut stream = m
            .provider
            .complete(req, cancel.clone())
            .await
            .map_err(|e| e.message)?;
        let (mut text, mut call) = (String::new(), String::new());
        while let Some(ev) = stream.next().await {
            match ev.map_err(|e| e.message)? {
                StreamEvent::TextDelta { text: t } => text.push_str(&t),
                StreamEvent::ToolCallDelta { json, .. } => call.push_str(&json),
                StreamEvent::Usage { usage } => {
                    if let Some((c, e)) = m.cost.as_ref().and_then(|c| {
                        c.record(&format!("{}/{}", m.provider.id(), m.model), &usage, None)
                    }) {
                        usd += c;
                        est |= e;
                    }
                }
                _ => {}
            }
        }
        let parsed = if call.trim().is_empty() {
            json_in(&text)
        } else {
            serde_json::from_str(&call).ok().or_else(|| json_in(&call))
        };
        let reply = if call.is_empty() { text } else { call };
        match parsed {
            Some(mut v) => {
                kor_agent::schema::coerce(&mut v, &schema);
                match kor_agent::schema::validate(&v, &schema) {
                    Ok(()) => return Ok((v, usd, est)),
                    Err(e) => last_err = e,
                }
            }
            None => last_err = "no JSON object in the reply".into(),
        }
        messages.push(Message::text(Role::Assistant, reply));
        messages.push(Message::text(
            Role::User,
            format!(
                "That did not match the schema: {last_err}. Reply again with only the corrected JSON."
            ),
        ));
    }
    Err(format!("{tool}: {last_err}"))
}

const PLANNER: &str = "You are the orchestrator of a team of coding agents working in one git \
repository. Split the goal into independent sub-tasks, one per agent, each with a short role \
(architecture, backend, frontend, test, docs, or whatever the goal needs), precise instructions, \
and the files or directories it owns. Ownership must not overlap unless unavoidable; if two \
agents must touch one file, say which part each one changes.";

const REVIEWER: &str = "You review one coding agent's change before it is merged. Approve it only \
if it does what its instructions ask, stays in its area, and its tests (if any) pass. Be brief.";

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

/// Called with a snapshot after every change of the checkpoint.
pub type Progress = Arc<dyn Fn(&TaskState) + Send + Sync>;

/// What `resolve` does with a conflicting sub-task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Ask the orchestrator for new instructions and run that part again on top of the task branch.
    Replan,
    /// Leave that part out.
    Drop,
}

#[allow(missing_debug_implementations)] // trait objects (spawner, progress callback)
pub struct Engine {
    /// Repository root.
    pub root: PathBuf,
    pub lang: Lang,
    pub orchestrator: Model,
    pub reviewer: Model,
    pub spawner: Arc<dyn Spawner>,
    pub progress: Progress,
}

/// What one sub-agent run returns to the loop that owns the checkpoint.
struct SubResult {
    sub: SubTask,
    reviewer_usd: f64,
    reviewer_estimated: bool,
}

impl Engine {
    fn save(&self, st: &mut TaskState) -> Result<(), String> {
        save(&self.root, st)?;
        (self.progress)(st);
        Ok(())
    }

    fn tag(&self, st: &TaskState) {
        if let Some(c) = &self.orchestrator.cost {
            c.set_role(format!("task:{}/orchestrator", st.id));
        }
        if let Some(c) = &self.reviewer.cost {
            c.set_role(format!("task:{}/reviewer", st.id));
        }
    }

    /// Plan (if not planned yet), run every runnable sub-task, review, merge, test.
    /// Safe to call again after a crash or cancel: finished parts are kept.
    ///
    /// # Errors
    /// Planning failed or the checkpoint / git state cannot be written. Sub-task failures are
    /// recorded in the state instead.
    pub async fn run(&self, st: &mut TaskState, cancel: &CancellationToken) -> Result<(), String> {
        self.tag(st);
        st.error = None;
        if st.subtasks.is_empty() {
            st.status = TaskStatus::Planning;
            self.save(st)?;
            if let Err(e) = self.plan(st, cancel).await {
                st.status = TaskStatus::Failed;
                st.error = Some(e.clone());
                self.save(st)?;
                return Err(e);
            }
        }
        self.run_subtasks(st, cancel).await?;
        if cancel.is_cancelled() {
            st.status = TaskStatus::Cancelled;
            return self.save(st);
        }
        self.merge_all(st, cancel).await
    }

    async fn plan(&self, st: &mut TaskState, cancel: &CancellationToken) -> Result<(), String> {
        let prompt = format!(
            "Goal: {}\n\nUse at most {} sub-tasks (one agent each).\n\nRepository files:\n{}",
            st.goal,
            st.settings.agents,
            file_list(&self.root)
        );
        let (v, usd, est) = structured(
            &self.orchestrator,
            PLANNER,
            &prompt,
            "submit_plan",
            kor_core::schemas::TASK_PLAN_V1,
            cancel,
        )
        .await?;
        st.orchestrator_usd += usd;
        st.estimated |= est;
        v["summary"]
            .as_str()
            .unwrap_or_default()
            .clone_into(&mut st.summary);
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        };
        for (index, s) in v["subtasks"]
            .as_array()
            .into_iter()
            .flatten()
            .take(st.settings.agents)
            .enumerate()
        {
            let role = s["role"].as_str().unwrap_or("agent").trim().to_owned();
            let (branch, worktree) = sub_names(&st.id, index, &role);
            let model = st
                .settings
                .models
                .get(&role)
                .or_else(|| st.settings.models.get(&slug(&role)))
                .filter(|m| !m.trim().is_empty())
                .cloned();
            st.subtasks.push(SubTask {
                index,
                goal: s["goal"].as_str().unwrap_or_default().to_owned(),
                files: strings(&s["files"]),
                role,
                status: SubStatus::Pending,
                branch,
                worktree,
                model,
                models_used: Vec::new(),
                base: None,
                commit: None,
                changed: Vec::new(),
                outside: Vec::new(),
                cost_usd: 0.0,
                estimated: false,
                test: None,
                review: None,
                error: None,
                session: None,
            });
        }
        Ok(())
    }

    async fn run_subtasks(
        &self,
        st: &mut TaskState,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let todo: Vec<usize> = st
            .subtasks
            .iter()
            .filter(|s| s.status.runnable())
            .map(|s| s.index)
            .collect();
        if todo.is_empty() {
            return Ok(());
        }
        let cap = st.settings.agent_budget(st.subtasks.len());
        let over_budget = st.settings.budget_usd > 0.0 && st.total_usd() >= st.settings.budget_usd;
        st.status = TaskStatus::Running;
        for i in &todo {
            let s = &mut st.subtasks[*i];
            if over_budget {
                s.status = SubStatus::Failed;
                s.error = Some("task budget reached".into());
            } else {
                s.status = SubStatus::Running;
                s.error = None;
            }
        }
        self.save(st)?;
        if over_budget {
            return Ok(());
        }
        let snapshot = st.clone();
        let jobs: Vec<SubTask> = todo.iter().map(|i| st.subtasks[*i].clone()).collect();
        let mut results = futures::stream::iter(jobs)
            .map(|sub| self.run_one(&snapshot, sub, cap, cancel))
            .buffer_unordered(st.settings.max_parallel.max(1));
        while let Some(r) = results.next().await {
            st.reviewer_usd += r.reviewer_usd;
            st.estimated |= r.reviewer_estimated;
            let i = r.sub.index;
            st.subtasks[i] = r.sub;
            self.save(st)?;
        }
        Ok(())
    }

    /// One sub-agent: fresh worktree, agent loop, commit, tests, review.
    async fn run_one(
        &self,
        st: &TaskState,
        mut sub: SubTask,
        cap: f64,
        cancel: &CancellationToken,
    ) -> SubResult {
        let mut out = SubResult {
            sub: sub.clone(),
            reviewer_usd: 0.0,
            reviewer_estimated: false,
        };
        let r = self.work(st, &mut sub, cap, cancel, &mut out).await;
        if let Err(e) = r {
            sub.status = if cancel.is_cancelled() {
                SubStatus::Cancelled
            } else {
                SubStatus::Failed
            };
            sub.error = Some(e);
        }
        out.sub = sub;
        out
    }

    async fn work(
        &self,
        st: &TaskState,
        sub: &mut SubTask,
        cap: f64,
        cancel: &CancellationToken,
        out: &mut SubResult,
    ) -> Result<(), String> {
        let wt = self.root.join(&sub.worktree);
        let base = sub.base.clone().unwrap_or_else(|| st.base.clone());
        git::fresh_worktree(&self.root, &wt, &sub.branch, &base)?;
        let spawned = self.spawner.spawn(&wt, &sub.role, sub.model.as_deref())?;
        let Spawned {
            mut agent,
            cost,
            session,
        } = spawned;
        cost.set_role(format!("task:{}/{}", st.id, slug(&sub.role)));
        agent.cost = Some(Arc::new(SubCost {
            inner: cost.clone(),
            pinned: sub.model.is_some(),
            cap_usd: cap,
            l: Localizer::new(self.lang),
        }));
        agent.max_turns = st.settings.max_turns;
        sub.session = session.map(|p| p.to_string_lossy().into_owned());
        // Only a rejected attempt leaves a review on a sub-task that runs again.
        let previous = sub.review.take();
        let prompt = sub_prompt(st, sub, previous.as_deref());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // Events are not needed here; drain them so the channel never grows.
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = agent.send(prompt, &tx, &Unattended, cancel.clone()).await;
        drop(tx);
        let _ = drain.await;
        let (usd, est) = cost.session_usd();
        sub.cost_usd += usd;
        sub.estimated |= est;
        sub.models_used = cost.session_rows().into_iter().map(|r| r.model).collect();
        match result {
            Ok(StopReason::Done(_)) => {}
            Ok(StopReason::Cancelled) => return Err("cancelled".into()),
            Ok(StopReason::Budget) => return Err("budget reached".into()),
            Ok(StopReason::MaxTurns) => return Err("turn limit reached".into()),
            Ok(StopReason::Blocked(why)) => return Err(why),
            Err(e) => return Err(e.message),
        }
        let msg = format!(
            "kor task #{}: {} ({})",
            st.id,
            sub.role,
            first_line(&sub.goal)
        );
        let Some(commit) = git::commit_all(&wt, &msg)? else {
            return Err("the agent changed no files".into());
        };
        sub.commit = Some(commit.clone());
        let (changed, diff) = git::diff(&wt, &base, &commit)?;
        sub.outside = changed
            .iter()
            .filter(|f| !owned(&sub.files, f))
            .cloned()
            .collect();
        sub.changed = changed;
        sub.test = self.test(&wt, &st.settings.test_command).await;
        let verdict = structured(
            &self.reviewer,
            REVIEWER,
            &review_prompt(sub, &diff),
            "submit_review",
            kor_core::schemas::TASK_REVIEW_V1,
            cancel,
        )
        .await?;
        out.reviewer_usd = verdict.1;
        out.reviewer_estimated = verdict.2;
        sub.review = verdict.0["notes"].as_str().map(str::to_owned);
        let tests_ok = sub.test.as_ref().is_none_or(|t| t.ok);
        sub.status = if verdict.0["approved"].as_bool() == Some(true) && tests_ok {
            SubStatus::Approved
        } else {
            SubStatus::Rejected
        };
        Ok(())
    }

    async fn test(&self, dir: &Path, command: &str) -> Option<TestRun> {
        if command.is_empty() {
            return None;
        }
        let ctx = match self.spawner.command_ctx(dir) {
            Ok(c) => c,
            Err(e) => {
                return Some(TestRun {
                    ok: false,
                    output: e,
                });
            }
        };
        let Some(bash) = kor_tools::default_tools()
            .into_iter()
            .find(|t| t.name() == "Bash")
        else {
            return Some(TestRun {
                ok: false,
                output: "no shell tool".into(),
            });
        };
        let o = bash.call(json!({ "command": command }), &ctx).await;
        let mut output = o.content;
        if output.len() > MAX_TEST_OUTPUT {
            let mut cut = output.len() - MAX_TEST_OUTPUT;
            while !output.is_char_boundary(cut) {
                cut += 1;
            }
            output = format!("[…]{}", &output[cut..]);
        }
        Some(TestRun {
            ok: !o.is_error,
            output,
        })
    }

    /// Merge approved sub-tasks in plan order, then test the task branch.
    async fn merge_all(
        &self,
        st: &mut TaskState,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let merged_wt = worktrees_dir(&self.root, &st.id).join("merged");
        git::integration_worktree(&self.root, &merged_wt, &st.branch, &st.base)?;
        st.status = TaskStatus::Merging;
        self.save(st)?;
        let mut merged_any = false;
        for i in 0..st.subtasks.len() {
            if st.subtasks[i].status != SubStatus::Approved {
                continue;
            }
            let (role, branch) = (st.subtasks[i].role.clone(), st.subtasks[i].branch.clone());
            let msg = format!("kor task #{}: merge {role}", st.id);
            match git::merge(&merged_wt, &branch, &msg) {
                Ok(git::Merge::Merged(_)) => {
                    merged_any = true;
                    let s = &mut st.subtasks[i];
                    s.status = SubStatus::Merged;
                    git::remove_worktree(&self.root, &self.root.join(&s.worktree), Some(&branch));
                }
                Ok(git::Merge::Conflict { files, diff }) => {
                    st.subtasks[i].status = SubStatus::Conflict;
                    st.conflicts.retain(|c| c.index != i);
                    st.conflicts.push(Conflict {
                        index: i,
                        role,
                        files,
                        diff,
                    });
                }
                Err(e) => {
                    let s = &mut st.subtasks[i];
                    s.status = SubStatus::Failed;
                    s.error = Some(e);
                }
            }
            self.save(st)?;
        }
        if merged_any || (st.test.is_none() && st.conflicts.is_empty()) {
            st.test = self.test(&merged_wt, &st.settings.test_command).await;
        }
        let all_in = st
            .subtasks
            .iter()
            .all(|s| matches!(s.status, SubStatus::Merged | SubStatus::Dropped));
        st.status = if cancel.is_cancelled() {
            TaskStatus::Cancelled
        } else if !st.conflicts.is_empty() {
            TaskStatus::Conflict
        } else if all_in && st.test.as_ref().is_none_or(|t| t.ok) {
            TaskStatus::Done
        } else {
            TaskStatus::Failed
        };
        if st.status == TaskStatus::Done {
            git::remove_worktree(&self.root, &merged_wt, None);
            let _ = std::fs::remove_dir_all(worktrees_dir(&self.root, &st.id));
        }
        self.save(st)
    }

    /// Settle the conflict of sub-task `index`, then carry on with the task.
    ///
    /// # Errors
    /// No conflict for that sub-task, re-planning failed, or the checkpoint cannot be written.
    pub async fn resolve(
        &self,
        st: &mut TaskState,
        index: usize,
        how: Resolution,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.tag(st);
        let Some(pos) = st.conflicts.iter().position(|c| c.index == index) else {
            return Err(format!("sub-task {} has no conflict", index + 1));
        };
        let conflict = st.conflicts[pos].clone();
        match how {
            Resolution::Drop => {
                let s = &mut st.subtasks[index];
                s.status = SubStatus::Dropped;
                git::remove_worktree(&self.root, &self.root.join(&s.worktree), Some(&s.branch));
            }
            Resolution::Replan => {
                let merged_wt = worktrees_dir(&self.root, &st.id).join("merged");
                git::integration_worktree(&self.root, &merged_wt, &st.branch, &st.base)?;
                let tip = git::head(&merged_wt)?;
                let s = &st.subtasks[index];
                let files: Vec<&str> = conflict.files.iter().map(|f| f.path.as_str()).collect();
                let prompt = format!(
                    "Overall goal: {}\n\nSub-task `{}` was:\n{}\nOwned files: {}\n\nIts merge into \
                     the task branch conflicted in: {}\n\nConflict as git shows it:\n{}\n\nThe task \
                     branch already holds the other agents' work. Write new instructions for this \
                     sub-task so that it applies cleanly on top of that work (it will start from \
                     the task branch), and list the files it owns.",
                    st.goal,
                    s.role,
                    s.goal,
                    s.files.join(", "),
                    files.join(", "),
                    conflict.diff
                );
                let (v, usd, est) = structured(
                    &self.orchestrator,
                    PLANNER,
                    &prompt,
                    "submit_replan",
                    kor_core::schemas::TASK_REPLAN_V1,
                    cancel,
                )
                .await?;
                st.orchestrator_usd += usd;
                st.estimated |= est;
                let s = &mut st.subtasks[index];
                s.goal = v["goal"].as_str().unwrap_or(&s.goal).to_owned();
                s.files = v["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect();
                s.base = Some(tip);
                s.status = SubStatus::Pending;
                s.commit = None;
                s.review = None;
            }
        }
        st.conflicts.remove(pos);
        self.save(st)?;
        self.run(st, cancel).await
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(72)
        .collect()
}

/// `path` is one of `files` or inside one of them (a directory).
fn owned(files: &[String], path: &str) -> bool {
    files.iter().any(|f| {
        let f = f.trim().trim_start_matches("./").trim_end_matches('/');
        !f.is_empty() && (path == f || path.starts_with(&format!("{f}/")))
    })
}

fn sub_prompt(st: &TaskState, sub: &SubTask, previous_review: Option<&str>) -> String {
    let mut p = format!(
        "You are the `{}` agent of a team working on: {}\n\nYour task:\n{}\n\nFiles you own: {}\n\
         Change only these unless the task says otherwise; other agents work on the rest in \
         parallel. Do not commit, push or switch branches: Kor commits your work and a reviewer \
         checks it before merge.",
        sub.role,
        st.goal,
        sub.goal,
        if sub.files.is_empty() {
            "(not specified)".to_owned()
        } else {
            sub.files.join(", ")
        }
    );
    if !st.settings.test_command.is_empty() {
        let _ = write!(
            p,
            "\nThe reviewer runs `{}`; it must pass.",
            st.settings.test_command
        );
    }
    if let Some(r) = previous_review {
        let _ = write!(
            p,
            "\n\nA previous attempt was rejected by the reviewer:\n{r}"
        );
    }
    p
}

fn review_prompt(sub: &SubTask, diff: &str) -> String {
    let test = match &sub.test {
        Some(t) => format!(
            "Tests {}:\n{}",
            if t.ok { "passed" } else { "FAILED" },
            t.output
        ),
        None => "No test command is configured.".into(),
    };
    let outside = if sub.outside.is_empty() {
        String::new()
    } else {
        format!(
            "\nChanged outside its owned files: {}",
            sub.outside.join(", ")
        )
    };
    format!(
        "Role: {}\nInstructions:\n{}\nOwned files: {}{outside}\n\n{test}\n\nDiff:\n{diff}",
        sub.role,
        sub.goal,
        sub.files.join(", ")
    )
}

/// Tracked files for the planner (first 400).
fn file_list(root: &Path) -> String {
    let all = git::git(root, &["ls-files"]).unwrap_or_default();
    let mut lines: Vec<&str> = all.lines().take(400).collect();
    if all.lines().count() > lines.len() {
        lines.push("…");
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_roles() {
        assert_eq!(slug("Back End!"), "back-end");
        assert_eq!(slug("  "), "agent");
        assert_eq!(slug("ÇÖP docs"), "p-docs");
        assert!(slug(&"x".repeat(50)).len() <= 32);
    }

    #[test]
    fn ownership_covers_files_and_directories() {
        let files = vec!["src/api/".to_owned(), "README.md".to_owned()];
        assert!(owned(&files, "src/api/x.rs"));
        assert!(owned(&files, "README.md"));
        assert!(!owned(&files, "src/apix.rs"));
        assert!(!owned(&[], "a"));
    }

    #[test]
    fn json_is_found_in_prose() {
        assert_eq!(
            json_in("Sure:\n```json\n{\"a\": 1}\n```"),
            Some(json!({"a": 1}))
        );
        assert_eq!(json_in("no json"), None);
    }

    #[test]
    fn budget_is_shared_unless_set_per_agent() {
        let mut s = Settings {
            budget_usd: 10.0,
            ..Settings::default()
        };
        assert!((s.agent_budget(5) - 2.0).abs() < 1e-9);
        s.budget_per_agent_usd = 3.0;
        assert!((s.agent_budget(5) - 3.0).abs() < 1e-9);
        assert!(Settings::default().agent_budget(5).abs() < f64::EPSILON);
    }

    #[test]
    fn ids_are_numbers() {
        assert!(valid_id("12"));
        assert!(!valid_id("../x"));
        assert!(!valid_id(""));
    }
}
