//! Layered TOML configuration with per-key provenance.
//!
//! Layers, lowest to highest: built-in defaults → system → user → project
//! (`.kor/config.toml`, nearest ancestor) → environment → command line.
//! Every effective key records the layer that set it, for `kor config explain`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use kor_core::{PermissionMode, PrivacyRequirement, PrivacyTier};
use kor_i18n::LangPref;
use serde::{Deserialize, Serialize};
use toml::Table;
pub use toml::Value;

/// Config files larger than this are rejected rather than parsed.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const ENV_PREFIX: &str = "KOR_";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: Ui,
    pub agent: Agent,
    pub privacy: Privacy,
    pub permissions: Permissions,
    pub computer: Computer,
    /// System profile and health alerts (§19.2–19.3, ADR 0059), `[health]`.
    pub health: Health,
    pub update: Update,
    pub routing: Routing,
    pub budget: Budget,
    pub context: ContextBudget,
    /// Project memory and the knowledge base (§6.1, §31.3), `[memory]`.
    pub memory: Memory,
    /// Live project graph for the agent (§7.5), `[graph]`.
    pub graph: GraphConfig,
    /// Command sandbox (§6.4, ADR 0033), `[sandbox]`; `sandbox = "on"` is short for its `mode`.
    pub sandbox: Sandbox,
    /// Self-hosted team server (§11), `[team]`.
    pub team: Team,
    /// Offline / air-gap (§6.5), `[offline]`.
    pub offline: Offline,
    /// Secret vault (§6.6, ADR 0036), `[vault]`.
    pub vault: Vault,
    /// Universal Git (§8.8, ADR 0040), `[forge]`.
    pub forge: Forge,
    /// Event-driven tasks run by `kor daemon` (§8.4, ADR 0039), `[[triggers]]`.
    pub triggers: Vec<Trigger>,
    /// `kor daemon` (§8.4, ADR 0039), `[daemon]`.
    pub daemon: Daemon,
    /// Notifications and asynchronous approval (§11.3, ADR 0039), `[notify]`.
    pub notify: Notify,
    /// Multi-agent tasks (`kor task`, §8.2, ADR 0037), `[orchestrator]`.
    pub orchestrator: Orchestrator,
    /// Session recording (§15.9, ADR 0043), `[record]`; `KOR_RECORD__ENABLED=true` for one session.
    pub record: Record,
    /// Devcontainer / Nix / mise awareness (§15.7, ADR 0043), `[env]`.
    pub env: DevEnv,
    /// `kor audit` analysis policy (§9, ADR 0046), `[audit]`.
    pub audit: AuditPolicy,
    /// `kor stats` / `kor debt` (§9, ADR 0047), `[stats]`.
    pub stats: Stats,
    /// Desktop tray icon (§27.1, ADR 0051), `[tray]`.
    pub tray: Tray,
    /// Always-on-top status window (§27.8, ADR 0051), `[pip]`.
    pub pip: Pip,
    /// Every automatic behaviour of Kor, one switch each (§28.14, ADR 0050), `[auto]`.
    pub auto: Auto,
    /// Terminal interface (§28.11, ADR 0050), `[tui]`.
    pub tui: Tui,
    /// Forced plan mode, the "stop and research" lock (§28.2, ADR 0050), `[plan]`.
    pub plan: PlanConfig,
    /// Key layout shortcut (§28.10): `[keys] mode = "vim"` overrides `ui.tui_keys`; `keymap.toml`
    /// files override both.
    pub keys: KeysConfig,
    /// Desktop OS integration (§25.3, ADR 0052), `[desktop]`.
    pub desktop: DesktopConfig,
    /// "Kor ile Aç" file-manager integration (§25.1, ADR 0052), `[shell]`.
    pub shell: ShellConfig,
    /// Integrations the desktop installs once on first run (ADR 0052), `[integrations]`.
    pub integrations: Integrations,
    /// Natural-language command routing (§30, ADR 0056), `[intent]`.
    pub intent: IntentConfig,
    /// PATH tool catalog and user tool definitions (§26.2, ADR 0056), `[tools]`.
    pub tools: ToolsConfig,
    /// Image, video and audio generation (§20.2, ADR 0060), `[media]`.
    pub media: MediaConfig,
    /// Which browser the `browser_*` tools drive (§18.2, ADR 0058), `[browser]`.
    pub browser: BrowserConfig,
    /// Performance contract (§32.3, ADR 0062), `[perf]`.
    pub perf: PerfConfig,
    /// Agent Client Protocol (§28.8, ADR 0057), `[acp]`: other ACP agents Kor may run as
    /// sub-agents.
    pub acp: AcpConfig,
}

/// `[acp]`: `[acp.agents.<name>] command = "…", args = […], env = { … }` — ACP agents the
/// `AcpAgent` tool and `kor acp run <name>` start as sub-agents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcpConfig {
    pub agents: BTreeMap<String, AcpAgent>,
}

/// One ACP agent program.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcpAgent {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// `[perf]`: `log` appends slow frames and calls to `<kor home>/perf.log` (local only, never sent;
/// `KOR_PERF__LOG=true` for one run); `low_power` turns animations off and slows background
/// indexing (`auto`: on battery below 20% or on a machine with 2 or fewer cores);
/// `plugin_budget_ms` is the time a plugin hook may take before the plugin is suspended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PerfConfig {
    pub log: bool,
    pub low_power: LowPower,
    pub plugin_budget_ms: u64,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            log: false,
            low_power: LowPower::Auto,
            plugin_budget_ms: 5000,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LowPower {
    #[default]
    Auto,
    On,
    Off,
}

/// `[browser] backend`: `chrome` (alias `extension`) drives the user's own Chrome through the Kor
/// extension, `headless` Kor's isolated browser, `auto` the extension when it is paired and
/// connected, else headless.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserBackend {
    #[default]
    Auto,
    #[serde(alias = "extension")]
    Chrome,
    Headless,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    pub backend: BrowserBackend,
}

/// `[intent]` (§30): free text that names a Kor command is routed to it. `enabled = false` sends
/// every text to the agent; `auto_run = false` confirms even read-only commands; `model_fallback`
/// lets a cheap model pick among close candidates (never in privacy/offline mode unless local).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntentConfig {
    pub enabled: bool,
    pub auto_run: bool,
    pub model_fallback: bool,
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_run: true,
            model_fallback: true,
        }
    }
}

/// `[tools]` (§26.2): `deny` hides and refuses commands by name (`["rm", "curl"]`); a non-empty
/// `allow` limits the catalog and user tools to those names; `catalog = false` keeps the PATH
/// catalog out of the agent's prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    pub catalog: bool,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            catalog: true,
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }
}

impl ToolsConfig {
    /// Whether the command `name` (`git`, `ffmpeg.exe`) may be offered to and run by the agent.
    pub fn permits(&self, name: &str) -> bool {
        let n = name.trim_end_matches(".exe").to_ascii_lowercase();
        let hit = |list: &[String]| {
            list.iter()
                .any(|x| x.trim_end_matches(".exe").eq_ignore_ascii_case(&n))
        };
        !hit(&self.deny) && (self.allow.is_empty() || hit(&self.allow))
    }
}

/// `[desktop]`: `hotkey` brings Kor to the front with the command palette (§25.3, default
/// `CmdOrCtrl+Shift+K`; empty turns it off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopConfig {
    pub hotkey: String,
}

/// `[shell]` (§25.1): `terminal` is the terminal "Kor CLI ile aç" opens (`wt`, `pwsh`, `cmd`,
/// `terminal`, `iterm`, `gnome-terminal`, `konsole`, `kitty`, `alacritty`, `wezterm`…; unset = the
/// platform default); `context_menu.extended` adds "Projeyi açıkla / Hata bul / Bu dosyayı test et"
/// (re-run `kor integrate install` or toggle it in the desktop settings).
///
/// `suggest` (§15.4, off by default) adds inline command suggestions to the `kor shell install`
/// block (re-run it after changing): shell history and project commands first, then
/// `suggest_model` (`provider/model`, e.g. `ollama/qwen2.5-coder:1.5b`; unset = no model) within
/// `suggest_budget_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    pub terminal: Option<String>,
    pub context_menu: ContextMenuConfig,
    pub suggest: bool,
    pub suggest_model: Option<String>,
    pub suggest_budget_ms: u64,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            terminal: None,
            context_menu: ContextMenuConfig::default(),
            suggest: false,
            suggest_model: None,
            suggest_budget_ms: 800,
        }
    }
}

/// `[integrations]` (ADR 0052, user decision 11 Nov 2026): the desktop's first run installs the
/// right-click menu, the default shell's integration and this OS's integrations once (recorded in
/// the data dir, so a later uninstall is never undone). `auto = false` skips that run; each switch
/// leaves one item out; `os` turns single §27.10 items off (`os.powertoys-run = false`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Integrations {
    pub auto: bool,
    pub context_menu: bool,
    pub shell: bool,
    pub os: BTreeMap<String, bool>,
}

impl Default for Integrations {
    fn default() -> Self {
        Self {
            auto: true,
            context_menu: true,
            shell: true,
            os: BTreeMap::new(),
        }
    }
}

impl Integrations {
    /// §27.10 item `id` is installed by the first run. Off unless listed: Wayland portals (four
    /// system permission dialogs), Shortcuts.app actions (one "Add Shortcut" sheet each) and the
    /// `systemd --user` service (a background daemon) — each needs the user present or runs Kor
    /// in the background.
    pub fn os_on(&self, id: &str) -> bool {
        self.os
            .get(id)
            .copied()
            .unwrap_or(!matches!(id, "portals" | "shortcuts" | "systemd-user"))
    }
}

/// `[shell.context_menu]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextMenuConfig {
    pub extended: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            hotkey: "CmdOrCtrl+Shift+K".into(),
        }
    }
}

/// `[auto]` (§28.14): nothing Kor does on its own is mandatory. Each key turns one automatic
/// behaviour off; `true` keeps today's behaviour, so the defaults change nothing. Where an older
/// key exists too (`memory.inject`, `graph.watch`, `update.auto`, `routing.enabled`, …) both must
/// be on. `kor config explain auto` lists them with their values and sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // one switch per automatic behaviour: the config schema itself
pub struct Auto {
    /// Summarize the conversation when the context window fills up.
    pub compact: bool,
    /// Put knowledge-base records and auto memory into the prompt at task start.
    pub memory_inject: bool,
    /// Write "what we learned" records and extracted memories after a task.
    pub memory_record: bool,
    /// Keep the project graph updated while a session is open (file watcher).
    pub graph_watch: bool,
    /// Write `hotspots.md` of the project memory from the graph.
    pub hotspots: bool,
    /// Install a verified newer release on its own (CLI and desktop).
    pub update: bool,
    /// Refresh the model catalog and model lists when they are stale.
    pub catalog_refresh: bool,
    /// Send each task to the model of its category (cost router).
    pub routing: bool,
    /// Run the project's checker after edits (verified edit loop).
    pub verify: bool,
    /// Allow non-destructive calls on their own in `approval = "auto"` triggers.
    pub trigger_approve: bool,
    /// Play the notification sound (desktop).
    pub sound: bool,
    /// Switch to plan mode when the message says "plan first / do not touch".
    pub plan_intent: bool,
    /// Suggest a title and tags when a session ends.
    pub session_suggest: bool,
}

impl Default for Auto {
    fn default() -> Self {
        Self {
            compact: true,
            memory_inject: true,
            memory_record: true,
            graph_watch: true,
            hotspots: true,
            update: true,
            catalog_refresh: true,
            routing: true,
            verify: true,
            trigger_approve: true,
            sound: true,
            plan_intent: true,
            session_suggest: true,
        }
    }
}

/// `[tui]` (§28.11). The TUI runs inline on the main screen (scrollback stays) unless
/// `alt_screen = true`; it sets the terminal title only with `title = true` and restores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tui {
    pub alt_screen: bool,
    pub title: bool,
    /// Inline images: `auto` (detect kitty / sixel), `kitty`, `sixel` or `off`.
    pub images: ImageProtocol,
    /// Capture the mouse (click and wheel on panels); off (the default) keeps the terminal's own
    /// scrolling and selection.
    pub mouse: bool,
}

impl Default for Tui {
    fn default() -> Self {
        Self {
            alt_screen: false,
            title: false,
            images: ImageProtocol::Auto,
            mouse: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageProtocol {
    #[default]
    Auto,
    Kitty,
    Sixel,
    Off,
}

/// `[plan]` (§28.2): with `lock = true` write tools are taken away until the user approves a plan
/// (`kor plan approve`, `/go`, the desktop button); plans live in `.kor/plans/<date>-<slug>.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanConfig {
    pub lock: bool,
}

/// `[keys]` (§28.10).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeysConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TuiKeys>,
}

/// Every `[auto]` key with the older keys that must also be on for the behaviour to run.
pub const AUTO_KEYS: &[(&str, &[&str])] = &[
    ("compact", &["context.auto_summarize_percent"]),
    ("memory_inject", &["memory.inject"]),
    ("memory_record", &["memory.learn"]),
    ("graph_watch", &["graph.enabled", "graph.watch"]),
    ("hotspots", &["graph.enabled"]),
    ("update", &["update.auto"]),
    ("catalog_refresh", &[]),
    ("routing", &["routing.enabled"]),
    ("verify", &["agent.verify"]),
    ("trigger_approve", &["triggers.approval"]),
    ("sound", &[]),
    ("plan_intent", &["plan.lock"]),
    ("session_suggest", &[]),
];

impl Auto {
    /// The value of one `[auto]` key by name.
    pub fn get(&self, key: &str) -> Option<bool> {
        Some(match key {
            "compact" => self.compact,
            "memory_inject" => self.memory_inject,
            "memory_record" => self.memory_record,
            "graph_watch" => self.graph_watch,
            "hotspots" => self.hotspots,
            "update" => self.update,
            "catalog_refresh" => self.catalog_refresh,
            "routing" => self.routing,
            "verify" => self.verify,
            "trigger_approve" => self.trigger_approve,
            "sound" => self.sound,
            "plan_intent" => self.plan_intent,
            "session_suggest" => self.session_suggest,
            _ => return None,
        })
    }
}

impl Config {
    /// Fold `[auto]` switches into the older keys they govern, so every reader of those keys
    /// (CLI, desktop, daemon) respects them without knowing `[auto]` (§28.14).
    pub fn apply_auto(&mut self) {
        let a = self.auto.clone();
        self.memory.inject &= a.memory_inject;
        self.memory.learn &= a.memory_record;
        self.graph.watch &= a.graph_watch;
        self.update.auto &= a.update;
        self.routing.enabled &= a.routing;
        if !a.verify {
            self.agent.verify = VerifyMode::Off;
        }
        if !a.trigger_approve {
            for t in &mut self.triggers {
                if t.approval == Approval::Auto {
                    t.approval = Approval::Notify;
                }
            }
        }
    }

    /// TUI / composer key layout: `[keys] mode` when set, else `ui.tui_keys`.
    pub fn key_mode(&self) -> TuiKeys {
        self.keys.mode.unwrap_or(self.ui.tui_keys)
    }
}

/// `[tray]`: the tray icon, and whether closing the main window keeps Kor running there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tray {
    pub enabled: bool,
    pub close_to_tray: bool,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            enabled: true,
            close_to_tray: true,
        }
    }
}

/// `[media]` (§20.2, ADR 0060): `[media.defaults.<capability>]` holds parameter values used
/// without asking (`duration = "5s"`, `model = "fal:fal-ai/flux/schnell"`); `confirm_above` asks
/// before a generation estimated above that many dollars (0 = never); `ask = false` stops the
/// questions about optional choices (required inputs are still asked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaConfig {
    pub ask: bool,
    pub confirm_above: f64,
    pub defaults: BTreeMap<String, Table>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            ask: true,
            confirm_above: 0.0,
            defaults: BTreeMap::new(),
        }
    }
}

/// `[pip]`: the small always-on-top status window. `auto_show` opens it when a task starts and
/// hides it 5 s after the last one ends; `hide_fullscreen` hides it while another app is full
/// screen; `opacity` is 0.3–1.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pip {
    pub auto_show: bool,
    pub hide_fullscreen: bool,
    pub opacity: f64,
}

impl Default for Pip {
    fn default() -> Self {
        Self {
            auto_show: false,
            hide_fullscreen: true,
            opacity: 0.95,
        }
    }
}

/// `[stats] team = true` shows the team ranking in `kor stats` (off by default: §9.1, only the
/// user sees their numbers); `debt_threshold_hours` is where `kor debt` projects to (0 = 400 h).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stats {
    pub team: bool,
    pub debt_threshold_hours: u32,
}

/// `[audit]` analysis policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditPolicy {
    pub licenses: LicensePolicy,
}

/// `[audit.licenses] deny = ["AGPL-3.0*", "SSPL-1.0"]`: SPDX ids (`*` wildcard, any case) that
/// `kor audit --licenses` reports as critical when a dependency's license cannot avoid them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LicensePolicy {
    pub deny: Vec<String>,
}

/// `[record] enabled = true`: every agent session is written to `.kor/sessions/<id>/`
/// (`events.jsonl` + asciinema `session.cast`), secrets masked. Off by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Record {
    pub enabled: bool,
}

/// `[env]`: commands run inside the project's detected environment (devcontainer, compose, nix,
/// mise/asdf) and pinned tool versions are checked before the first one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevEnv {
    /// `false`: ignore the project's environment files.
    pub enabled: bool,
    /// `stop` (default): refuse commands when a pinned version does not match or the
    /// environment's tool is missing; `warn`: run them and say so.
    pub on_mismatch: OnMismatch,
    /// Service of `docker-compose.dev.yml` commands run in; empty = the first one in the file.
    pub service: String,
    /// Run commands inside the project's container (devcontainer, docker compose). Off by
    /// default: the container is often not running, and its commands bypass the OS sandbox.
    pub containers: bool,
}

impl Default for DevEnv {
    fn default() -> Self {
        Self {
            enabled: true,
            on_mismatch: OnMismatch::Stop,
            service: String::new(),
            containers: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnMismatch {
    #[default]
    Stop,
    Warn,
}

/// `[orchestrator]`: how `kor task "<goal>" --agents N` splits and runs work. Empty model
/// strings mean "the session model" (sub-agents: the smart cost router decides per sub-task).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Orchestrator {
    /// Sub-agents when `--agents` is not given (1–16).
    pub agents: u8,
    /// Sub-agents running at the same time (1–16).
    pub max_parallel: u8,
    /// `provider/model` that plans and re-plans.
    pub model: String,
    /// `provider/model` that reviews each sub-agent's diff before merge.
    pub review_model: String,
    /// Pinned model per role: `[orchestrator.models] backend = "openai/gpt-4.1"`.
    pub models: BTreeMap<String, String>,
    /// Whole-task budget in USD; 0 = none. Each sub-agent gets an equal share unless
    /// `budget_per_agent_usd` is set.
    pub budget_usd: f64,
    pub budget_per_agent_usd: f64,
    /// Run (sandboxed) in each sub-agent's worktree before review and on the merged branch;
    /// empty = no tests.
    pub test_command: String,
    /// Tool-loop turns per sub-agent.
    pub max_turns: u32,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self {
            agents: 5,
            max_parallel: 4,
            model: String::new(),
            review_model: String::new(),
            models: BTreeMap::new(),
            budget_usd: 0.0,
            budget_per_agent_usd: 0.0,
            test_command: String::new(),
            max_turns: 40,
        }
    }
}

/// What fires a `[[triggers]]` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
    /// A push to `branch` (local git hook or forge webhook).
    GitPush,
    /// A five-field cron `schedule` (local time).
    Cron,
    /// `POST <path>` on the daemon, signed with the vault secret named by `secret`.
    Webhook,
    /// A line of `source` matching `pattern`.
    LogMatch,
}

/// One `[[triggers]]` entry; every fire is a queued, audited agent task in a fresh worktree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub event: TriggerEvent,
    pub task: String,
    /// Unique name (`kor triggers test <name>`); default `<event>-<n>`.
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// `git_push`: branch filter (empty = any branch).
    #[serde(default)]
    pub branch: String,
    /// `cron`: `min hour day-of-month month day-of-week`.
    #[serde(default)]
    pub schedule: String,
    /// `webhook`: request path, e.g. `/hooks/deploy`.
    #[serde(default)]
    pub path: String,
    /// `webhook` (required) and `git_push` from a forge: vault entry holding the HMAC secret.
    #[serde(default)]
    pub secret: String,
    /// `log_match`: a file path, `journald:<unit>` or `docker:<container>`.
    #[serde(default)]
    pub source: String,
    /// `log_match`: regular expression.
    #[serde(default)]
    pub pattern: String,
    /// `log_match`: quiet period after a fire, seconds (default 300).
    #[serde(default)]
    pub debounce_secs: u64,
    /// `provider/model` for this trigger's tasks (default: `agent.model`).
    #[serde(default)]
    pub model: String,
    /// Spend limit per run in USD (0 = none); the run stops when reached.
    #[serde(default)]
    pub budget_usd: f64,
    /// Project folder for a trigger in the user config (a project's own triggers use the project).
    #[serde(default)]
    pub root: String,
    /// Permission requests of a run: `notify` (send every one for approval), `auto` (allow
    /// non-destructive ones, send destructive ones), `deny` (refuse all).
    #[serde(default)]
    pub approval: Approval,
    /// Commit the result on a `kor/<name>-<id>` branch, push it and open a pull request.
    #[serde(default)]
    pub pr: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    #[default]
    Notify,
    Auto,
    Deny,
}

/// `[daemon]`: the webhook listener of `kor daemon`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    /// Listen address; loopback by default. A non-loopback address needs `allow_remote`.
    pub listen: String,
    /// Allow listening beyond this computer (put a TLS proxy in front of it).
    pub allow_remote: bool,
    /// Address the approve links in Slack / e-mail point to (empty = no links, text only).
    pub public_url: String,
}

impl Default for Daemon {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:7431".into(),
            allow_remote: false,
            public_url: String::new(),
        }
    }
}

/// `[notify]`: where trigger results and approval requests go. Secrets come from the vault:
/// `KOR_TELEGRAM_TOKEN`, `KOR_SLACK_WEBHOOK`, `KOR_SMTP_PASSWORD`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // independent on/off channels (ADR 0051)
pub struct Notify {
    /// Telegram chat id that receives messages and may answer approvals.
    pub telegram_chat: String,
    /// Send to the Slack incoming webhook in the vault.
    pub slack: bool,
    pub email_to: String,
    pub email_from: String,
    pub smtp_host: String,
    /// 465 = TLS, 587 = STARTTLS.
    pub smtp_port: u16,
    pub smtp_user: String,
    /// Minutes an approval waits for an answer before it counts as rejected.
    pub approval_timeout_mins: u64,
    /// "Call me" (§28.4, ADR 0051): OS toast notifications from the desktop.
    pub toast: bool,
    /// Terminal `BEL` and window title when the CLI/TUI waits for an answer.
    pub bell: bool,
    /// Taskbar flash / Dock bounce when the desktop waits for an answer.
    pub flash: bool,
    /// Short notification sound (desktop, CLI and TUI; `assets/sounds`).
    pub sound: bool,
    /// Full notification only after the user has been idle this long (seconds); a focused,
    /// active window gets a quiet inline highlight. 0 = always notify.
    pub idle_secs: u64,
    /// `HH-HH` local hours without sound, bell or flash (`22-08`); empty = none.
    pub quiet_hours: String,
    /// Muted categories: `approval`, `question`, `error`, `done`, `budget`, `update`.
    pub muted: Vec<String>,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            telegram_chat: String::new(),
            slack: false,
            email_to: String::new(),
            email_from: String::new(),
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_user: String::new(),
            approval_timeout_mins: 60,
            toast: true,
            bell: true,
            flash: true,
            sound: true,
            idle_secs: 30,
            quiet_hours: String::new(),
            muted: Vec::new(),
        }
    }
}

/// `[forge.hosts] "git.example.com" = "gitlab"`: self-hosted forges (`github`, `gitlab`, `gitea`
/// (also Forgejo), `bitbucket`). github.com, gitlab.com, codeberg.org and bitbucket.org are known.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Forge {
    pub hosts: BTreeMap<String, String>,
}

/// `[vault]`: where `kor vault` keeps secrets and which commands receive them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vault {
    /// `keychain` (OS credential store) or `file` (encrypted local file, opt-in, for machines
    /// without a keychain; passphrase from `KOR_VAULT_PASSPHRASE` or a prompt).
    pub backend: VaultBackend,
    /// `[vault.scope] DATABASE_URL = ["psql", "cargo"]`: only commands that run one of these
    /// programs get the secret; `[]` = no command. Secrets not listed go to every command.
    pub scope: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultBackend {
    #[default]
    Keychain,
    File,
}

/// `[offline] enabled = true` (also `kor --offline`, `KOR_OFFLINE=1`): privacy mode plus no
/// connection beyond this computer (loopback only). ADR 0035.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Offline {
    pub enabled: bool,
    /// Local runtime chosen by `kor offline init` (a local provider id such as `local-llamacpp`,
    /// `local` for Ollama, `local-lmstudio`). ADR 0034.
    pub runtime: String,
    /// Base URL of that runtime (`http://127.0.0.1:9931/v1`).
    pub endpoint: String,
    /// Model used in offline mode when none is given (`local-llamacpp/qwen3-4b`).
    pub model: String,
}

/// `[team] server = "https://kor.example.com"`: the user's own team server; empty = not
/// connected. The access token lives in the OS keychain, never here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Team {
    pub server: String,
}

/// Context Budget Manager (§5.3), `[context]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextBudget {
    /// Warn (and offer `/compact`) when one category passes this percent of the window; 0 = never.
    pub warn_percent: u8,
    /// Summarize the conversation automatically past this percent of the window; 0 = only the
    /// built-in auto-compact near the end of the window.
    pub auto_summarize_percent: u8,
    /// `provider/model` that writes summaries (a cheap or local one); empty = the chat model.
    pub summarize_model: String,
    /// Token budget of the repository signature map in the system prompt; 0 = no map.
    pub repo_map_tokens: u32,
    /// Keep a local log of every request's breakdown (`kor context`, the desktop chart).
    pub log: bool,
    /// §28.1: before an automatic summary, list what will be summarized and let the user keep
    /// items or cancel; `false` only shows the list.
    pub compact_confirm: bool,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            warn_percent: 40,
            auto_summarize_percent: 0,
            summarize_model: String::new(),
            repo_map_tokens: 4000,
            log: true,
            compact_confirm: true,
        }
    }
}

/// `[memory]`: knowledge base recall at task start, "what we learned" at task end, git notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // independent switches of the `[memory]` table
pub struct Memory {
    /// Put relevant knowledge base records into the prompt at task start.
    pub inject: bool,
    /// Token budget of those records; 0 = none.
    pub max_tokens: u32,
    /// Write a short record of each finished task to the knowledge base.
    pub learn: bool,
    /// Allow `kor memory notes push|pull` (decisions as `git notes --ref=kor`).
    pub git_notes: bool,
    /// §27.15: keep the knowledge base as browsable Markdown in `folder` (one way).
    pub folder_view: bool,
    /// The folder of the view; empty = `~/Kor/Bilgi`.
    pub folder: String,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            inject: true,
            max_tokens: 1500,
            learn: true,
            git_notes: false,
            folder_view: false,
            folder: String::new(),
        }
    }
}

/// `[graph]`: graph tools for the agent (§7.5). `strict` blocks the session's first raw file read
/// once and points the model at the graph tools; `watch` keeps the graph updated while a session
/// is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphConfig {
    pub enabled: bool,
    pub strict: bool,
    pub watch: bool,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strict: false,
            watch: true,
        }
    }
}

/// `on`: commands run in the OS sandbox; without a backend Kor warns and asks before each one.
/// `ask`: no sandbox, every command asks. `off`: no sandbox, the permission mode decides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    #[default]
    On,
    Off,
    Ask,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Ask => "ask",
        }
    }
}

/// `[sandbox]` (§6.4, ADR 0033). `kor config set sandbox ask` keeps writing the short form
/// `sandbox = "ask"`, which loads as `[sandbox] mode = "ask"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sandbox {
    pub mode: SandboxMode,
    /// `auto`: the OS's own backend, else Docker when installed; `native`; `docker`.
    pub backend: SandboxBackend,
    /// What confined commands may reach, as `host:port` (`*` = everything); empty = no network.
    /// Host names (`registry.npmjs.org:443`, `*.github.com:443`) go through Kor's filtering proxy.
    pub network: Vec<String>,
    /// Confined commands write to a throwaway layer over the project; the project never changes.
    pub overlay: bool,
    /// Memory cap in MiB for a command; 0 = none.
    pub memory_mb: u32,
    /// CPU cap in percent of the machine; 0 or 100 = none.
    pub cpu_percent: u8,
    /// Docker image for the Docker backend; Kor never pulls it on its own (`kor sandbox pull`).
    pub image: String,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self {
            // Off by default since 11 Nov 2026 (user decision): turn it on in Settings or with
            // `kor config set sandbox.mode on`.
            mode: SandboxMode::Off,
            backend: SandboxBackend::Auto,
            network: Vec::new(),
            overlay: false,
            memory_mb: 8 * 1024,
            cpu_percent: 90,
            image: "alpine:3.20".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    #[default]
    Auto,
    Native,
    Docker,
}

/// `sandbox = "on"` (the short form) is `[sandbox] mode = "on"`, so layers merge field by field.
fn normalize(table: &mut Table) {
    if let Some(Value::String(mode)) = table.get("sandbox") {
        let mut t = Table::new();
        t.insert("mode".into(), Value::String(mode.clone()));
        table.insert("sandbox".into(), Value::Table(t));
    }
}

/// Provider failover (§4.5): `[routing] fallback = ["openrouter/openai/gpt-4o-mini"]` — models
/// tried in order when every key of the chosen provider hits 429, 5xx or a timeout.
///
/// Smart cost router (§5.1): with `enabled = true` each task is classified and sent to the
/// model set for its category (`simple_edit = "groq/llama-3.3-70b"`), else to the cheapest
/// known model good enough for it. Empty category keys mean "decide from the catalog".
/// `KOR_ROUTING__ENABLED=true|false` forces it for one run (e.g. `kor bench`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Routing {
    pub fallback: Vec<String>,
    pub enabled: bool,
    pub simple_edit: String,
    pub complex_refactor: String,
    pub test_generation: String,
    pub docs: String,
    pub review: String,
    pub planning: String,
    /// Draft-then-verify: the routed model drafts, this model reviews the result ("" = off).
    pub verify_with: String,
    /// Consensus mode: the models `kor consensus` asks in parallel (2–3 cheap ones).
    pub consensus: Vec<String>,
    /// Shadow mode: a cheap model that answers every finished task in the background ("" = off).
    pub shadow: String,
    /// Average agreement (percent) at which switching to the shadow model is suggested.
    pub shadow_threshold: u8,
    /// Tasks compared before any suggestion.
    pub shadow_min_samples: u32,
    /// A cheap model that repairs malformed tool calls ("" = the session's own model).
    pub repair: String,
}

/// Spending limits (§5.1). A limit of 0 is no limit. Day and month follow local time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budget {
    pub daily_limit_usd: f64,
    pub monthly_limit_usd: f64,
    /// Share of a limit at which the `on-budget-alert` hook fires once.
    pub alert_at_percent: u8,
    pub on_limit: OnLimit,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            daily_limit_usd: 0.0,
            monthly_limit_usd: 0.0,
            alert_at_percent: 80,
            on_limit: OnLimit::Ask,
        }
    }
}

/// What happens once a limit is reached: switch to the cheapest known model, stop, or ask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnLimit {
    Downgrade,
    Pause,
    #[default]
    Ask,
}

/// Self-update (`kor update`): channel and whether a found release is installed automatically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Update {
    /// Install a verified newer release on its own; `false` only notifies.
    pub auto: bool,
    pub channel: UpdateChannel,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            auto: true,
            channel: UpdateChannel::Stable,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
    Nightly,
}

/// Desktop control (screen, mouse, keyboard, UI automation). On by default since 11 Nov 2026
/// (user decision, ADR 0059); the per-app tiers and the human-first stop still gate every step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Computer {
    pub enabled: bool,
    /// Per-app tier (§19.1, ADR 0059): `computer.apps.chrome = "full"`. App names as the OS
    /// reports the front process, lower case, without `.exe`. Unlisted apps use built-in defaults
    /// (terminals, password managers, banking: `blocked`; browsers: `read`; IDEs: `click`; rest: `ask`).
    pub apps: std::collections::BTreeMap<String, ComputerTier>,
}

impl Default for Computer {
    fn default() -> Self {
        Self {
            enabled: true,
            apps: std::collections::BTreeMap::new(),
        }
    }
}

/// What desktop control may do in one app (§19.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerTier {
    /// Nothing: no screenshot, input or window action while it is in front.
    Blocked,
    /// Screenshots and the accessibility tree only.
    Read,
    /// Also left clicks, scrolling and focusing its windows; no typing or keys.
    Click,
    /// Everything, each step approved.
    #[default]
    Ask,
    /// Everything without asking (destructive key combinations still ask).
    Full,
}

impl ComputerTier {
    pub const ALL: [Self; 5] = [
        Self::Blocked,
        Self::Read,
        Self::Click,
        Self::Ask,
        Self::Full,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Read => "read",
            Self::Click => "click",
            Self::Ask => "ask",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == s)
    }
}

/// `[health]` (§19.2, §19.3, ADR 0059): system profile refresh and health alert thresholds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Health {
    /// Health alerts on (the system profile is always kept).
    pub enabled: bool,
    /// Minutes between system profile refreshes.
    pub refresh_minutes: u32,
    /// Disk fill warning / critical (%); at critical Kor stops writing tasks before they start.
    pub disk_warn: u8,
    pub disk_critical: u8,
    /// RAM in use (%) and swap in use (%).
    pub memory_warn: u8,
    pub swap_warn: u8,
    /// Kor and its child processes: CPU (% of the whole machine) and memory (MB).
    pub kor_cpu_warn: u8,
    pub kor_memory_mb: u32,
    /// A child process above this CPU (% of one core) for `runaway_minutes` is reported.
    pub runaway_cpu: u16,
    pub runaway_minutes: u32,
    /// CPU temperature (°C).
    pub temp_warn: u8,
    /// Battery (%) when not charging.
    pub battery_critical: u8,
    /// One build cache (`target/`, `node_modules`, …) above this size (GB).
    pub cache_warn_gb: u32,
    /// Docker images, containers and build cache together (GB).
    pub docker_warn_gb: u32,
    /// Kor's own cache and index folders (GB).
    pub kor_cache_warn_gb: u32,
    /// Alert kinds turned off: `disk`, `memory`, `swap`, `kor`, `runaway`, `temperature`,
    /// `battery`, `cache`, `docker`, `kor_cache`.
    pub muted: Vec<String>,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            enabled: true,
            refresh_minutes: 5,
            disk_warn: 85,
            disk_critical: 95,
            memory_warn: 90,
            swap_warn: 80,
            kor_cpu_warn: 80,
            kor_memory_mb: 4096,
            runaway_cpu: 90,
            runaway_minutes: 10,
            temp_warn: 90,
            battery_critical: 10,
            cache_warn_gb: 5,
            docker_warn_gb: 20,
            kor_cache_warn_gb: 2,
            muted: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // readability switches are independent config keys
pub struct Ui {
    /// Interface language; `auto` follows the OS locale.
    pub language: LangPref,
    /// CLI/TUI spinner frames from the theme (§16.1): `ember`, `braille`, `flame`, `cube`, `spark`, `forge`.
    pub spinner: String,
    /// `kor` without arguments opens the full-screen TUI in a terminal; `false` keeps the line REPL.
    pub default_tui: bool,
    /// TUI key bindings: `default`, `vim` or `emacs`.
    pub tui_keys: TuiKeys,
    /// Readability (§28.9), shared by the desktop chat and the terminal.
    /// Wrap long lines (default on); off lets the terminal / chat scroll horizontally.
    pub wrap: bool,
    /// Terminal wrap column; 0 = the terminal width.
    pub wrap_width: u16,
    /// Chat line width: `narrow`, `medium`, `wide` or `full`.
    pub max_width: LineWidth,
    pub density: Density,
    /// Chat font size in px (desktop), 12–24.
    pub font_size: u8,
    /// Line height ×100 (150 = 1.5).
    pub line_height: u16,
    /// Code block font family (desktop); empty = the theme's monospace font.
    pub code_font: String,
    /// Diff view: `inline` or `split` (side by side).
    pub diff_view: DiffView,
    /// Tool output detail: `quiet`, `normal` or `verbose`; `/verbosity` changes it mid-task.
    pub tool_output: Verbosity,
    /// Accessibility: screen reader mode (no spinners/animations, plain text announcements).
    pub screen_reader: bool,
    pub high_contrast: bool,
    pub reduced_motion: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            language: LangPref::default(),
            spinner: "ember".into(),
            default_tui: true,
            tui_keys: TuiKeys::Default,
            wrap: true,
            wrap_width: 0,
            max_width: LineWidth::Medium,
            density: Density::Comfortable,
            font_size: 14,
            line_height: 150,
            code_font: String::new(),
            diff_view: DiffView::Inline,
            tool_output: Verbosity::Normal,
            screen_reader: false,
            high_contrast: false,
            reduced_motion: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineWidth {
    Narrow,
    #[default]
    Medium,
    Wide,
    Full,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffView {
    #[default]
    Inline,
    Split,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TuiKeys {
    #[default]
    Default,
    #[serde(alias = "vi")]
    Vim,
    Emacs,
}

/// §28.13 when the agent asks instead of proceeding: `never` (best guess, assumptions listed),
/// `critical` (irreversible, expensive or several equally good ways) or `often`.
/// `low|medium|high` are accepted as aliases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskThreshold {
    #[serde(alias = "low")]
    Never,
    #[default]
    #[serde(alias = "medium")]
    Critical,
    #[serde(alias = "high")]
    Often,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    /// Language the model is asked to reply in; `auto` follows the user.
    pub reply_language: LangPref,
    /// Default model as `provider/model`; empty means "ask on the command line".
    pub model: String,
    /// Model round-trips allowed for one user message.
    pub max_turns: u32,
    /// Output token cap per model call; 0 lets the adapter decide.
    pub max_tokens: u32,
    /// How tools reach the model: `auto` (native when the provider supports it, text
    /// otherwise, switching to text if native tools are rejected), `native`, or `text`.
    pub tool_mode: ToolCalling,
    /// Verified edit loop: `auto` checks changed files with the project's own checker
    /// (`cargo check`, `tsc`, `go vet`, `py_compile`, `node --check`); `off` disables it.
    pub verify: VerifyMode,
    /// A `Write` that would replace an existing file longer than this is refused with a hint
    /// to use `Edit`; 0 turns the check off.
    pub write_max_lines: u32,
    /// Reasoning effort for the agent's model calls: `none`, `minimal`, `low`, `medium`, `high`,
    /// `xhigh` or `max`; unset leaves the provider's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<kor_core::ReasoningEffort>,
    /// When the agent asks a clarifying question (§28.13): `never`, `critical` or `often`.
    pub ask_threshold: AskThreshold,
    /// Check every proposed edit against the loaded rules before it is shown (§28.3); a broken
    /// rule sends the edit back to the model up to twice, then asks.
    pub rules_check: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyMode {
    #[default]
    Auto,
    Off,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCalling {
    #[default]
    Auto,
    Native,
    Text,
}

/// The model a fresh install starts with: the default layer of [`load`] (`agent.model`, origin
/// "default"), so the CLI, TUI and desktop share it and `kor config explain` shows where it came from.
pub const DEFAULT_MODEL: &str = "venice/deepseek-v4-1-flash-uncensored";

impl Default for Agent {
    fn default() -> Self {
        Self {
            reply_language: LangPref::Auto,
            // Empty here; `load` puts [`DEFAULT_MODEL`] into its default layer. A `Config::default()`
            // built in code (tests, embedders) never reaches a paid provider on its own.
            model: String::new(),
            max_turns: 50,
            max_tokens: 0,
            tool_mode: ToolCalling::Auto,
            verify: VerifyMode::Auto,
            write_max_lines: 400,
            reasoning: None,
            ask_threshold: AskThreshold::Critical,
            rules_check: true,
        }
    }
}

/// Permission rules, e.g. `Bash(npm run *)`, `Edit(src/**)`, `Read`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Permissions {
    pub mode: PermissionMode,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Privacy {
    /// Providers below this tier are never selected.
    pub min_tier: PrivacyTier,
    /// Privacy mode (`kor --privacy-mode`): local providers only; web, MCP, catalog refresh and
    /// every other outbound call of Kor are off.
    pub mode: bool,
    /// §31.2 policy filter (`kor config set privacy.require no-training,eu-only`): models whose
    /// privacy card does not meet every entry are dimmed in the picker and skipped by the router.
    /// A comma-separated string or a list.
    #[serde(deserialize_with = "requirements")]
    pub require: Vec<PrivacyRequirement>,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            min_tier: PrivacyTier::Unknown,
            mode: false,
            require: Vec::new(),
        }
    }
}

fn requirements<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<PrivacyRequirement>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        One(String),
        Many(Vec<String>),
    }
    let joined = match Raw::deserialize(d)? {
        Raw::One(s) => s,
        Raw::Many(v) => v.join(","),
    };
    PrivacyRequirement::parse_list(&joined).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Default,
    System,
    User,
    Project,
    Env,
    Cli,
}

/// One effective setting and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub value: Value,
    pub layer: Layer,
    /// File path, env var name, `default`, or `cli:<key>`.
    pub origin: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{origin}: {source}")]
    Io {
        origin: String,
        source: std::io::Error,
    },
    #[error("{origin}: file exceeds {MAX_FILE_BYTES} bytes")]
    TooLarge { origin: String },
    #[error("{origin}: {message}")]
    Invalid { origin: String, message: String },
}

impl ConfigError {
    pub fn origin(&self) -> &str {
        match self {
            Self::Io { origin, .. } | Self::TooLarge { origin } | Self::Invalid { origin, .. } => {
                origin
            }
        }
    }

    /// What is wrong, without the file (for messages that name [`Self::origin`] themselves).
    pub fn message(&self) -> String {
        match self {
            Self::Io { source, .. } => source.to_string(),
            Self::TooLarge { .. } => format!("file exceeds {MAX_FILE_BYTES} bytes"),
            Self::Invalid { message, .. } => message.clone(),
        }
    }
}

/// Candidate config file locations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    pub system: Option<PathBuf>,
    pub user: Option<PathBuf>,
    /// Nearest existing `.kor/config.toml` at or above the working directory.
    pub project: Option<PathBuf>,
}

impl Paths {
    pub fn discover(cwd: &Path) -> Self {
        Self {
            system: system_dir().map(|d| d.join("config.toml")),
            user: user_dir().map(|d| d.join("config.toml")),
            project: cwd
                .ancestors()
                .map(|d| d.join(".kor").join("config.toml"))
                .find(|p| p.is_file()),
        }
    }
}

/// `$XDG_CONFIG_HOME/kor`, `~/Library/Application Support/kor`, `%APPDATA%\kor`.
pub fn user_dir() -> Option<PathBuf> {
    // `KOR_CONFIG_DIR`: an isolated configuration (E2E scenarios, ADR 0061); Windows resolves the
    // known folders without looking at %APPDATA%, so an env var is the only portable override.
    if let Some(d) = std::env::var_os("KOR_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::config_dir().map(|d| d.join("kor"))
}

/// Machine-wide config directory.
pub fn system_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("ProgramData").map(|d| PathBuf::from(d).join("kor"))
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from("/Library/Application Support/kor"))
    } else {
        Some(PathBuf::from("/etc/kor"))
    }
}

#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    /// Dotted key → effective setting, sorted.
    pub settings: BTreeMap<String, Setting>,
    /// Config files that existed and were read.
    pub files: Vec<PathBuf>,
}

/// Load and merge all layers.
///
/// `env` is usually `std::env::vars()`; only `KOR_LANG` and `KOR_<SECTION>__<KEY>`
/// are read. `cli` holds dotted-key overrides from command-line flags.
pub fn load(
    paths: &Paths,
    env: impl IntoIterator<Item = (String, String)>,
    cli: &[(&str, Value)],
) -> Result<Loaded, ConfigError> {
    let mut defaults = Value::try_from(Config::default())
        .ok()
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();
    set_dotted(
        &mut defaults,
        "agent.model",
        Value::String(DEFAULT_MODEL.to_owned()),
    );

    let mut layers: Vec<(Layer, String, Table)> =
        vec![(Layer::Default, "default".into(), defaults)];
    let mut files = Vec::new();

    for (layer, path) in [
        (Layer::System, &paths.system),
        (Layer::User, &paths.user),
        (Layer::Project, &paths.project),
    ] {
        let Some(path) = path else { continue };
        if let Some(table) = read_file(path)? {
            files.push(path.clone());
            layers.push((layer, path.display().to_string(), table));
        }
    }

    let mut env: Vec<_> = env.into_iter().collect();
    env.sort();
    for (name, raw) in env {
        let mut t = Table::new();
        // `KOR_OFFLINE=1` (ADR 0035): a switch, not a `KOR_<SECTION>__<KEY>` value.
        if name == "KOR_OFFLINE" {
            let on = matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
            set_dotted(&mut t, "offline.enabled", Value::Boolean(on));
            layers.push((Layer::Env, name, t));
            continue;
        }
        let Some(key) = env_key(&name) else { continue };
        set_dotted(&mut t, &key, scalar(&raw));
        layers.push((Layer::Env, name, t));
    }

    for (key, value) in cli {
        let mut t = Table::new();
        set_dotted(&mut t, key, value.clone());
        layers.push((Layer::Cli, format!("cli:{key}"), t));
    }

    let mut merged = Table::new();
    let mut settings = BTreeMap::new();
    for (layer, origin, mut table) in layers {
        normalize(&mut table);
        // Validate each layer alone so errors name the file or variable at fault.
        Config::deserialize(Value::Table(table.clone())).map_err(|e| ConfigError::Invalid {
            origin: origin.clone(),
            message: e.to_string(),
        })?;
        flatten("", &table, &mut |k, v| {
            settings.insert(
                k,
                Setting {
                    value: v.clone(),
                    layer,
                    origin: origin.clone(),
                },
            );
        });
        merge(&mut merged, table);
    }

    let mut config =
        Config::deserialize(Value::Table(merged)).map_err(|e| ConfigError::Invalid {
            origin: "merged".into(),
            message: e.to_string(),
        })?;
    // Offline is privacy mode plus loopback only: every `privacy.mode` gate applies (ADR 0035).
    if config.offline.enabled {
        config.privacy.mode = true;
    }
    config.apply_auto();
    Ok(Loaded {
        config,
        settings,
        files,
    })
}

fn read_file(path: &Path) -> Result<Option<Table>, ConfigError> {
    let origin = || path.display().to_string();
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Io {
                origin: origin(),
                source,
            });
        }
    };
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|source| ConfigError::Io {
            origin: origin(),
            source,
        })?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(ConfigError::TooLarge { origin: origin() });
    }
    text.parse::<Table>()
        .map(Some)
        .map_err(|e| ConfigError::Invalid {
            origin: origin(),
            message: e.to_string(),
        })
}

/// `KOR_LANG` → `ui.language`; `KOR_PRIVACY__MIN_TIER` → `privacy.min_tier`.
fn env_key(name: &str) -> Option<String> {
    if name == "KOR_LANG" {
        return Some("ui.language".into());
    }
    let rest = name.strip_prefix(ENV_PREFIX)?;
    let (section, key) = rest.split_once("__")?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (ok(section) && ok(key)).then(|| format!("{}.{}", section.to_lowercase(), key.to_lowercase()))
}

/// Interpret an env value as a TOML scalar (`true`, `3`), else as a plain string.
fn scalar(raw: &str) -> Value {
    format!("v = {raw}")
        .parse::<Table>()
        .ok()
        .filter(|t| t.len() == 1)
        .and_then(|mut t| t.remove("v"))
        .filter(|v| !v.is_table())
        .unwrap_or_else(|| Value::String(raw.to_owned()))
}

fn set_dotted(table: &mut Table, key: &str, value: Value) {
    match key.split_once('.') {
        None => {
            table.insert(key.to_owned(), value);
        }
        Some((head, rest)) => {
            let entry = table
                .entry(head.to_owned())
                .or_insert_with(|| Value::Table(Table::new()));
            if !entry.is_table() {
                *entry = Value::Table(Table::new());
            }
            if let Value::Table(t) = entry {
                set_dotted(t, rest, value);
            }
        }
    }
}

fn merge(base: &mut Table, over: Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(Value::Table(b)), Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

fn flatten(prefix: &str, table: &Table, f: &mut impl FnMut(String, &Value)) {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Table(t) => flatten(&key, t, f),
            v => f(key, v),
        }
    }
}

/// Set one dotted key (for example `ui.language`) in `file` to a string, keeping comments and
/// every other setting. A missing file is created.
///
/// The edited file is checked with [`load`] before it replaces the original, so an invalid value
/// never reaches disk; the replacement is a rename, so a crash cannot leave half a file.
pub fn set_string(file: &Path, key: &str, value: &str) -> Result<(), ConfigError> {
    set_item(file, key, toml_edit::value(value))
}

/// Set a key from command-line text: `true`/`3` become TOML booleans/numbers, anything else a
/// string (`kor config set computer.enabled true`). Validated like [`set_string`].
///
/// # Errors
/// As [`set_string`].
pub fn set_scalar(file: &Path, key: &str, raw: &str) -> Result<(), ConfigError> {
    // A TOML array literal (`'["*"]'`, `'["registry.npmjs.org:443"]'`) is written as a list.
    if raw.trim_start().starts_with('[')
        && let Ok(v @ toml_edit::Value::Array(_)) = raw.parse::<toml_edit::Value>()
    {
        return set_item(file, key, toml_edit::Item::Value(v));
    }
    let item = match scalar(raw) {
        Value::Boolean(b) => toml_edit::value(b),
        Value::Integer(i) => toml_edit::value(i),
        Value::Float(f) => toml_edit::value(f),
        _ => toml_edit::value(raw),
    };
    set_item(file, key, item)
}

/// [`set_string`] for a list of strings (e.g. `permissions.allow`).
///
/// # Errors
/// As [`set_string`].
pub fn set_string_list(file: &Path, key: &str, values: &[String]) -> Result<(), ConfigError> {
    let mut array = toml_edit::Array::new();
    for v in values {
        array.push(v.as_str());
    }
    set_item(file, key, toml_edit::value(array))
}

/// Remove one dotted key (`computer.apps.chrome`) from `file`, keeping everything else; a missing
/// file or key is not an error. Validated and replaced like [`set_string`] (ADR 0059).
///
/// # Errors
/// As [`set_string`].
pub fn unset(file: &Path, key: &str) -> Result<(), ConfigError> {
    let origin = file.display().to_string();
    let io = |source: std::io::Error| ConfigError::Io {
        origin: origin.clone(),
        source,
    };
    let text = match fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(e)),
    };
    let mut doc: toml_edit::DocumentMut =
        text.parse()
            .map_err(|e: toml_edit::TomlError| ConfigError::Invalid {
                origin: origin.clone(),
                message: e.to_string(),
            })?;
    let mut parts: Vec<&str> = key.split('.').collect();
    let Some(leaf) = parts.pop() else {
        return Ok(());
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parts {
        match table
            .get_mut(part)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            Some(t) => table = t,
            None => return Ok(()),
        }
    }
    if table.remove(leaf).is_none() {
        return Ok(());
    }
    let tmp = file.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string()).map_err(io)?;
    let check = Paths {
        system: None,
        user: Some(tmp.clone()),
        project: None,
    };
    if let Err(e) = load(&check, std::iter::empty(), &[]) {
        let _ = fs::remove_file(&tmp);
        return Err(ConfigError::Invalid {
            origin,
            message: e.message(),
        });
    }
    fs::rename(&tmp, file).map_err(io)
}

fn set_item(file: &Path, key: &str, item: toml_edit::Item) -> Result<(), ConfigError> {
    let origin = file.display().to_string();
    let invalid = |message: String| ConfigError::Invalid {
        origin: origin.clone(),
        message,
    };
    let io = |source: std::io::Error| ConfigError::Io {
        origin: origin.clone(),
        source,
    };
    let text = match fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| invalid(e.to_string()))?;
    // `sandbox` is both the short `sandbox = "on"` and the `[sandbox]` table (ADR 0033).
    let key = if key == "sandbox"
        && doc
            .get("sandbox")
            .is_some_and(toml_edit::Item::is_table_like)
    {
        "sandbox.mode"
    } else {
        key
    };
    if key.starts_with("sandbox.")
        && let Some(mode) = doc
            .get("sandbox")
            .and_then(toml_edit::Item::as_str)
            .map(str::to_owned)
    {
        let mut t = toml_edit::Table::new();
        t.insert("mode", toml_edit::value(mode));
        doc.insert("sandbox", toml_edit::Item::Table(t));
    }
    let mut parts: Vec<&str> = key.split('.').collect();
    let leaf = parts
        .pop()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| invalid(format!("invalid key: {key}")))?;
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parts {
        table = table
            .entry(part)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| invalid(format!("{part} is not a table")))?;
    }
    table.insert(leaf, item);

    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = file.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string()).map_err(io)?;
    let check = Paths {
        system: None,
        user: Some(tmp.clone()),
        project: None,
    };
    if let Err(e) = load(&check, std::iter::empty(), &[]) {
        let _ = fs::remove_file(&tmp);
        // The check ran on the temporary copy; the message is about the user's file.
        return Err(invalid(e.message()));
    }
    fs::rename(&tmp, file).map_err(io)
}

#[cfg(test)]
mod tests {

    #[test]
    fn browser_backend_accepts_extension_alias() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        assert_eq!(Config::default().browser.backend, BrowserBackend::Auto);
        set_scalar(&f, "browser.backend", "extension").unwrap();
        let paths = Paths {
            system: None,
            user: Some(f.clone()),
            project: None,
        };
        assert_eq!(
            load(&paths, [], &[]).unwrap().config.browser.backend,
            BrowserBackend::Chrome
        );
        set_scalar(&f, "browser.backend", "headless").unwrap();
        let paths = Paths {
            system: None,
            user: Some(f.clone()),
            project: None,
        };
        assert_eq!(
            load(&paths, [], &[]).unwrap().config.browser.backend,
            BrowserBackend::Headless
        );
        assert!(set_scalar(&f, "browser.backend", "firefox").is_err());
    }

    #[test]
    fn auto_keys_ask_threshold_and_readability() {
        let d = Config::default();
        assert!(AUTO_KEYS.iter().all(|(k, _)| d.auto.get(k) == Some(true)));
        assert_eq!(d.auto.get("nope"), None);
        assert!(!d.tui.alt_screen && d.ui.wrap && d.context.compact_confirm);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        set_scalar(&f, "auto.compact", "false").unwrap();
        set_scalar(&f, "agent.ask_threshold", "high").unwrap();
        set_scalar(&f, "ui.max_width", "wide").unwrap();
        set_scalar(&f, "keys.mode", "vi").unwrap();
        assert!(set_scalar(&f, "auto.nope", "false").is_err());
        assert!(set_scalar(&f, "agent.ask_threshold", "sometimes").is_err());
        let paths = Paths {
            system: None,
            user: Some(f),
            project: None,
        };
        let env = [("KOR_AUTO__ROUTING".to_owned(), "false".to_owned())];
        let c = load(&paths, env, &[]).unwrap();
        assert!(!c.config.auto.compact && !c.config.auto.routing);
        let mut folded = Config::default();
        folded.routing.enabled = true;
        folded.auto.routing = false;
        folded.auto.verify = false;
        folded.auto.memory_inject = false;
        folded.apply_auto();
        assert!(!folded.routing.enabled && !folded.memory.inject && folded.memory.learn);
        assert_eq!(folded.agent.verify, VerifyMode::Off);
        assert_eq!(c.config.agent.ask_threshold, AskThreshold::Often);
        assert_eq!(c.config.ui.max_width, LineWidth::Wide);
        assert_eq!(c.config.key_mode(), TuiKeys::Vim);
        assert_eq!(c.settings["auto.routing"].layer, Layer::Env);
    }

    #[test]
    fn set_scalar_types_values_and_computer_is_on_by_default() {
        assert!(Config::default().computer.enabled);
        // The default model lives in `load`'s default layer only: code-built configs (tests) have none.
        assert!(Config::default().agent.model.is_empty());
        let none = Paths {
            system: None,
            user: None,
            project: None,
        };
        let fresh = load(&none, std::iter::empty(), &[]).unwrap();
        assert_eq!(fresh.config.agent.model, DEFAULT_MODEL);
        assert_eq!(
            fresh.settings.get("agent.model").map(|s| s.layer),
            Some(Layer::Default)
        );
        let i = Config::default().integrations;
        assert!(
            i.auto && i.context_menu && i.shell && i.os_on("powertoys-run") && !i.os_on("portals")
        );
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        set_scalar(&f, "computer.enabled", "false").unwrap();
        // A removed integration is remembered as `false` (the desktop's "Kaldır").
        set_scalar(&f, "integrations.os.widgets", "false").unwrap();
        set_scalar(&f, "integrations.context_menu", "false").unwrap();
        let off = load(
            &Paths {
                system: None,
                user: Some(f.clone()),
                project: None,
            },
            std::iter::empty(),
            &[],
        )
        .unwrap();
        assert!(!off.config.computer.enabled);
        assert!(!off.config.integrations.os_on("widgets") && !off.config.integrations.context_menu);
        set_scalar(&f, "computer.enabled", "true").unwrap();
        set_scalar(&f, "agent.max_turns", "12").unwrap();
        let loaded = load(
            &Paths {
                system: None,
                user: Some(f.clone()),
                project: None,
            },
            std::iter::empty(),
            &[],
        )
        .unwrap();
        assert!(loaded.config.computer.enabled);
        assert_eq!(loaded.config.agent.max_turns, 12);
        assert!(set_scalar(&f, "computer.nope", "true").is_err());
        assert!(set_scalar(&f, "agent.max_turns", "many").is_err());
        assert_eq!(Config::default().agent.reasoning, None);
        set_scalar(&f, "agent.reasoning", "xhigh").unwrap();
        assert!(set_scalar(&f, "agent.reasoning", "huge").is_err());
        // A TOML array literal becomes a list (`kor config set sandbox.network '["*"]'`).
        set_scalar(&f, "sandbox.network", r#"["*", "registry.npmjs.org:443"]"#).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(
            text.contains(r#"network = ["*", "registry.npmjs.org:443"]"#),
            "{text}"
        );

        // `kor config set sandbox ask`: a top-level key after tables exist.
        assert_eq!(Config::default().sandbox.mode, SandboxMode::Off);
        set_scalar(&f, "sandbox", "ask").unwrap();
        let paths = Paths {
            system: None,
            user: Some(f.clone()),
            project: None,
        };
        let loaded = load(&paths, std::iter::empty(), &[]).unwrap();
        assert_eq!(loaded.config.sandbox.mode, SandboxMode::Ask);
        assert!(loaded.config.computer.enabled);
        assert!(set_scalar(&f, "sandbox", "maybe").is_err());
    }

    #[test]
    fn sandbox_short_form_and_table_merge_across_layers() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        let project = dir.path().join("project.toml");
        std::fs::write(&user, "sandbox = \"ask\"\n").unwrap();
        std::fs::write(
            &project,
            "[sandbox]\nnetwork = [\"github.com:443\"]\nmemory_mb = 512\n",
        )
        .unwrap();
        let paths = Paths {
            system: None,
            user: Some(user.clone()),
            project: Some(project.clone()),
        };
        let s = load(&paths, std::iter::empty(), &[])
            .unwrap()
            .config
            .sandbox;
        assert_eq!(
            s.mode,
            SandboxMode::Ask,
            "the short form survives a table above it"
        );
        assert_eq!(
            (s.network, s.memory_mb),
            (vec!["github.com:443".to_owned()], 512)
        );
        assert_eq!(s.cpu_percent, 90);

        // Setting a field turns the short form into a table; `sandbox` then sets `mode`.
        set_scalar(&user, "sandbox.cpu_percent", "50").unwrap();
        set_scalar(&user, "sandbox", "off").unwrap();
        let text = std::fs::read_to_string(&user).unwrap();
        assert!(
            text.contains("[sandbox]") && text.contains("mode = \"off\""),
            "{text}"
        );
        let s = load(
            &Paths {
                system: None,
                user: Some(user),
                project: None,
            },
            std::iter::empty(),
            &[],
        )
        .unwrap()
        .config
        .sandbox;
        assert_eq!((s.mode, s.cpu_percent), (SandboxMode::Off, 50));
        let env = [("KOR_SANDBOX__BACKEND".to_owned(), "docker".to_owned())];
        let s = load(&Paths::default(), env, &[]).unwrap().config.sandbox;
        assert_eq!(s.backend, SandboxBackend::Docker);
    }

    #[test]
    fn set_string_keeps_comments_and_rejects_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        std::fs::write(&f, "# benim notum\n[agent]\nmax_turns = 7\n").unwrap();
        set_string(&f, "ui.language", "fr").unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("# benim notum"));
        assert!(text.contains("max_turns = 7"));
        assert!(text.contains(r#"language = "fr""#));
        assert!(set_string(&f, "ui.language", "klingon").is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), text);
        assert!(!dir.path().join("config.toml.tmp").exists());
    }
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
        p
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn layers_override_in_order_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            system: Some(write(dir.path(), "sys.toml", "[ui]\nlanguage = \"fr\"\n")),
            user: Some(write(
                dir.path(),
                "user.toml",
                "[privacy]\nmin_tier = \"no_train\"\n",
            )),
            project: Some(write(dir.path(), "proj.toml", "[ui]\nlanguage = \"tr\"\n")),
        };

        let l = load(&paths, env(&[]), &[]).unwrap();
        assert_eq!(l.config.ui.language, LangPref::Tr);
        assert_eq!(l.config.privacy.min_tier, PrivacyTier::NoTrain);
        assert_eq!(l.settings["ui.language"].layer, Layer::Project);
        assert_eq!(l.settings["agent.reply_language"].layer, Layer::Default);
        assert_eq!(l.files.len(), 3);

        let l = load(&paths, env(&[("KOR_LANG", "en")]), &[]).unwrap();
        assert_eq!(l.config.ui.language, LangPref::En);
        assert_eq!(l.settings["ui.language"].origin, "KOR_LANG");

        let l = load(
            &paths,
            env(&[("KOR_LANG", "en")]),
            &[("ui.language", Value::String("fr".into()))],
        )
        .unwrap();
        assert_eq!(l.config.ui.language, LangPref::Fr);
        assert_eq!(l.settings["ui.language"].layer, Layer::Cli);
    }

    #[test]
    fn generic_env_mapping_and_unrelated_vars() {
        let l = load(
            &Paths::default(),
            env(&[
                ("KOR_PRIVACY__MIN_TIER", "local"),
                ("KOR_LOG", "debug"),
                ("PATH", "/bin"),
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(l.config.privacy.min_tier, PrivacyTier::Local);
    }

    #[test]
    fn privacy_require_takes_a_comma_string_or_a_list() {
        let l = load(
            &Paths::default(),
            env(&[("KOR_PRIVACY__REQUIRE", "no-training, eu-only")]),
            &[],
        )
        .unwrap();
        assert_eq!(
            l.config.privacy.require,
            vec![PrivacyRequirement::NoTraining, PrivacyRequirement::EuOnly]
        );
        let t: Config = toml::from_str(
            "[privacy]
require = [\"local-only\"]
",
        )
        .unwrap();
        assert_eq!(t.privacy.require, vec![PrivacyRequirement::LocalOnly]);
        assert!(
            toml::from_str::<Config>(
                "[privacy]
require = \"mars-only\"
"
            )
            .is_err()
        );
    }

    #[test]
    fn offline_switch_implies_privacy_mode() {
        let l = load(&Paths::default(), env(&[("KOR_OFFLINE", "1")]), &[]).unwrap();
        assert!(l.config.offline.enabled && l.config.privacy.mode);
        let l = load(&Paths::default(), env(&[("KOR_OFFLINE", "0")]), &[]).unwrap();
        assert!(!l.config.offline.enabled && !l.config.privacy.mode);
        let l = load(
            &Paths::default(),
            env(&[]),
            &[("offline.enabled", Value::Boolean(true))],
        )
        .unwrap();
        assert!(l.config.privacy.mode);
    }

    #[test]
    fn errors_name_their_origin() {
        let dir = tempfile::tempdir().unwrap();
        let bad = write(dir.path(), "u.toml", "[ui]\nlangauge = \"tr\"\n");
        let paths = Paths {
            user: Some(bad.clone()),
            ..Paths::default()
        };
        let err = load(&paths, env(&[]), &[]).unwrap_err();
        assert_eq!(err.origin(), bad.display().to_string());

        let err = load(&Paths::default(), env(&[("KOR_UI__LANGUAGE", "de")]), &[]).unwrap_err();
        assert_eq!(err.origin(), "KOR_UI__LANGUAGE");

        let syntax = write(dir.path(), "s.toml", "[ui\n");
        let err = load(
            &Paths {
                project: Some(syntax),
                ..Paths::default()
            },
            env(&[]),
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let big = write(dir.path(), "big.toml", &"#".repeat(1024 * 1024 + 10));
        let err = load(
            &Paths {
                user: Some(big),
                ..Paths::default()
            },
            env(&[]),
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::TooLarge { .. }));
    }

    #[test]
    fn missing_files_are_skipped_and_project_is_discovered() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".kor/config.toml", "");
        let nested = dir.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        let p = Paths::discover(&nested);
        assert_eq!(p.project, Some(dir.path().join(".kor").join("config.toml")));

        let l = load(
            &Paths {
                user: Some(dir.path().join("nope.toml")),
                ..Paths::default()
            },
            env(&[]),
            &[],
        )
        .unwrap();
        assert!(l.files.is_empty());
    }

    #[test]
    fn routing_and_budget_parse_and_env_forces_routing() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(
            dir.path(),
            "u.toml",
            "[routing]\nsimple_edit = \"groq/llama\"\nfallback = [\"openrouter/auto\"]\n\
             [budget]\ndaily_limit_usd = 5.0\non_limit = \"downgrade\"\n",
        );
        let paths = Paths {
            user: Some(f),
            ..Paths::default()
        };
        let l = load(&paths, env(&[]), &[]).unwrap();
        assert!(!l.config.routing.enabled, "routing is off by default");
        assert_eq!(l.config.routing.simple_edit, "groq/llama");
        assert_eq!(l.config.budget.on_limit, OnLimit::Downgrade);
        assert_eq!(l.config.budget.alert_at_percent, 80);
        let l = load(&paths, env(&[("KOR_ROUTING__ENABLED", "true")]), &[]).unwrap();
        assert!(l.config.routing.enabled);
        assert!(load(&paths, env(&[("KOR_BUDGET__ON_LIMIT", "maybe")]), &[]).is_err());
    }

    #[test]
    fn env_scalars() {
        assert_eq!(scalar("true"), Value::Boolean(true));
        assert_eq!(scalar("tr"), Value::String("tr".into()));
        assert_eq!(scalar("1\nx = 2"), Value::String("1\nx = 2".into()));
        assert_eq!(env_key("KOR_A__B"), Some("a.b".into()));
        assert_eq!(env_key("KOR_A__"), None);
        assert_eq!(env_key("KOR_A.B__C"), None);
    }
}
