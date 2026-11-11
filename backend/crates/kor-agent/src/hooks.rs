//! Command hooks.
//!
//! Each hook gets a JSON object on stdin (`session_id`, `transcript_path`, `cwd`,
//! `hook_event_name` plus event fields). Exit 0 succeeds (stdout may be JSON with
//! `decision`, `continue`, `hookSpecificOutput`); exit 2 blocks with stderr as the reason;
//! other codes are reported to the user and ignored.
//!
//! Two formats: Claude Code `settings.json` hooks (loaded by `kor-compat`, event names like
//! `PreToolUse`) and Kor's own `hooks.toml` (§15.1, event names in [`KOR_EVENTS`]), read from
//! `<config>/kor/hooks.toml` and, in a trusted project, `.kor/hooks.toml`:
//!
//! ```toml
//! [[pre-edit]]
//! command = "cargo fmt --check"   # run by the project's shell
//! matcher = "Edit|Write"          # optional tool-name regex
//! timeout = 30                    # optional, seconds
//! files = ["src/**/*.rs"]         # optional (§28.12): only for these files
//! branch = "main"                 # optional: only on this branch (glob)
//! model = "anthropic/*"           # optional: only with this model (glob)
//!
//! [[on-task-done]]
//! notify = "Task finished"        # instead of `command`: a message to the user
//!
//! [[post-edit]]
//! mcp = "linter/check"            # instead of `command`: call an MCP tool
//! args = { strict = true }
//! disabled = true                 # kept, not run
//! ```
//!
//! The agent emits Claude events; [`kor_events`] maps each to the Kor events it covers, so
//! both formats see the same moments. A Kor hook's JSON stdout is also fed back to the model.

use kor_platform::NoWindow as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_compat::{Compat, Eco, Hook, KorHook, Scope, Source};
use kor_tools::Shell;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::permissions::Decision;

const DEFAULT_TIMEOUT: u64 = 60;
/// §32.3 (ADR 0062): a plugin's hook may take this long before the plugin is suspended
/// (`[perf] plugin_budget_ms`; 0 = no budget).
static PLUGIN_BUDGET_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(5000);
/// Whether slow plugin hooks are also written to `perf.log` (`[perf] log`).
static PERF_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set from the config when a session starts (`kor_runtime`).
pub fn set_plugin_budget(ms: u64, perf_log: bool) {
    PLUGIN_BUDGET_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
    PERF_LOG.store(perf_log, std::sync::atomic::Ordering::Relaxed);
}

/// The installed plugin a hook file belongs to: (plugins root, plugin id) as
/// [`crate::plugins::installed`] names them (`name`, or `plugin@market` under `cache/`).
pub fn plugin_of(hook_file: &Path) -> Option<(PathBuf, String)> {
    let dir = hook_file
        .ancestors()
        .skip(1)
        .find(|d| d.join(".kor-plugin").is_dir() || d.join(".claude-plugin").is_dir())?;
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let parent = dir.parent()?;
    match parent.parent() {
        Some(cache) if cache.file_name().is_some_and(|n| n == "cache") => {
            let market = parent.file_name()?.to_string_lossy().into_owned();
            Some((cache.parent()?.to_owned(), format!("{name}@{market}")))
        }
        _ => Some((parent.to_owned(), name)),
    }
}

/// A plugin hook over its time budget: the plugin is suspended (it stays installed; `kor plugin
/// enable <id>` turns it back on) and the user is told. Returns the notice.
fn over_budget(h: &Hook, took: Duration) -> Option<String> {
    let budget = PLUGIN_BUDGET_MS.load(std::sync::atomic::Ordering::Relaxed);
    let ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
    if budget == 0 || ms <= budget {
        return None;
    }
    let (root, id) = plugin_of(&h.source.path)?;
    if PERF_LOG.load(std::sync::atomic::Ordering::Relaxed) {
        perf_log_line(&id, &h.event, ms);
    }
    crate::plugins::set_enabled(&root, &id, false).ok()?;
    Some(format!(
        "plugin `{id}` was suspended: its `{}` hook took {ms} ms (budget {budget} ms).          Turn it back on with `kor plugin enable {id}`.",
        h.event
    ))
}

/// One `perf.log` line in `kor_runtime::budgets::PerfEntry`'s format.
pub fn perf_entry(plugin: &str, event: &str, ms: u64) -> Value {
    #[allow(clippy::cast_precision_loss)] // milliseconds
    let ms = ms as f64;
    json!({
        "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "source": "plugin",
        "kind": "hook",
        "what": plugin,
        "ms": ms,
        "detail": event,
    })
}

fn perf_log_line(plugin: &str, event: &str, ms: u64) {
    let Some(p) = log_path().and_then(|l| l.parent().map(|d| d.join("perf.log"))) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = std::io::Write::write_all(
            &mut f,
            format!(
                "{}
",
                perf_entry(plugin, event, ms)
            )
            .as_bytes(),
        );
    }
}
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_TOML: u64 = 1024 * 1024;

/// Kor hook events (Master Build Prompt §15.1).
pub const KOR_EVENTS: &[&str] = &[
    "pre-edit",
    "post-edit",
    "pre-command",
    "post-command",
    "pre-commit",
    "on-task-start",
    "on-task-done",
    "on-approval-needed",
    "on-budget-alert",
    // §28.12: a tool call that failed.
    "on-error",
];

const EDIT_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit", "Delete"];
const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell", "RunElevated"];

/// The Kor events a Claude-format event stands for. `on-budget-alert` has no Claude twin; the
/// agent runs it by name.
pub fn kor_events(event: &str, tool: Option<&str>, fields: &Value) -> Vec<&'static str> {
    let is = |set: &[&str]| tool.is_some_and(|t| set.contains(&t));
    match event {
        "PreToolUse" | "PostToolUse" if is(EDIT_TOOLS) => {
            vec![if event == "PreToolUse" {
                "pre-edit"
            } else {
                "post-edit"
            }]
        }
        "PreToolUse" if is(COMMAND_TOOLS) => {
            let mut v = vec!["pre-command"];
            if fields["tool_input"]["command"]
                .as_str()
                .is_some_and(is_git_commit)
            {
                v.push("pre-commit");
            }
            v
        }
        "PostToolUse" if fields["tool_response"]["is_error"] == Value::Bool(true) => {
            let mut v = if is(COMMAND_TOOLS) {
                vec!["post-command"]
            } else if is(EDIT_TOOLS) {
                vec!["post-edit"]
            } else {
                Vec::new()
            };
            v.push("on-error");
            v
        }
        "PostToolUse" if is(COMMAND_TOOLS) => vec!["post-command"],
        "UserPromptSubmit" => vec!["on-task-start"],
        "Stop" => vec!["on-task-done"],
        "Notification" => vec!["on-approval-needed"],
        _ => Vec::new(),
    }
}

/// shortcut: any `git … commit` word sequence counts (also `git log --grep commit`); parse git's
/// global options if that ever misfires.
fn is_git_commit(cmd: &str) -> bool {
    cmd.split(['&', '|', ';', '\n']).any(|part| {
        let mut words = part.split_whitespace();
        words.any(|w| w == "git" || w == "git.exe") && words.any(|w| w == "commit")
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlHook {
    command: Option<String>,
    matcher: Option<String>,
    timeout: Option<u64>,
    files: Option<Files>,
    branch: Option<String>,
    model: Option<String>,
    mcp: Option<String>,
    args: Option<toml::Value>,
    notify: Option<String>,
    #[serde(default)]
    disabled: bool,
}

/// `files = "src/**"` or `files = ["a/**", "b/**"]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Files {
    One(String),
    Many(Vec<String>),
}

fn json_of(v: &toml::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Parse one `hooks.toml`. Unknown events and malformed entries become warnings.
pub fn parse_toml(text: &str, source: &Source, warnings: &mut Vec<String>) -> Vec<Hook> {
    let path = source.path.display();
    let table: BTreeMap<String, Vec<TomlHook>> = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("{path}: {e}"));
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for (event, hooks) in table {
        if !KOR_EVENTS.contains(&event.as_str()) {
            warnings.push(format!(
                "{path}: unknown hook event `{event}` (known: {})",
                KOR_EVENTS.join(", ")
            ));
            continue;
        }
        for h in hooks {
            let command = h
                .command
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty());
            let actions = usize::from(command.is_some())
                + usize::from(h.mcp.is_some())
                + usize::from(h.notify.is_some());
            if actions != 1 {
                warnings.push(format!(
                    "{path}: a `{event}` hook needs exactly one of command, mcp or notify"
                ));
                continue;
            }
            if h.mcp.as_deref().is_some_and(|m| {
                m.split_once('/')
                    .is_none_or(|(s, t)| s.is_empty() || t.is_empty())
            }) {
                warnings.push(format!("{path}: `{event}` hook: mcp must be `server/tool`"));
                continue;
            }
            let files = match h.files {
                Some(Files::One(f)) => vec![f],
                Some(Files::Many(v)) => v,
                None => Vec::new(),
            };
            let extra = KorHook {
                files,
                branch: h.branch,
                model: h.model,
                mcp: h.mcp.clone(),
                args: h.args.as_ref().map(|a| json_of(a).to_string()),
                notify: h.notify.clone(),
                disabled: h.disabled,
            };
            let shown = command.map_or_else(
                || match (&h.mcp, &h.notify) {
                    (Some(m), _) => format!("mcp:{m}"),
                    (_, Some(n)) => format!("notify:{n}"),
                    _ => String::new(),
                },
                str::to_owned,
            );
            out.push(Hook {
                event: event.clone(),
                matcher: h.matcher,
                command: shown,
                timeout_secs: h.timeout,
                source: source.clone(),
                kor: (extra != KorHook::default()).then_some(extra),
            });
        }
    }
    out
}

/// Add `<user_dir>/hooks.toml` and, when the project is trusted, `<root>/.kor/hooks.toml`.
pub fn add_kor_hooks(c: &mut Compat, root: &Path, user_dir: Option<&Path>) {
    let files = [
        user_dir.map(|d| (d.join("hooks.toml"), Scope::User)),
        Some((root.join(".kor").join("hooks.toml"), Scope::Project)),
    ];
    for (path, scope) in files.into_iter().flatten() {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if scope == Scope::Project && !c.trusted {
            c.warnings.push(format!(
                "{}: hooks ignored until you trust this project (`kor trust`)",
                path.display()
            ));
            continue;
        }
        if meta.len() > MAX_TOML {
            c.warnings
                .push(format!("{}: larger than 1 MiB, skipped", path.display()));
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let source = Source {
                    eco: Eco::Kor,
                    scope,
                    path,
                };
                let hooks = parse_toml(&text, &source, &mut c.warnings);
                c.hooks.extend(hooks);
            }
            Err(e) => c.warnings.push(format!("{}: {e}", path.display())),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutcome {
    /// The action must not happen; the text explains why (shown to the model).
    pub block: Option<String>,
    /// A `PreToolUse` permission decision.
    pub decision: Option<Decision>,
    /// Extra context for the model.
    pub context: Vec<String>,
    /// Problems to show the user (non-zero exits, timeouts).
    pub notices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HookRunner {
    pub hooks: Vec<Hook>,
    pub shell: Shell,
    pub root: PathBuf,
    pub session_id: String,
    pub transcript: Option<PathBuf>,
    /// The session's current `provider/model`, for `model = "…"` conditions (§28.12).
    pub model: Option<String>,
}

/// `<kor home>/hooks-log.jsonl` (same home as `kor_runtime::kor_home`): one line per hook run,
/// for the desktop hook editor's log view and `kor hooks log`.
pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("KOR_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|d| d.join("kor")))
        .map(|h| h.join("hooks-log.jsonl"))
}

fn log_run(h: &Hook, result: &str, took: Duration, message: Option<&String>) {
    let Some(p) = log_path() else { return };
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let line = json!({
        "at": at,
        "event": h.event,
        "command": h.command,
        "source": kor_tools::display_path(&h.source.path),
        "result": result,
        "ms": u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
        "message": message.map(|m| m.chars().take(500).collect::<String>()),
    });
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes());
    }
}

/// The checked-out branch of `root`, if it is a git work tree.
fn current_branch(root: &Path) -> Option<String> {
    crate::gitflow::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty())
}

fn glob_match(pattern: &str, text: &str) -> bool {
    globset::Glob::new(pattern).map_or(pattern == text, |g| g.compile_matcher().is_match(text))
}

/// `files`, `branch` and `model` conditions of a Kor hook (all that are set must hold).
pub fn conditions_met(
    k: &KorHook,
    root: &Path,
    input: &Value,
    branch: Option<&str>,
    model: Option<&str>,
) -> bool {
    if k.disabled {
        return false;
    }
    if !k.files.is_empty() {
        let ti = &input["tool_input"];
        let Some(p) = ti["file_path"]
            .as_str()
            .or_else(|| ti["path"].as_str())
            .or_else(|| ti["notebook_path"].as_str())
        else {
            return false;
        };
        let p = Path::new(p);
        let rel = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        if !k.files.iter().any(|g| glob_match(g, &rel)) {
            return false;
        }
    }
    if let Some(b) = &k.branch
        && !branch.is_some_and(|cur| glob_match(b, cur))
    {
        return false;
    }
    if let Some(m) = &k.model
        && !model.is_some_and(|cur| glob_match(m, cur))
    {
        return false;
    }
    true
}

fn matches(matcher: Option<&str>, tool: Option<&str>) -> bool {
    let (Some(m), Some(t)) = (matcher, tool) else {
        return true;
    };
    Regex::new(&format!("^(?:{m})$")).map_or(m == t, |r| r.is_match(t))
}

impl HookRunner {
    pub fn has(&self, event: &str) -> bool {
        let kor = kor_events(event, None, &Value::Null);
        self.hooks
            .iter()
            .any(|h| h.event == event || kor.contains(&h.event.as_str()))
    }

    /// Run every hook for `event` (and the Kor events it stands for) whose matcher fits `tool`,
    /// merging the outcomes. Each hook sees its own event name in `hook_event_name`.
    pub async fn run(&self, event: &str, tool: Option<&str>, fields: Value) -> HookOutcome {
        let mut out = HookOutcome::default();
        let kor = kor_events(event, tool, &fields);
        let mut input = json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript,
            "cwd": kor_tools::display_path(&self.root),
            "hook_event_name": event,
        });
        if let (Some(obj), Value::Object(extra)) = (input.as_object_mut(), fields) {
            obj.extend(extra);
        }
        let mut branch: Option<Option<String>> = None;
        for h in self.hooks.iter().filter(|h| {
            (h.event == event || kor.contains(&h.event.as_str()))
                && matches(h.matcher.as_deref(), tool)
        }) {
            if let Some(k) = &h.kor {
                let branch = branch.get_or_insert_with(|| current_branch(&self.root));
                if !conditions_met(
                    k,
                    &self.root,
                    &input,
                    branch.as_deref(),
                    self.model.as_deref(),
                ) {
                    continue;
                }
            }
            input["hook_event_name"] = h.event.clone().into();
            let started = std::time::Instant::now();
            let before = (out.block.is_some(), out.notices.len());
            match h.kor.as_ref() {
                Some(k) if k.notify.is_some() => {
                    out.notices.push(k.notify.clone().unwrap_or_default());
                }
                Some(k) if k.mcp.is_some() => self.run_mcp(h, k, &input, &mut out).await,
                _ => {
                    self.run_one(h, &h.event, &input.to_string(), &mut out)
                        .await;
                }
            }
            let result = if out.block.is_some() && !before.0 {
                "blocked"
            } else if out.notices.len() > before.1
                && h.kor.as_ref().is_none_or(|k| k.notify.is_none())
            {
                "failed"
            } else {
                "ok"
            };
            let elapsed = started.elapsed();
            log_run(
                h,
                result,
                elapsed,
                out.notices.last().filter(|_| out.notices.len() > before.1),
            );
            if let Some(n) = over_budget(h, elapsed) {
                out.notices.push(n);
            }
        }
        out
    }

    /// An `mcp = "server/tool"` hook: start the configured server, call the tool with `args`
    /// plus the event as `event`; its text output goes back to the model, a tool error to the user.
    #[allow(clippy::similar_names)] // args / argv: the call's JSON and the server's argv
    async fn run_mcp(&self, h: &Hook, k: &KorHook, input: &Value, out: &mut HookOutcome) {
        let Some((server, tool)) = k.mcp.as_deref().and_then(|m| m.split_once('/')) else {
            return;
        };
        let home = dirs::home_dir();
        let kor_dir = kor_platform::config_dir();
        let src = crate::mcp_tools::Sources {
            project_root: &self.root,
            project_trusted: true,
            home: home.as_deref(),
            kor_config_dir: kor_dir.as_deref(),
        };
        let (specs, _) = crate::mcp_tools::discover(&src, &|v| std::env::var(v).ok());
        let Some(spec) = specs.iter().find(|s| s.name == server) else {
            out.notices.push(format!(
                "hook `{}`: no MCP server `{server}` is configured",
                h.command
            ));
            return;
        };
        let mut args: Value = k
            .args
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_else(|| json!({}));
        if let Some(obj) = args.as_object_mut() {
            obj.insert("event".into(), input.clone());
        }
        let secs = h.timeout_secs.unwrap_or(DEFAULT_TIMEOUT).clamp(1, 600);
        let result = async {
            let client = crate::mcp_tools::McpClient::start(spec, &self.root).await?;
            client
                .request(
                    "tools/call",
                    json!({"name": tool, "arguments": args}),
                    Duration::from_secs(secs),
                    None,
                )
                .await
        }
        .await;
        match result {
            Ok(v) => {
                let o = crate::mcp_tools::to_output(&v);
                if o.is_error {
                    out.notices
                        .push(format!("hook `{}` failed: {}", h.command, o.content));
                } else if !o.content.trim().is_empty() {
                    out.context.push(o.content);
                }
            }
            Err(e) => out
                .notices
                .push(format!("hook `{}` failed: {e}", h.command)),
        }
    }

    async fn run_one(&self, h: &Hook, event: &str, payload: &str, out: &mut HookOutcome) {
        let mut args = self.shell.prefix.clone();
        args.push(h.command.clone());
        let mut cmd = tokio::process::Command::new(&self.shell.program);
        cmd.no_window();
        cmd.args(args)
            .current_dir(&self.root)
            .env("CLAUDE_PROJECT_DIR", &self.root)
            .env("KOR_PROJECT_DIR", &self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                out.notices
                    .push(format!("hook `{}` could not start: {e}", h.command));
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            // A hook that ignores stdin closes the pipe early; that is fine.
            let _ = stdin.write_all(payload.as_bytes()).await;
        }
        let secs = h.timeout_secs.unwrap_or(DEFAULT_TIMEOUT).clamp(1, 600);
        let result =
            tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output()).await;
        let Ok(Ok(o)) = result else {
            out.notices
                .push(format!("hook `{}` timed out after {secs}s", h.command));
            return;
        };
        let text = |b: &[u8]| {
            let s = String::from_utf8_lossy(&b[..b.len().min(MAX_OUTPUT)]);
            s.trim().to_owned()
        };
        let (stdout, stderr) = (text(&o.stdout), text(&o.stderr));
        match o.status.code() {
            Some(0) => apply_stdout(event, &stdout, out),
            Some(2) => {
                let reason = if stderr.is_empty() {
                    format!("blocked by hook `{}`", h.command)
                } else {
                    stderr
                };
                append(&mut out.block, reason);
            }
            code => out.notices.push(format!(
                "hook `{}` failed ({}): {stderr}",
                h.command,
                code.map_or_else(|| "signal".into(), |c| format!("exit {c}"))
            )),
        }
    }
}

fn apply_stdout(event: &str, stdout: &str, out: &mut HookOutcome) {
    let Ok(v) = serde_json::from_str::<Value>(stdout)
        .map_err(|_| ())
        .and_then(|v| if v.is_object() { Ok(v) } else { Err(()) })
    else {
        // Plain output becomes context for the model.
        if !stdout.is_empty() && matches!(event, "UserPromptSubmit" | "SessionStart") {
            out.context.push(stdout.to_owned());
        }
        return;
    };
    let reason = v["reason"].as_str().unwrap_or("blocked by hook").to_owned();
    if v["continue"] == Value::Bool(false) {
        append(
            &mut out.block,
            v["stopReason"]
                .as_str()
                .map_or(reason.clone(), str::to_owned),
        );
    }
    match v["decision"].as_str() {
        Some("block") => append(&mut out.block, reason.clone()),
        Some("approve") => out.decision = Some(merge(out.decision, Decision::Allow)),
        _ => {}
    }
    let specific = &v["hookSpecificOutput"];
    if let Some(d) = specific["permissionDecision"].as_str() {
        let d = match d {
            "allow" => Decision::Allow,
            "ask" => Decision::Ask,
            _ => Decision::Deny,
        };
        if d == Decision::Deny {
            let why = specific["permissionDecisionReason"]
                .as_str()
                .unwrap_or(&reason)
                .to_owned();
            append(&mut out.block, why);
        }
        out.decision = Some(merge(out.decision, d));
    }
    if let Some(c) = specific["additionalContext"].as_str() {
        out.context.push(c.to_owned());
    }
    // §15.1: a Kor hook's own JSON (not Claude control fields) goes back to the model as is.
    let control = [
        "continue",
        "decision",
        "hookSpecificOutput",
        "reason",
        "stopReason",
    ];
    if KOR_EVENTS.contains(&event) && !control.iter().any(|k| v.get(k).is_some()) {
        out.context.push(stdout.to_owned());
    }
}

fn append(slot: &mut Option<String>, text: String) {
    *slot = Some(match slot.take() {
        Some(prev) => format!("{prev}\n{text}"),
        None => text,
    });
}

/// The strictest decision wins: deny > ask > allow.
fn merge(a: Option<Decision>, b: Decision) -> Decision {
    let rank = |d: Decision| match d {
        Decision::Allow => 0,
        Decision::Ask => 1,
        Decision::Deny => 2,
    };
    match a {
        Some(a) if rank(a) >= rank(b) => a,
        _ => b,
    }
}

/// Tool input uses `file_path`, `old_string`, and `new_string`, so existing hook scripts keep working.
pub fn claude_input(root: &std::path::Path, input: &Value) -> Value {
    let mut v = input.clone();
    if let Some(obj) = v.as_object_mut() {
        if let Some(p) = input["path"].as_str()
            && let Ok(abs) = kor_tools::resolve(root, p)
        {
            obj.insert("file_path".into(), kor_tools::display_path(&abs).into());
        }
        if let Some(first) = input["edits"].as_array().and_then(|a| a.first()) {
            obj.insert("old_string".into(), first["old_string"].clone());
            obj.insert("new_string".into(), first["new_string"].clone());
        }
        if let Some(t) = input["timeout_secs"].as_u64() {
            obj.insert("timeout".into(), (t * 1000).into());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_compat::{Eco, Scope, Source};

    fn runner(hooks: Vec<(&str, Option<&str>, String)>) -> (HookRunner, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks
            .into_iter()
            .map(|(event, matcher, command)| Hook {
                event: event.into(),
                matcher: matcher.map(str::to_owned),
                command,
                timeout_secs: Some(20),
                source: Source {
                    eco: Eco::Claude,
                    scope: Scope::User,
                    path: "s".into(),
                },
                kor: None,
            })
            .collect();
        let r = HookRunner {
            hooks,
            shell: Shell::detect(),
            root: dir.path().canonicalize().unwrap(),
            session_id: "s1".into(),
            transcript: None,
            model: None,
        };
        (r, dir)
    }

    #[test]
    fn slow_plugin_hooks_suspend_their_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("plugins");
        let market = root.join("cache").join("m").join("p");
        std::fs::create_dir_all(market.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(root.join("direct").join(".kor-plugin")).unwrap();
        let file = market.join("hooks").join("hooks.json");
        assert_eq!(plugin_of(&file), Some((root.clone(), "p@m".to_owned())));
        assert_eq!(
            plugin_of(&root.join("direct").join("hooks.toml")),
            Some((root.clone(), "direct".to_owned()))
        );
        assert_eq!(plugin_of(Path::new("/no/plugin/hooks.toml")), None);
        let h = Hook {
            event: "PreToolUse".into(),
            matcher: None,
            command: "x".into(),
            timeout_secs: None,
            source: Source {
                eco: Eco::Claude,
                scope: Scope::User,
                path: file,
            },
            kor: None,
        };
        set_plugin_budget(100, false);
        assert_eq!(over_budget(&h, Duration::from_millis(80)), None);
        assert!(crate::plugins::is_enabled(&root, "p@m"));
        let n = over_budget(&h, Duration::from_millis(250)).unwrap();
        assert!(n.contains("p@m") && n.contains("250 ms"), "{n}");
        assert!(!crate::plugins::is_enabled(&root, "p@m"));
        // 0 turns the budget off.
        set_plugin_budget(0, false);
        assert_eq!(over_budget(&h, Duration::from_secs(60)), None);
        set_plugin_budget(5000, false);
        let e = perf_entry("p@m", "PreToolUse", 250);
        assert_eq!(
            (e["source"].as_str(), e["kind"].as_str(), e["ms"].as_f64()),
            (Some("plugin"), Some("hook"), Some(250.0))
        );
    }

    /// A command that prints `text` to stdout (or stderr) and exits with `code`.
    fn emit(text: &str, code: i32, to_stderr: bool) -> String {
        if kor_tools::Shell::detect().prefix.len() > 1 {
            let q = text.replace('\'', "''");
            if to_stderr {
                format!("[Console]::Error.WriteLine('{q}'); exit {code}")
            } else {
                format!("[Console]::Out.WriteLine('{q}'); exit {code}")
            }
        } else {
            let q = text.replace('\'', "'\\''");
            format!(
                "printf '%s\\n' '{q}' {}; exit {code}",
                if to_stderr { ">&2" } else { "" }
            )
        }
    }

    #[tokio::test]
    async fn exit_codes_and_json_output() {
        let (r, _d) = runner(vec![
            ("PreToolUse", Some("Bash"), emit("no rm please", 2, true)),
            (
                "PreToolUse",
                Some("Edit|Write"),
                emit(
                    r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#,
                    0,
                    false,
                ),
            ),
            (
                "UserPromptSubmit",
                None,
                emit("today is release day", 0, false),
            ),
            ("Stop", None, emit("oops", 7, true)),
            ("PreCompact", None, emit("not yet", 2, true)),
        ]);
        let o = r
            .run("PreToolUse", Some("Bash"), json!({"tool_name": "Bash"}))
            .await;
        assert_eq!(o.block.as_deref(), Some("no rm please"));

        let o = r.run("PreToolUse", Some("Write"), json!({})).await;
        assert_eq!((o.block, o.decision), (None, Some(Decision::Allow)));

        let o = r.run("PreCompact", None, json!({"trigger": "auto"})).await;
        assert_eq!(o.block.as_deref(), Some("not yet"));

        let o = r.run("PreToolUse", Some("Read"), json!({})).await;
        assert_eq!(o, HookOutcome::default());

        let o = r
            .run("UserPromptSubmit", None, json!({"prompt": "hi"}))
            .await;
        assert_eq!(o.context, ["today is release day"]);

        let o = r.run("Stop", None, json!({})).await;
        assert!(
            o.block.is_none() && o.notices[0].contains("exit 7"),
            "{o:?}"
        );
    }

    #[tokio::test]
    async fn hook_receives_json_on_stdin() {
        let save = if kor_tools::Shell::detect().prefix.len() > 1 {
            "[IO.File]::WriteAllText('hook_input.json', [Console]::In.ReadToEnd())"
        } else {
            "cat > hook_input.json"
        }
        .to_owned();
        let (r, _d) = runner(vec![("SessionStart", None, save)]);
        let o = r
            .run("SessionStart", None, json!({"source": "startup"}))
            .await;
        assert!(o.notices.is_empty(), "{o:?}");
        let saved = std::fs::read_to_string(r.root.join("hook_input.json")).unwrap();
        let v: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(v["hook_event_name"], "SessionStart");
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["source"], "startup");
    }

    #[tokio::test]
    async fn kor_events_run_on_the_matching_claude_moments() {
        let (r, _d) = runner(vec![
            ("pre-edit", None, emit("format first", 2, true)),
            ("post-command", None, emit(r#"{"lint":"clean"}"#, 0, false)),
            ("pre-commit", None, emit("tests first", 2, true)),
            (
                "on-task-done",
                None,
                emit("{\"continue\":false,\"stopReason\":\"wait\"}", 0, false),
            ),
        ]);
        let o = r.run("PreToolUse", Some("Write"), json!({})).await;
        assert_eq!(o.block.as_deref(), Some("format first"));
        let o = r.run("PostToolUse", Some("Bash"), json!({})).await;
        assert_eq!(o.context, [r#"{"lint":"clean"}"#]);
        let commit = json!({"tool_input": {"command": "cargo fmt && git commit -m x"}});
        let o = r.run("PreToolUse", Some("Bash"), commit).await;
        assert_eq!(o.block.as_deref(), Some("tests first"));
        let o = r
            .run(
                "PreToolUse",
                Some("Bash"),
                json!({"tool_input": {"command": "git status"}}),
            )
            .await;
        assert_eq!(o, HookOutcome::default());
        let o = r.run("Stop", None, json!({})).await;
        assert_eq!(o.block.as_deref(), Some("wait"));
        assert!(r.has("Stop") && !r.has("Notification"));
    }

    #[test]
    fn hooks_toml_is_read_and_project_needs_trust() {
        let dir = tempfile::tempdir().unwrap();
        let (user, root) = (dir.path().join("cfg"), dir.path().join("proj"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(root.join(".kor")).unwrap();
        std::fs::write(
            user.join("hooks.toml"),
            "[[pre-edit]]\ncommand = \"fmt\"\nmatcher = \"Edit\"\ntimeout = 5\n\n[[on-lunch]]\ncommand = \"x\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".kor").join("hooks.toml"),
            "[[on-task-done]]\ncommand = \"notify\"\n",
        )
        .unwrap();

        let mut c = Compat::default();
        add_kor_hooks(&mut c, &root, Some(&user));
        assert_eq!(c.hooks.len(), 1);
        let h = &c.hooks[0];
        assert_eq!(
            (
                h.event.as_str(),
                h.matcher.as_deref(),
                h.command.as_str(),
                h.timeout_secs
            ),
            ("pre-edit", Some("Edit"), "fmt", Some(5))
        );
        assert_eq!((h.source.eco, h.source.scope), (Eco::Kor, Scope::User));
        assert!(c.warnings.iter().any(|w| w.contains("on-lunch")));
        assert!(c.warnings.iter().any(|w| w.contains("kor trust")));

        let mut c = Compat {
            trusted: true,
            ..Compat::default()
        };
        add_kor_hooks(&mut c, &root, None);
        assert_eq!(c.hooks.len(), 1);
        assert_eq!(c.hooks[0].source.scope, Scope::Project);

        let mut w = Vec::new();
        let src = c.hooks[0].source.clone();
        assert!(parse_toml("[[pre-edit]]\nbogus = 1\ncommand = \"x\"\n", &src, &mut w).is_empty());
        assert!(parse_toml("[[pre-edit]]\ncommand = \" \"\n", &src, &mut w).is_empty());
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn kor_event_mapping() {
        let none = Value::Null;
        assert_eq!(
            kor_events("PostToolUse", Some("Edit"), &none),
            ["post-edit"]
        );
        assert_eq!(
            kor_events("PreToolUse", Some("PowerShell"), &none),
            ["pre-command"]
        );
        assert_eq!(
            kor_events("UserPromptSubmit", None, &none),
            ["on-task-start"]
        );
        assert_eq!(
            kor_events("Notification", None, &none),
            ["on-approval-needed"]
        );
        assert!(kor_events("PreToolUse", Some("Read"), &none).is_empty());
        assert!(is_git_commit("git -C x commit -m y"));
        assert!(!is_git_commit("git status; echo commit"));
    }

    #[test]
    fn claude_shaped_input() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let v = claude_input(
            &root,
            &json!({"path": "a.rs", "edits": [{"old_string": "x", "new_string": "y"}]}),
        );
        assert!(v["file_path"].as_str().unwrap().ends_with("a.rs"));
        assert_eq!(
            (v["old_string"].as_str(), v["new_string"].as_str()),
            (Some("x"), Some("y"))
        );
        assert!(matches(Some("Edit|Write"), Some("Write")));
        assert!(!matches(Some("Edit"), Some("EditX")));
        assert!(matches(None, Some("Bash")));
    }
}
