//! Built-in tools. Every path a tool touches is confined to the project root;
//! every failure is returned to the model as an error result, never a panic.

pub mod devenv;
mod jail;
// `kor layout save|restore` on the real desktop (§27.7, ADR 0059).
pub mod layouts;
pub mod param_card;
pub mod pdf;
pub mod plans;
pub mod record;
// Rule compliance of the agent's edits (§28.3, ADR 0063).
pub mod rules_check;
mod secret_scan;
mod tools;
pub mod undo;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_core::{PermissionMode, ToolSpec};
use kor_sandbox::SandboxKind;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use jail::{resolve, without_verbatim};
pub use secret_scan::scan as scan_secrets;
pub use secret_scan::redact as redact_secrets;
pub use secret_scan::find as find_secrets;

/// Rule ids from the team-memory secret scan. Empty when the text is clean.
pub fn secret_labels(content: &str) -> Vec<&'static str> {
    secret_scan::scan(content)
}
pub use tools::chrome;
pub use tools::browser::{html_screenshot, html_to_pdf, lab_vitals};
pub use tools::ocr::image_text;
pub use tools::shell::Shell;
pub use tools::review::{Proposal, review_risk, write_reviewed};
pub use param_card::{ParamAnswer, ParamCard, ParamField};

/// What a tool does, for permission defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// An image for the model (e.g. a screenshot); attached to the next message.
    pub image: Option<kor_core::ImageSource>,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            image: None,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            image: None,
        }
    }
}

#[derive(Debug)]
pub struct ToolCtx {
    /// Canonical project root.
    pub root: PathBuf,
    pub cancel: CancellationToken,
    pub sandbox: SandboxKind,
    /// Allow network access for sandboxed commands.
    pub network: bool,
    /// Network allowlist, memory/CPU caps and Docker image for sandboxed commands (ADR 0033).
    pub sandbox_options: kor_sandbox::Options,
    /// Blocks the sandbox left in command output since the agent last took them (audit log).
    pub sandbox_denials: Mutex<Vec<kor_sandbox::Denial>>,
    /// Environment variables removed before spawning commands (API keys).
    pub scrub_env: Vec<String>,
    /// Extra environment for commands (for example `env` from settings).
    pub env: Vec<(String, String)>,
    /// Vault secrets limited to some programs (`[vault.scope]`): name, value, programs. Added
    /// only to commands that run one of the programs ([`scoped_env_for`]).
    pub scoped_env: Vec<(String, String, Vec<String>)>,
    /// Paths the agent may not touch (`.korignore`, `.cursorignore`, ...).
    pub ai_ignore: Option<Arc<ignore::gitignore::Gitignore>>,
    /// Files the model has read this session. Edit and Write require a fresh full read.
    pub read_files: Mutex<HashMap<PathBuf, ReadStamp>>,
    /// `TodoWrite` checklist for this session.
    pub todos: Mutex<Vec<Todo>>,
    /// Canonical directories outside the project the agent may also read and write.
    /// Auto-memory (`~/.claude/projects/<slug>/memory`) is the intended use.
    pub extra_roots: Vec<PathBuf>,
    /// Team memory directory. Writes here are refused when `secret_scan` matches.
    pub team_memory: Option<PathBuf>,
    /// Page text cache and the optional second-model call for `WebFetch`.
    pub fetch: FetchState,
    /// When false, `Bash` refuses `run_in_background`.
    pub background_tasks: bool,
    /// Background bash jobs for `TaskOutput` / `TaskStop`.
    pub jobs: Arc<Mutex<HashMap<String, Job>>>,
    /// Session permission mode. `EnterPlanMode` / `ExitPlanMode` write it.
    pub plan: Arc<Mutex<PermissionMode>>,
    /// Tool names hidden from the model until `ToolSearch` selects them.
    pub deferred: Mutex<std::collections::HashSet<String>>,
    pub revealed: Mutex<std::collections::HashSet<String>>,
    /// Searchable text (description) of deferred tools, for `ToolSearch` keyword matches.
    pub deferred_about: Mutex<std::collections::HashMap<String, String>>,
    /// Session working directory. `EnterWorktree` points this at the new worktree.
    pub cwd: Arc<Mutex<PathBuf>>,
    /// Undo journal of the session (`kor undo`); `None` when the session is not saved.
    pub undo: Arc<Mutex<Option<undo::Journal>>>,
    /// Session recording (§15.9, `[record] enabled`); `None` = not recorded.
    pub record: Option<Arc<record::Recorder>>,
    /// Project environment commands run in and its pinned versions (§15.7).
    pub dev_env: Option<Arc<devenv::Session>>,
    /// Plan mode and the "stop and research" lock of this session (§28.2).
    pub plan_gate: Arc<plans::Gate>,
    /// Loaded rules checked against every proposed edit (§28.3); `None` = off.
    pub rule_checks: Option<Arc<rules_check::RuleSet>>,
    /// The editor driving the session over ACP (§28.8, ADR 0057); `None` = files and commands
    /// are local only.
    pub editor: Option<Arc<dyn EditorBridge>>,
}

/// File and terminal access the editor offers an ACP session (§28.8, ADR 0057): reads see its
/// unsaved buffers, writes go through it (its undo works), and — only when Kor's sandbox does
/// not confine commands — commands run in its terminal so the user watches them live.
#[async_trait]
pub trait EditorBridge: Send + Sync + std::fmt::Debug {
    fn can_read(&self) -> bool;
    fn can_write(&self) -> bool;
    fn can_terminal(&self) -> bool;
    async fn read_text(&self, path: &std::path::Path) -> Result<String, String>;
    async fn write_text(&self, path: &std::path::Path, content: &str) -> Result<(), String>;
    /// Run `program args` in `cwd`: combined output and the exit code (`None` = killed).
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &[(String, String)],
        timeout: std::time::Duration,
        cancel: &CancellationToken,
    ) -> Result<(String, Option<i32>), String>;
}

/// A background command.
#[derive(Debug)]
pub struct Job {
    pub output: String,
    pub cancel: CancellationToken,
    pub done: bool,
}

/// Whether the model may see this tool. Deferred names stay hidden until revealed.
pub fn tool_visible(ctx: &ToolCtx, name: &str) -> bool {
    !lock(&ctx.deferred).contains(name) || lock(&ctx.revealed).contains(name)
}

/// Make a deferred tool visible from now on (no-op for other tools).
pub fn reveal(ctx: &ToolCtx, name: &str) {
    if lock(&ctx.deferred).contains(name) {
        lock(&ctx.revealed).insert(name.to_owned());
    }
}

/// What `Read` last returned for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadStamp {
    pub mtime_ms: u128,
    /// `offset` was not 1, or the read stopped before the end of the file.
    pub partial: bool,
    /// Set only by `Read`. `Write` and `Edit` update the content but do not count as a read.
    pub from_read: bool,
    pub content: String,
    pub offset: usize,
    /// `None` means the default line cap.
    pub limit: Option<usize>,
}

/// Second model call for `WebFetch`. The page and the user's prompt are already
/// wrapped in the secondary instructions.
#[async_trait]
pub trait PageSummarizer: Send + Sync {
    async fn summarize(&self, instructions: &str) -> Result<String, String>;
}

/// `WebFetch` cache (15 minutes) and optional summarizer. Debug hides the trait object.
#[derive(Default)]
pub struct FetchState {
    pub cache: Mutex<HashMap<String, (String, std::time::Instant)>>,
    pub summarizer: Option<Arc<dyn PageSummarizer>>,
}

impl std::fmt::Debug for FetchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchState")
            .field("cached", &lock(&self.cache).len())
            .field("summarizer", &self.summarizer.is_some())
            .finish()
    }
}

/// One `TodoWrite` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub content: String,
    pub status: String,
    pub active_form: String,
}

/// Ignore files that hide paths from the agent, in addition to `.gitignore` for search.
pub const AI_IGNORE_FILES: &[&str] = &[
    ".korignore",
    ".cursorignore",
    ".cursorindexingignore",
    ".aiderignore",
];

/// Programs a shell command line starts: the first word of every `|`, `;`, `&`, `(` segment,
/// without leading `NAME=value` assignments, directory or `.exe`/`.cmd`/`.bat`, lowercased.
fn command_programs(command: &str) -> Vec<String> {
    command
        .split(['|', ';', '&', '\n', '(', ')', '`'])
        .filter_map(|seg| seg.split_whitespace().find(|w| !w.contains('=')))
        .map(|w| {
            let w = w.trim_matches(['"', '\'']);
            let base = w.rsplit(['/', '\\']).next().unwrap_or(w).to_ascii_lowercase();
            [".exe", ".cmd", ".bat"]
                .iter()
                .find_map(|x| base.strip_suffix(x).map(str::to_owned))
                .unwrap_or(base)
        })
        .collect()
}

/// The `[vault.scope]` secrets `command` may receive: those whose program list names one of the
/// programs the command line starts. shortcut: a word-level look at the command line, not a
/// shell parser; a secret scoped to `psql` also reaches `echo $X; psql` (its output is still
/// masked before any model sees it).
pub fn scoped_env_for<'a>(
    command: &str,
    scoped: &'a [(String, String, Vec<String>)],
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let programs = command_programs(command);
    scoped
        .iter()
        .filter(move |(_, _, allow)| {
            allow
                .iter()
                .any(|a| programs.iter().any(|p| p.eq_ignore_ascii_case(a.trim())))
        })
        .map(|(k, v, _)| (k.as_str(), v.as_str()))
}

impl ToolCtx {
    /// # Errors
    /// If `root` does not exist or cannot be canonicalized.
    pub fn new(root: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self {
            root: jail::without_verbatim(&root.canonicalize()?),
            cancel: CancellationToken::new(),
            sandbox: SandboxKind::None,
            network: true,
            sandbox_options: kor_sandbox::Options::default(),
            sandbox_denials: Mutex::new(Vec::new()),
            scrub_env: Vec::new(),
            env: Vec::new(),
            scoped_env: Vec::new(),
            ai_ignore: None,
            read_files: Mutex::new(HashMap::new()),
            todos: Mutex::new(Vec::new()),
            extra_roots: Vec::new(),
            team_memory: None,
            fetch: FetchState::default(),
            background_tasks: false,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            plan: Arc::new(Mutex::new(PermissionMode::Default)),
            deferred: Mutex::new(std::collections::HashSet::new()),
            revealed: Mutex::new(std::collections::HashSet::new()),
            deferred_about: Mutex::new(std::collections::HashMap::new()),
            cwd: Arc::new(Mutex::new(jail::without_verbatim(&root.canonicalize()?))),
            undo: Arc::new(Mutex::new(None)),
            record: None,
            dev_env: None,
            plan_gate: Arc::default(),
            rule_checks: None,
            editor: None,
        })
    }

    /// How commands of this session are confined (`sandbox` says by what).
    pub fn sandbox_policy(&self) -> kor_sandbox::Policy {
        kor_sandbox::Policy {
            root: self.root.clone(),
            network: self.network,
            options: self.sandbox_options.clone(),
        }
    }

    /// Find sandbox blocks in a command's `output`, say so at the end of `text` (the tool result)
    /// and keep them for the agent's audit log (ADR 0033).
    pub fn note_sandbox_blocks(&self, policy: &kor_sandbox::Policy, output: &str, text: &mut String) {
        let found = kor_sandbox::denials(self.sandbox, policy, output);
        if found.is_empty() {
            return;
        }
        for d in &found {
            text.push_str("\n[sandbox] ");
            text.push_str(&d.describe());
        }
        text.push_str(
            "\n[sandbox] The OS sandbox blocked this on purpose. Do not try to get around it; \
             ask the user if the command really needs it.",
        );
        lock(&self.sandbox_denials).extend(found);
    }

    /// Before a shell command: list the project for the undo journal (`None` when no journal).
    pub async fn shell_before(&self) -> Option<undo::Before> {
        if lock(&self.undo).is_none() {
            return None;
        }
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || undo::scan_before(&root)).await.ok()
    }

    /// After a shell command: record what it created or changed since `before`.
    pub async fn shell_after(&self, before: undo::Before) -> Option<undo::ShellChanges> {
        let journal = Arc::clone(&self.undo);
        tokio::task::spawn_blocking(move || lock(&journal).as_mut().and_then(|j| j.record_shell(&before).ok()))
            .await
            .ok()
            .flatten()
    }

    /// Record the first state of `paths` in the undo journal before a tool changes them.
    ///
    /// # Errors
    /// If an original cannot be saved; the change must then not run (it could not be undone).
    pub fn record_undo(&self, paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let cwd = lock(&self.cwd).clone();
        let mut j = lock(&self.undo);
        let Some(j) = j.as_mut() else { return Ok(()) };
        for p in paths {
            j.record(&cwd, p)
                .map_err(|e| format!("could not save `{p}` for undo: {e}"))?;
        }
        Ok(())
    }

    /// Load root-level AI ignore files. Errors in a file are returned, valid lines still apply.
    pub fn load_ai_ignore(&mut self) -> Vec<String> {
        let mut b = ignore::gitignore::GitignoreBuilder::new(&self.root);
        let mut errors = Vec::new();
        for name in AI_IGNORE_FILES {
            let p = self.root.join(name);
            if p.is_file()
                && let Some(e) = b.add(&p)
            {
                errors.push(e.to_string());
            }
        }
        match b.build() {
            Ok(g) if !g.is_empty() => self.ai_ignore = Some(Arc::new(g)),
            Ok(_) => {}
            Err(e) => errors.push(e.to_string()),
        }
        errors
    }

    /// Resolve a tool path inside the root and outside AI-ignored paths.
    ///
    /// # Errors
    /// If the path escapes the root or is excluded by an ignore file.
    pub fn resolve(&self, input: &str) -> Result<PathBuf, String> {
        let mut last = String::from("path is outside the project");
        for (i, root) in std::iter::once(&self.root)
            .chain(self.extra_roots.iter())
            .enumerate()
        {
            match jail::resolve(root, input) {
                Ok(p) => {
                    if i == 0
                        && let Some(g) = &self.ai_ignore
                        && g.matched_path_or_any_parents(&p, p.is_dir()).is_ignore()
                    {
                        return Err(format!(
                            "`{input}` is excluded by the project's ignore file"
                        ));
                    }
                    return Ok(p);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// A child context that shares roots, ignore rules, and the page summarizer.
    #[must_use]
    pub fn sibling(&self) -> Self {
        Self {
            root: self.root.clone(),
            cancel: self.cancel.clone(),
            sandbox: self.sandbox,
            network: self.network,
            sandbox_options: self.sandbox_options.clone(),
            sandbox_denials: Mutex::new(Vec::new()),
            scrub_env: self.scrub_env.clone(),
            env: self.env.clone(),
            scoped_env: self.scoped_env.clone(),
            ai_ignore: self.ai_ignore.clone(),
            read_files: Mutex::new(lock(&self.read_files).clone()),
            todos: Mutex::new(Vec::new()),
            extra_roots: self.extra_roots.clone(),
            team_memory: self.team_memory.clone(),
            fetch: FetchState {
                cache: Mutex::new(HashMap::new()),
                summarizer: self.fetch.summarizer.clone(),
            },
            background_tasks: self.background_tasks,
            jobs: Arc::clone(&self.jobs),
            plan: Arc::clone(&self.plan),
            deferred: Mutex::new(lock(&self.deferred).clone()),
            revealed: Mutex::new(lock(&self.revealed).clone()),
            deferred_about: Mutex::new(lock(&self.deferred_about).clone()),
            cwd: Arc::clone(&self.cwd),
            // Subagent edits go to the same journal: one `kor undo` reverts the whole session.
            undo: Arc::clone(&self.undo),
            // A sub-agent's events go to its own channel, not the parent's recording.
            record: None,
            dev_env: self.dev_env.clone(),
            plan_gate: Arc::clone(&self.plan_gate),
            rule_checks: self.rule_checks.clone(),
            editor: self.editor.clone(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    /// Name used in permission rules (`Read`, `Edit`, `Bash`, ...).
    fn rule_name(&self) -> &'static str;
    fn kind(&self) -> ToolKind;
    fn spec(&self) -> ToolSpec;
    /// The path or command permission rules match against.
    fn target(&self, input: &Value) -> Option<String>;
    /// One line describing the call, for permission prompts and logs.
    fn summary(&self, input: &Value) -> String {
        self.target(input).unwrap_or_default()
    }
    /// Paths this call may create, change or delete, recorded for `kor undo` before it runs.
    fn touches(&self, input: &Value) -> Vec<String> {
        if self.kind() == ToolKind::Edit {
            self.target(input).into_iter().collect()
        } else {
            Vec::new()
        }
    }
    /// The files this call would write, computed without writing, for per-file review (§12.3).
    /// `None` when the tool has no file preview or the call would fail anyway.
    async fn propose(&self, _input: &Value, _ctx: &ToolCtx) -> Option<Vec<Proposal>> {
        None
    }
    /// Choices to put to the user before the call (§20.2): a media tool asks for size, duration,
    /// voice... in one card instead of guessing. `None`: nothing to ask.
    async fn param_card(&self, _input: &Value, _ctx: &ToolCtx) -> Option<ParamCard> {
        None
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput;
}

/// The default tool set.
pub fn default_tools() -> Vec<Arc<dyn Tool>> {
    let mut v: Vec<Arc<dyn Tool>> = vec![
        Arc::new(tools::fs::ReadFile),
        Arc::new(tools::fs::WriteFile),
        Arc::new(tools::fs::Edit),
        Arc::new(tools::symbol::EditSymbol),
        Arc::new(tools::files::MultiEdit),
        Arc::new(tools::files::Delete),
        Arc::new(tools::files::Move),
        Arc::new(tools::patch::ApplyPatch),
        Arc::new(tools::fs::List),
        Arc::new(tools::ocr::Ocr),
        Arc::new(tools::search::Glob),
        Arc::new(tools::search::Grep),
        Arc::new(tools::shell::Bash::new(Shell::detect())),
        Arc::new(tools::todo::TodoWrite),
        Arc::new(tools::web::WebFetch),
        Arc::new(tools::notebook::NotebookEdit),
        Arc::new(tools::tasks::TaskCreate),
        Arc::new(tools::tasks::TaskGet),
        Arc::new(tools::tasks::TaskUpdate),
        Arc::new(tools::tasks::TaskList),
        Arc::new(tools::swarm::TeamCreate),
        Arc::new(tools::swarm::TeamDelete),
        Arc::new(tools::swarm::SendMessage),
        Arc::new(tools::websearch::WebSearch),
        Arc::new(tools::extra::AskUserQuestion),
        Arc::new(tools::extra::EnterPlanMode),
        Arc::new(tools::extra::ExitPlanMode),
        Arc::new(tools::extra::ConfigTool),
        Arc::new(tools::extra::SleepTool),
        Arc::new(tools::extra::ToolSearch),
        Arc::new(tools::extra::TaskOutput),
        Arc::new(tools::extra::TaskStop),
        Arc::new(tools::extra::CronCreate),
        Arc::new(tools::extra::CronList),
        Arc::new(tools::extra::CronDelete),
        Arc::new(tools::extra::SyntheticOutput),
        Arc::new(tools::lsp::LspTool),
        Arc::new(tools::powershell::PowerShellTool),
        Arc::new(tools::elevated::RunElevated),
        Arc::new(tools::worktree::EnterWorktree),
        Arc::new(tools::worktree::ExitWorktree),
    ];
    v.extend(tools::browser::tools());
    // While desktop control is off (`computer.enabled = false`) the model never sees these tools.
    if !tools::computer::disabled() {
        v.extend(computer_tools());
    }
    v
}

/// Screen, mouse, keyboard and UI-automation tools (listed by [`default_tools`] only when enabled).
pub fn computer_tools() -> Vec<Arc<dyn Tool>> {
    let mut v: Vec<Arc<dyn Tool>> = vec![
        Arc::new(tools::computer::Screenshot),
        Arc::new(tools::computer::LeftClick),
        Arc::new(tools::computer::RightClick),
        Arc::new(tools::computer::DoubleClick),
        Arc::new(tools::computer::MouseMove),
        Arc::new(tools::computer::TypeText),
        Arc::new(tools::computer::KeyPress),
        Arc::new(tools::computer::Scroll),
        Arc::new(tools::computer::LeftClickDrag),
        Arc::new(tools::computer::UndoLastAction),
    ];
    v.extend(tools::desktop::tools());
    v
}

/// Turn desktop control on or off for this process (config `computer.enabled`).
pub fn set_computer_enabled(on: bool) {
    tools::computer::set_enabled(on);
}

/// Chrome or Edge on this machine (`KOR_BROWSER` overrides), for headless printing and the agent browser.
pub fn find_browser() -> Option<std::path::PathBuf> {
    tools::browser::find_browser()
}

/// Desktop tools. They ask unless the mode is `bypassPermissions`.
pub fn is_computer_tool(name: &str) -> bool {
    tools::computer::is_computer_tool(name) || tools::chrome::is_action(name)
}

/// Window listing and placement for `kor layout` (§27.7, ADR 0059).
pub use tools::desktop::WindowControl;

/// Desktop control is on for this process (config `computer.enabled` / `KOR_COMPUTER_USE`).
pub fn computer_enabled() -> bool {
    tools::computer::enabled()
}

/// §19.1: the permission engine may run this desktop action without asking — the target app's
/// tier is `full`, the action is not destructive, and the user has not interrupted Kor.
pub fn computer_auto_allowed(name: &str, input: &Value) -> bool {
    if !tools::computer::COMPUTER_TOOLS.contains(&name) {
        // Window / accessibility tools name their target by handle; they keep asking.
        return false;
    }
    let (action, destructive) = tools::computer::action_of(name, input);
    kor_computer::auto_allowed(action, kor_computer::target_app(None).as_deref(), destructive)
}

/// §19.1 "human first": the user moved the mouse or pressed a key while Kor acted; the next
/// desktop action must be approved in every permission mode.
pub fn computer_paused() -> bool {
    kor_computer::paused().is_some()
}

/// The user approved a desktop action: resume (their approval click is not an interruption).
pub fn computer_approved() {
    kor_computer::approved();
}

/// First present string among `keys`.
pub(crate) fn field<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Path for humans and prompts, without the Windows verbatim prefix.
pub fn display_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\")
        .map_or_else(|| s.clone(), str::to_owned)
}

/// Receives a running tool's progress lines (queue position, logs, download progress).
pub type ProgressSink = Arc<dyn Fn(String) + Send + Sync>;

tokio::task_local! {
    static PROGRESS: ProgressSink;
}

/// Run `fut` (one tool call) with `sink` receiving its [`report_progress`] lines. The agent wraps
/// each call so its existing `ToolProgress` event carries them; parallel calls get their own sink.
pub async fn with_progress<F: std::future::Future>(sink: ProgressSink, fut: F) -> F::Output {
    PROGRESS.scope(sink, fut).await
}

/// Report a progress line for the tool call running in this task; a no-op outside one (CLI and
/// desktop callers of the same helpers print or emit their own progress).
pub fn report_progress(line: impl Into<String>) {
    let _ = PROGRESS.try_with(|s| s(line.into()));
}

/// Parse tool input into `T`, turning schema violations into an error result.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, ToolOutput> {
    serde_json::from_value(input).map_err(|e| ToolOutput::err(format!("invalid input: {e}")))
}

/// Keep the head and tail of long output.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let half = max / 2;
    let head_end = (0..=half)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    let tail_start = (s.len() - half..s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len());
    format!(
        "{}\n… [{} bytes omitted] …\n{}",
        &s[..head_end],
        tail_start - head_end,
        &s[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_scope_matches_programs_of_the_command_line() {
        let scoped = vec![
            ("DB_URL".to_owned(), "x".to_owned(), vec!["psql".to_owned()]),
            ("NPM_TOKEN".to_owned(), "y".to_owned(), vec!["npm".to_owned(), "pnpm".to_owned()]),
            ("NONE".to_owned(), "z".to_owned(), Vec::new()),
        ];
        let names = |c: &str| scoped_env_for(c, &scoped).map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(names("psql -c 'select 1'"), ["DB_URL"]);
        assert_eq!(names("cd db && PGHOST=x /usr/bin/psql -f a.sql"), ["DB_URL"]);
        assert_eq!(names("C:\\tools\\NPM.CMD publish | tee log"), ["NPM_TOKEN"]);
        assert!(names("cargo test; echo psql").is_empty());
        assert!(names("curl https://x").is_empty());
    }

    #[tokio::test]
    async fn progress_reaches_only_the_scoped_call() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let sink: ProgressSink = Arc::new(move |l| lock(&s2).push(l));
        report_progress("outside: dropped");
        with_progress(sink, async { report_progress("in queue (position 2)") }).await;
        assert_eq!(*lock(&seen), vec!["in queue (position 2)".to_owned()]);
    }

    #[test]
    fn truncate_keeps_both_ends_on_char_boundaries() {
        let s = "ğ".repeat(100);
        let t = truncate(&s, 50);
        assert!(t.starts_with('ğ') && t.ends_with('ğ'));
        assert!(t.contains("omitted"));
        assert_eq!(truncate("short", 50), "short");
    }

    #[test]
    fn display_path_strips_verbatim_prefix() {
        assert_eq!(display_path(std::path::Path::new(r"\\?\C:\x")), r"C:\x");
        assert_eq!(display_path(std::path::Path::new("/x")), "/x");
    }

    #[test]
    fn tool_names_are_unique() {
        // Another test may have turned desktop control on; count each tool once either way.
        let extra = computer_tools();
        let mut tools = default_tools();
        tools.retain(|t| !extra.iter().any(|e| e.name() == t.name()));
        tools.extend(extra);
        let mut names: Vec<_> = tools.iter().map(|t| t.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), tools.len());
    }
}
