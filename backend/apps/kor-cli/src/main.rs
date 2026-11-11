//! `kor` command-line entry point.

mod acp_cmd;
mod audit_cmd;
mod auth;
mod bench_cmd;
mod brand;
mod catalog_cmd;
mod chrome;
mod chrome_cmd;
mod clean_cmd;
mod comment_cmd;
mod commit_cmd;
mod compat;
mod completions_cmd;
mod component_cmd;
mod computer_cmd;
mod consensus_cmd;
mod context_cmd;
mod cost_cmd;
mod daemon_cmd;
mod db_cmd;
mod deadcode_cmd;
mod deploy_cmd;
mod deps_cmd;
mod diagram_cmd;
mod discover_cmd;
mod docs_cmd;
mod docs_web_cmd;
mod doctor;
mod env_cmd;
mod estimate_cmd;
mod explain_cmd;
mod export_cmd;
mod extra;
mod files_cmd;
mod find_cmd;
mod folder_cmd;
mod graph_cmd;
mod hf_cmd;
mod hooks_cmd;
mod install_hooks_cmd;
mod integrate_cmd;
mod intent_cmd;
mod know_cmd;
mod layout_cmd;
mod mcp_cmd;
mod media_cmd;
mod memory_cmd;
mod notify_cmd;
mod oauth;
mod offline_cmd;
mod osint_cmd;
mod packages_cmd;
mod peer_cmd;
mod perf_cmd;
mod plan_cmd;
mod pr_cmd;
mod prefs_cmd;
mod prereq_cmd;
mod privacy_cmd;
mod provider_cmd;
mod providers;
mod proxy_cmd;
mod quality_cmd;
mod remote_cmd;
mod repl;
mod replay_cmd;
mod review_cmd;
mod rules_cmd;
mod sandbox_cmd;
mod screenshot_cmd;
mod seo_cmd;
mod sessions_cmd;
mod shell_cmd;
mod status_cmd;
mod system_cmd;
mod task_cmd;
mod templates_cmd;
mod tool_cmd;
mod triggers_cmd;
mod tui;
mod undo;
mod update;
mod vault_cmd;
mod visualize_cmd;
mod voice_cmd;
mod watch_cmd;
mod web_cmd;

use std::process::ExitCode;

// §32.2 (ADR 0062)
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use kor_config::{ConfigError, Layer, Loaded, Paths};
use kor_core::PermissionMode;
use kor_i18n::{Lang, Localizer, args};

#[derive(Debug, Parser)]
#[command(name = "kor", version)]
struct Cli {
    #[arg(long, global = true, value_parser = parse_lang)]
    lang: Option<Lang>,
    /// Local providers only; every outbound call of Kor is off (config `privacy.mode`).
    #[arg(long, global = true)]
    privacy_mode: bool,
    /// Run on a server over SSH (`user@host[:port]` or a connections.toml id).
    #[arg(long, global = true)]
    remote: Option<String>,
    /// Project folder on the server (with --remote).
    #[arg(long)]
    project: Option<String>,
    #[arg(short = 'p', long)]
    print: bool,
    /// With -p: events as JSON lines, the context breakdown as `context.usage` (§28.1).
    #[arg(long, requires = "print")]
    json: bool,
    #[arg(short, long)]
    model: Option<String>,
    #[arg(long, value_parser = parse_mode)]
    mode: Option<PermissionMode>,
    prompt: Vec<String>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    Resume {
        id: Option<String>,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long, value_parser = parse_mode)]
        mode: Option<PermissionMode>,
    },
    /// Session tree (`kor sessions [ls|search|mv|pin|archive|tag|color|rename|suggest|export|import]`, §28.6).
    Sessions(sessions_cmd::SessionsArgs),
    /// Shell completions (`kor completions <shell> [--install|--uninstall]`, §26.1).
    Completions(completions_cmd::CompletionsArgs),
    /// Plans saved in plan mode (`kor plan list|show|approve`, §28.2).
    Plan {
        #[command(subcommand)]
        command: Option<plan_cmd::PlanCmd>,
    },
    /// Running tasks, pending approvals, daemon and remote tasks (`kor status [--watch|--json]`).
    Status(status_cmd::StatusArgs),
    /// Approve a pending request from any session, the daemon or a remote task.
    Approve(status_cmd::AnswerArgs),
    /// Deny a pending request.
    Deny(status_cmd::AnswerArgs),
    /// Approval queue: every pending request, one card at a time.
    Approvals(status_cmd::ApprovalsArgs),
    /// Notification center (`kor notifications [list|read|clear]`).
    Notifications {
        #[command(subcommand)]
        command: Option<notify_cmd::NotifyCmd>,
    },
    /// OS shell integration: context menus, terminals (ADR 0052).
    Integrate(integrate_cmd::IntegrateArgs),
    /// Batch file operations with a plan (§27.10).
    Files(files_cmd::FilesArgs),
    /// Kor folder in the file manager (ADR 0052).
    Folder {
        #[command(subcommand)]
        command: folder_cmd::FolderCmd,
    },
    /// Mount a project or remote as a drive (ADR 0052).
    Mount(folder_cmd::MountArgs),
    /// Extra OS integrations (ADR 0052).
    OsIntegration(osint_cmd::OsIntegrationArgs),
    /// Hosting panels and servers (§29.1).
    Host {
        #[command(subcommand)]
        command: web_cmd::HostCmd,
    },
    /// WordPress/WooCommerce (§29.3–29.4).
    Wp {
        #[command(subcommand)]
        command: web_cmd::WpCmd,
    },
    /// Redirect maps (§29.11).
    Redirects {
        #[command(subcommand)]
        command: web_cmd::RedirectsCmd,
    },
    /// Proposals and contracts (§29.12).
    Proposal {
        #[command(subcommand)]
        command: web_cmd::ProposalCmd,
    },
    /// Client workspaces and reports (§29.13).
    Client {
        #[command(subcommand)]
        command: web_cmd::ClientCmd,
    },
    /// Turkey layer (§29.14).
    Tr {
        #[command(subcommand)]
        command: web_cmd::TrCmd,
    },
    /// Technical SEO (kor seo audit|verify|gsc|bing|content|meta|logs|competitors|agents|opportunities).
    Seo(seo_cmd::SeoArgs),
    /// Natural-language command routing (`kor intents list|test|add|forget`, §30).
    Intents {
        #[command(subcommand)]
        command: intent_cmd::IntentsCmd,
    },
    /// Command-line tools on PATH and typed tool definitions (`kor tool …`, §26.2).
    Tool {
        #[command(subcommand)]
        command: tool_cmd::ToolCmd,
    },
    /// Kor-to-Kor bridge: pair, delegate, hand off (§31.1).
    Peer {
        #[command(subcommand)]
        command: peer_cmd::PeerCmd,
    },
    /// Agent Client Protocol: serve editors, run ACP agents (§28.8).
    Acp(acp_cmd::AcpArgs),
    /// Kor browser extension: pair, install, domains, form profiles.
    Chrome {
        #[command(subcommand)]
        command: chrome_cmd::ChromeCmd,
    },
    /// Desktop control and per-app tiers (§19.1).
    Computer {
        #[command(subcommand)]
        command: computer_cmd::ComputerCmd,
    },
    /// This machine's profile and health (§19.2–19.3).
    System {
        #[command(subcommand)]
        command: Option<system_cmd::SystemCmd>,
    },
    /// Clean build and tool caches (§19.3).
    Clean(clean_cmd::CleanArgs),
    /// Window layouts across monitors (§27.7).
    Layout {
        #[command(subcommand)]
        command: layout_cmd::LayoutCmd,
    },
    /// System prerequisites only the OS can install (§24.3).
    Prereq {
        #[command(subcommand)]
        command: prereq_cmd::PrereqCmd,
    },
    #[command(hide = true)]
    InstallHooks {
        #[command(subcommand)]
        command: install_hooks_cmd::HookCmd,
    },
    /// All user data in one archive (`kor export --all`, §22.2).
    Export(export_cmd::ExportArgs),
    /// Knowledge base search, weekly digest, stale notes (§27.15).
    Know(know_cmd::KnowArgs),
    /// Time and cost estimate before a task (§27.16).
    Estimate(estimate_cmd::EstimateArgs),
    /// Project rule compliance (§28.3).
    Rules {
        #[command(subcommand)]
        command: rules_cmd::RulesCmd,
    },
    /// Private documents in the knowledge base, page docs cache (§28.7, §27.11).
    Docs {
        #[command(subcommand)]
        command: docs_cmd::DocsCmd,
    },
    /// Full-screen terminal interface (§15.18).
    Tui {
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long, value_parser = parse_mode)]
        mode: Option<PermissionMode>,
    },
    Compat {
        #[command(subcommand)]
        command: CompatCmd,
    },
    Trust {
        #[arg(long)]
        revoke: bool,
    },
    Doctor {
        /// Fix what can be fixed safely, asking first for each (§26.1).
        #[arg(long)]
        fix: bool,
        /// With --fix: answer yes in advance.
        #[arg(long, short = 'y', requires = "fix")]
        yes: bool,
        /// Summarise perf.log: where Kor was slow (§32.3).
        #[arg(long, conflicts_with = "fix")]
        perf: bool,
        /// Run a traced task through the core with the mock provider and write the IPC trace (§17.2).
        #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "", conflicts_with_all = ["fix", "perf"])]
        ipc_trace: Option<std::path::PathBuf>,
    },
    /// Model intelligence, speed and price data with sources (`kor catalog [model]`).
    Catalog {
        model: Option<String>,
        #[arg(long, conflicts_with = "offline")]
        refresh: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Hooks from `hooks.toml` and Claude Code settings (`kor hooks list`).
    Hooks {
        #[command(subcommand)]
        command: hooks_cmd::HooksCmd,
    },
    /// Image, video and audio models on fal.ai (`kor media run <model>`).
    Media {
        #[command(subcommand)]
        command: media_cmd::MediaCmd,
    },
    /// Hugging Face Hub search, model info, downloads and task inference.
    Hf {
        #[command(subcommand)]
        command: hf_cmd::HfCmd,
    },
    /// Agents the `Agent` tool can start.
    Agents,
    /// This project's memory (`MEMORY.md`, the project memory folder) and git notes (§6.1).
    Memory {
        #[command(subcommand)]
        command: Option<memory_cmd::MemoryCmd>,
    },
    /// Search the knowledge base of earlier work in every project (§31.3).
    Recall(memory_cmd::RecallArgs),
    /// Delete a knowledge base record.
    Forget { id: i64 },
    /// Knowledge base export (`kor knowledge export [--md|--json] [dir]`).
    Knowledge {
        #[command(subcommand)]
        command: memory_cmd::KnowledgeCmd,
    },
    /// Folders of the multi-repo view (`kor workspace add|list|remove`).
    Workspace {
        #[command(subcommand)]
        command: memory_cmd::WorkspaceCmd,
    },
    /// Live project graph (`kor graph status|explain|path|query|hotspots|diff|impact|export|update`).
    Graph(graph_cmd::GraphArgs),
    /// Database schema in the project graph (`kor db connect|refresh|tables|show`).
    Db {
        #[command(subcommand)]
        command: Option<db_cmd::DbCmd>,
    },
    /// Secret vault (`kor vault add|list|remove|import`).
    Vault {
        #[command(subcommand)]
        command: vault_cmd::VaultCmd,
    },
    /// Security/compliance analysis (`kor audit --security|--compliance`) and the hash-chained audit log (`kor audit list|verify`).
    Audit(audit_cmd::AuditArgs),
    /// Command sandbox: backend, limits, network allowlist, self-test (`kor sandbox status|test|pull`).
    Sandbox {
        #[command(subcommand)]
        command: sandbox_cmd::SandboxCmd,
    },
    /// Offline / air-gap (§6.5).
    Offline {
        #[command(subcommand)]
        command: offline_cmd::OfflineCmd,
    },
    /// Multi-agent task (§8.2): `kor task "<goal>" --agents N`, list|status|resume|fork|cancel|resolve.
    Task(task_cmd::TaskArgs),
    /// Pull requests on GitHub, GitLab, Gitea/Forgejo, Bitbucket.
    Pr {
        #[command(subcommand)]
        command: pr_cmd::PrCmd,
    },
    /// Review a diff with a persona.
    Review(review_cmd::ReviewArgs),
    /// Background process that runs triggers (`kor daemon start|stop|status|logs|run|install|uninstall`).
    Daemon {
        #[command(subcommand)]
        command: daemon_cmd::DaemonCmd,
    },
    /// Event-driven tasks (`kor triggers list|test|history|approve|reject|hook`).
    Triggers {
        #[command(subcommand)]
        command: triggers_cmd::TriggersCmd,
    },
    /// Deploy changed services (`kor deploy <env>|risk-check <env>|status`).
    Deploy(deploy_cmd::DeployArgs),
    /// Watch logs for errors and fix them (`kor watch logs|incidents|fix|pr|clear`).
    Watch {
        #[command(subcommand)]
        command: watch_cmd::WatchCmd,
    },
    /// Natural-language codebase search (§8.9).
    Find(find_cmd::FindArgs),
    /// Dependencies: vulnerabilities, unused, duplicates, upgrades, list (§9).
    Deps {
        #[command(subcommand)]
        command: deps_cmd::DepsCmd,
    },
    /// Code archaeology of a file (`kor explain <file> [--level junior|normal|expert]`).
    Explain(explain_cmd::ExplainArgs),
    /// Project quality grade per dimension (`kor quality [--history]`).
    Quality(quality_cmd::QualityArgs),
    /// Technical debt in estimated hours (`kor debt`).
    Debt(quality_cmd::DebtArgs),
    /// Coding style profile (`kor style [check]`).
    Style(quality_cmd::StyleArgs),
    /// Weekly developer statistics (`kor stats [--week]`).
    Stats(quality_cmd::StatsArgs),
    /// What a change affects (`kor impact <file|symbol> | --diff | --staged`).
    Impact(quality_cmd::ImpactArgs),
    /// Unreachable code (`kor dead-code [--prune]`).
    #[command(name = "dead-code")]
    DeadCode(deadcode_cmd::DeadCodeArgs),
    /// UI screenshot → components (§10).
    Screenshot(screenshot_cmd::ScreenshotArgs),
    /// Architecture diagram → module skeleton (§10).
    Diagram(diagram_cmd::DiagramArgs),
    /// Architecture map, call graph and data flow from the project graph (§10).
    Visualize(visualize_cmd::VisualizeArgs),
    /// Speech → task text with local whisper.cpp (§10).
    Voice(voice_cmd::VoiceArgs),
    /// Profile a command, the tests or an HTTP endpoint (`kor perf <target> | report <file> | list`).
    Perf(perf_cmd::PerfArgs),
    /// Replay recorded sessions (`kor replay [<id>] | list | export <id>`).
    Replay(replay_cmd::ReplayArgs),
    /// Project environment and pinned tool versions (`kor env status`).
    Env {
        #[command(subcommand)]
        command: env_cmd::EnvCmd,
    },
    /// Remote side of `kor --remote` (internal).
    RemoteServe {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, hide = true)]
        worker: Option<String>,
    },
    /// Follow a remote task again (with --remote); no task = list.
    Attach { task: Option<String> },
    /// Read-only scan of a server over SSH (§15.16).
    Discover(discover_cmd::DiscoverArgs),
    /// Monorepo packages: list, affected by changes, dependencies, cost (§15.15).
    Packages {
        #[command(subcommand)]
        command: Option<packages_cmd::PackagesCmd>,
    },
    /// SSH hosts and databases from `connections.toml`.
    Connections,
    /// Cost dashboard, export and routing decisions.
    Cost {
        #[arg(long, default_value_t = 30)]
        days: u32,
        #[arg(long, value_enum)]
        export: Option<cost_cmd::Export>,
        #[command(subcommand)]
        command: Option<cost_cmd::CostCmd>,
    },
    /// Remaining provider quota per key.
    Quota,
    /// Context window breakdown of the last request, thresholds and recent use (§5.3).
    Context {
        #[arg(long, default_value_t = 1)]
        history: usize,
        #[command(subcommand)]
        command: Option<memory_cmd::ContextCmd>,
    },
    /// Race models on the project's own test suite and suggest routing (`kor bench`).
    Bench(bench_cmd::BenchArgs),
    /// Shell integration (`kor shell install|uninstall|status|capture`).
    Shell {
        #[command(subcommand)]
        command: shell_cmd::ShellCmd,
    },
    /// Explain the last failed command and offer a corrected one.
    Fix {
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Explain why the last command failed.
    Why {
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Ask 2–3 models in parallel and merge what they agree on (§5.1).
    Consensus(consensus_cmd::ConsensusArgs),
    /// Translate code comments into a language; write missing ones (§12.2).
    Comment(comment_cmd::CommentArgs),
    /// Conventional Commits message from the staged diff.
    Commit(commit_cmd::CommitArgs),
    /// Keep a Changelog section from commits.
    Changelog(commit_cmd::ChangelogArgs),
    /// Prompt and task templates (`kor templates list|show|new`).
    Templates {
        #[command(subcommand)]
        command: templates_cmd::TemplatesCmd,
    },
    /// Run a task template: `kor run template:<name> --<input> value`.
    Run {
        target: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        inputs: Vec<String>,
    },
    /// Scheduled tasks (run by the desktop app while it is open).
    Routines {
        #[command(subcommand)]
        command: extra::RoutineCmd,
    },
    /// Check for, install or roll back a signed release (`kor update`, `kor update check`).
    Update {
        #[arg(long)]
        from: Option<std::path::PathBuf>,
        #[command(subcommand)]
        command: Option<update::UpdateCmd>,
    },
    /// Downloadable helper programs: Python, Node, PDF renderer, OCR models, headless Chromium.
    Component {
        #[command(subcommand)]
        command: component_cmd::ComponentCmd,
    },
    /// Revert every file change of a session (`kor undo`, `kor undo --list`).
    Undo {
        #[arg(long)]
        list: bool,
        #[arg(long)]
        session: Option<String>,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCmd,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
    Providers {
        #[command(subcommand)]
        command: Option<privacy_cmd::ProvidersCmd>,
    },
    /// Test, monitor and add providers; manage the key pool.
    Provider {
        #[command(subcommand)]
        command: provider_cmd::ProviderCmd,
    },
    /// Embedded CLIProxyAPI-compatible gateway (`kor proxy serve|status|key`).
    Proxy {
        #[command(subcommand)]
        command: proxy_cmd::ProxyCmd,
    },
    Models {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, conflicts_with = "offline")]
        refresh: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Chrome native messaging host (4-byte length prefix).
    ChromeNativeHost,
    /// List stdio servers declared in .mcp.json, or serve tools on stdio.
    Mcp {
        #[command(subcommand)]
        command: Option<mcp_cmd::McpCmd>,
    },
    /// Plugin marketplaces and installs.
    Plugin {
        #[command(subcommand)]
        command: PluginCmd,
    },
    Chat {
        #[arg(short, long)]
        model: String,
        #[arg(short, long)]
        system: Option<String>,
        #[arg(long)]
        max_tokens: Option<u32>,
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum PluginCmd {
    Marketplace {
        #[command(subcommand)]
        command: MarketCmd,
    },
    Install {
        plugin: String,
    },
    Uninstall {
        plugin: String,
    },
    List,
    Enable {
        plugin: String,
    },
    Disable {
        plugin: String,
    },
    Update {
        plugin: String,
    },
    Doctor {
        plugin: String,
    },
}

#[derive(Debug, Subcommand)]
enum MarketCmd {
    Add { name: String, source: String },
    List,
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
enum AuthCmd {
    Login {
        provider: String,
        /// Claude.ai subscription OAuth instead of an API key.
        #[arg(long)]
        claudeai: bool,
    },
    Status {
        provider: Option<String>,
    },
    Logout {
        provider: String,
    },
}

#[derive(Debug, Subcommand)]
enum CompatCmd {
    Status {
        #[arg(long)]
        all: bool,
    },
    Conflicts,
    Export {
        #[arg(long)]
        to: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCmd {
    Explain {
        /// Only this section (`auto`, `keys`, `ui`, `tui`, `context`…).
        section: Option<String>,
    },
    Paths,
    /// `kor config set computer.enabled true`: writes the user config file, validated.
    Set {
        key: String,
        value: String,
    },
}

fn parse_lang(s: &str) -> Result<Lang, String> {
    s.parse()
}

fn parse_mode(s: &str) -> Result<PermissionMode, String> {
    s.parse()
}

/// Localize clap's about/help strings. Clap's own built-ins stay English.
#[allow(clippy::too_many_lines)] // one flat builder chain, one block per command
fn localized_command(l: &Localizer) -> clap::Command {
    let provider = |c: clap::Command| c.mut_arg("provider", |a| a.help(l.t("arg-provider")));
    Cli::command()
        .about(l.t("app-about"))
        .mut_arg("lang", |a| a.help(l.t("arg-lang")))
        .mut_arg("privacy_mode", |a| a.help(l.t("arg-privacy-mode")))
        .mut_arg("print", |a| a.help(l.t("arg-print")))
        .mut_arg("json", |a| a.help(l.t("arg-print-json")))
        .mut_arg("model", |a| a.help(l.t("arg-model-top")))
        .mut_arg("mode", |a| a.help(l.t("arg-mode")))
        .mut_arg("prompt", |a| a.help(l.t("arg-prompt-top")))
        .mut_subcommand("resume", |c| {
            c.about(l.t("cmd-resume"))
                .mut_arg("id", |a| a.help(l.t("arg-session-id")))
                .mut_arg("model", |a| a.help(l.t("arg-model-top")))
                .mut_arg("mode", |a| a.help(l.t("arg-mode")))
        })
        .mut_subcommand("sessions", |c| sessions_cmd::localize(c, l))
        .mut_subcommand("completions", |c| completions_cmd::localize(c, l))
        .mut_subcommand("plan", |c| plan_cmd::localize(c, l))
        .mut_subcommand("tui", |c| {
            c.about(l.t("cmd-tui"))
                .mut_arg("model", |a| a.help(l.t("arg-model-top")))
                .mut_arg("mode", |a| a.help(l.t("arg-mode")))
        })
        .mut_subcommand("compat", |c| {
            c.about(l.t("cmd-compat"))
                .mut_subcommand("status", |s| {
                    s.about(l.t("cmd-compat-status"))
                        .mut_arg("all", |a| a.help(l.t("arg-all")))
                })
                .mut_subcommand("conflicts", |s| s.about(l.t("cmd-compat-conflicts")))
                .mut_subcommand("export", |s| {
                    s.about(l.t("cmd-compat-export"))
                        .mut_arg("to", |a| a.help(l.t("arg-compat-export-to")))
                })
        })
        .mut_subcommand("trust", |c| {
            c.about(l.t("cmd-trust"))
                .mut_arg("revoke", |a| a.help(l.t("arg-revoke")))
        })
        .mut_subcommand("doctor", |c| {
            c.about(l.t("cmd-doctor"))
                .mut_arg("fix", |a| a.help(l.t("arg-doctor-fix")))
                .mut_arg("yes", |a| a.help(l.t("arg-yes")))
                .mut_arg("perf", |a| a.help(l.t("arg-doctor-perf")))
                .mut_arg("ipc_trace", |a| a.help(l.t("arg-doctor-ipc-trace")))
        })
        .mut_subcommand("context", |c| memory_cmd::localize(c, l))
        .mut_subcommand("recall", |c| memory_cmd::localize(c, l))
        .mut_subcommand("graph", |c| graph_cmd::localize(c, l))
        .mut_subcommand("db", |c| db_cmd::localize(c, l))
        .mut_subcommand("vault", |c| vault_cmd::localize(c, l))
        .mut_subcommand("audit", |c| audit_cmd::localize(c, l))
        .mut_subcommand("sandbox", |c| sandbox_cmd::localize(c, l))
        .mut_subcommand("offline", |c| offline_cmd::localize(c, l))
        .mut_subcommand("status", |c| status_cmd::localize_status(c, l))
        .mut_subcommand("approve", |c| status_cmd::localize_approve(c, l))
        .mut_subcommand("deny", |c| status_cmd::localize_deny(c, l))
        .mut_subcommand("approvals", |c| status_cmd::localize_approvals(c, l))
        .mut_subcommand("notifications", |c| notify_cmd::localize(c, l))
        .mut_subcommand("integrate", |c| integrate_cmd::localize(c, l))
        .mut_subcommand("files", |c| files_cmd::localize(c, l))
        .mut_subcommand("folder", |c| folder_cmd::localize(c, l))
        .mut_subcommand("mount", |c| folder_cmd::localize_mount(c, l))
        .mut_subcommand("os-integration", |c| osint_cmd::localize(c, l))
        .mut_subcommand("host", |c| web_cmd::localize_host(c, l))
        .mut_subcommand("wp", |c| web_cmd::localize_wp(c, l))
        .mut_subcommand("redirects", |c| web_cmd::localize_redirects(c, l))
        .mut_subcommand("proposal", |c| web_cmd::localize_proposal(c, l))
        .mut_subcommand("client", |c| web_cmd::localize_client(c, l))
        .mut_subcommand("tr", |c| web_cmd::localize_tr(c, l))
        .mut_subcommand("seo", |c| seo_cmd::localize(c, l))
        .mut_subcommand("intents", |c| intent_cmd::localize(c, l))
        .mut_subcommand("tool", |c| tool_cmd::localize(c, l))
        .mut_subcommand("peer", |c| peer_cmd::localize(c, l))
        .mut_subcommand("acp", |c| acp_cmd::localize(c, l))
        .mut_subcommand("chrome", |c| chrome_cmd::localize(c, l))
        .mut_subcommand("computer", |c| computer_cmd::localize(c, l))
        .mut_subcommand("system", |c| system_cmd::localize(c, l))
        .mut_subcommand("clean", |c| clean_cmd::localize(c, l))
        .mut_subcommand("layout", |c| layout_cmd::localize(c, l))
        .mut_subcommand("prereq", |c| prereq_cmd::localize(c, l))
        .mut_subcommand("export", |c| export_cmd::localize(c, l))
        .mut_subcommand("know", |c| know_cmd::localize(c, l))
        .mut_subcommand("estimate", |c| estimate_cmd::localize(c, l))
        .mut_subcommand("rules", |c| rules_cmd::localize(c, l))
        .mut_subcommand("docs", |c| docs_cmd::localize(c, l))
        .mut_subcommand("task", |c| task_cmd::localize(c, l))
        .mut_subcommand("pr", |c| pr_cmd::localize(c, l))
        .mut_subcommand("review", |c| review_cmd::localize(c, l))
        .mut_subcommand("daemon", |c| daemon_cmd::localize(c, l))
        .mut_subcommand("triggers", |c| triggers_cmd::localize(c, l))
        .mut_subcommand("deploy", |c| deploy_cmd::localize(c, l))
        .mut_subcommand("watch", |c| watch_cmd::localize(c, l))
        .mut_subcommand("find", |c| find_cmd::localize(c, l))
        .mut_subcommand("deps", |c| deps_cmd::localize(c, l))
        .mut_subcommand("explain", |c| explain_cmd::localize(c, l))
        .mut_subcommand("quality", |c| quality_cmd::localize_quality(c, l))
        .mut_subcommand("debt", |c| quality_cmd::localize_debt(c, l))
        .mut_subcommand("style", |c| quality_cmd::localize_style(c, l))
        .mut_subcommand("stats", |c| quality_cmd::localize_stats(c, l))
        .mut_subcommand("impact", |c| quality_cmd::localize_impact(c, l))
        .mut_subcommand("dead-code", |c| deadcode_cmd::localize(c, l))
        .mut_subcommand("screenshot", |c| screenshot_cmd::localize(c, l))
        .mut_subcommand("diagram", |c| diagram_cmd::localize(c, l))
        .mut_subcommand("visualize", |c| visualize_cmd::localize(c, l))
        .mut_subcommand("voice", |c| voice_cmd::localize(c, l))
        .mut_subcommand("perf", |c| perf_cmd::localize(c, l))
        .mut_subcommand("replay", |c| replay_cmd::localize(c, l))
        .mut_subcommand("env", |c| env_cmd::localize(c, l))
        .mut_subcommand("mcp", |c| mcp_cmd::localize(c, l))
        .mut_arg("remote", |a| a.help(l.t("arg-remote")))
        .mut_arg("project", |a| a.help(l.t("arg-remote-project-top")))
        .mut_subcommand("remote-serve", |c| remote_cmd::localize_serve(c, l))
        .mut_subcommand("attach", |c| remote_cmd::localize_attach(c, l))
        .mut_subcommand("discover", |c| discover_cmd::localize(c, l))
        .mut_subcommand("forget", |c| memory_cmd::localize(c, l))
        .mut_subcommand("knowledge", |c| memory_cmd::localize(c, l))
        .mut_subcommand("workspace", |c| memory_cmd::localize(c, l))
        .mut_subcommand("packages", |c| {
            c.about(l.t("cmd-packages"))
                .mut_subcommand("affected", |s| {
                    s.about(l.t("cmd-packages-affected"))
                        .mut_arg("since", |a| a.help(l.t("arg-packages-since")))
                })
                .mut_subcommand("deps", |s| {
                    s.about(l.t("cmd-packages-deps"))
                        .mut_arg("name", |a| a.help(l.t("arg-packages-name")))
                })
                .mut_subcommand("cost", |s| {
                    s.about(l.t("cmd-packages-cost"))
                        .mut_arg("days", |a| a.help(l.t("arg-cost-days")))
                })
        })
        .mut_subcommand("bench", |c| bench_cmd::localize(c, l))
        .mut_subcommand("shell", |c| {
            c.about(l.t("cmd-shell"))
                .mut_subcommand("install", |s| s.about(l.t("cmd-shell-install")))
                .mut_subcommand("uninstall", |s| s.about(l.t("cmd-shell-uninstall")))
                .mut_subcommand("status", |s| s.about(l.t("cmd-shell-status")))
                .mut_subcommand("capture", |s| s.about(l.t("cmd-shell-capture")))
        })
        .mut_subcommand("fix", |c| c.about(l.t("cmd-fix")))
        .mut_subcommand("why", |c| c.about(l.t("cmd-why")))
        .mut_subcommand("comment", |c| comment_cmd::localize(c, l))
        .mut_subcommand("consensus", |c| consensus_cmd::localize(c, l))
        .mut_subcommand("commit", |c| c.about(l.t("cmd-commit")))
        .mut_subcommand("changelog", |c| c.about(l.t("cmd-changelog")))
        .mut_subcommand("templates", |c| {
            c.about(l.t("cmd-templates"))
                .mut_subcommand("list", |s| s.about(l.t("cmd-templates-list")))
                .mut_subcommand("show", |s| {
                    s.about(l.t("cmd-templates-show"))
                        .mut_arg("name", |a| a.help(l.t("arg-template-name")))
                })
                .mut_subcommand("new", |s| {
                    s.about(l.t("cmd-templates-new"))
                        .mut_arg("name", |a| a.help(l.t("arg-template-name")))
                        .mut_arg("personal", |a| a.help(l.t("arg-template-personal")))
                })
        })
        .mut_subcommand("run", |c| {
            c.about(l.t("cmd-run"))
                .mut_arg("target", |a| a.help(l.t("arg-run-target")))
                .mut_arg("inputs", |a| a.help(l.t("arg-run-inputs")))
        })
        .mut_subcommand("catalog", |c| {
            c.about(l.t("cmd-catalog"))
                .mut_arg("model", |a| a.help(l.t("arg-catalog-model")))
                .mut_arg("refresh", |a| a.help(l.t("arg-refresh")))
                .mut_arg("offline", |a| a.help(l.t("arg-offline")))
        })
        .mut_subcommand("hooks", |c| hooks_cmd::localize(c, l))
        .mut_subcommand("media", |c| media_cmd::localize(c, l))
        .mut_subcommand("hf", |c| {
            c.about(l.t("cmd-hf"))
                .mut_subcommand("search", |s| {
                    s.about(l.t("cmd-hf-search"))
                        .mut_arg("query", |a| a.help(l.t("arg-hf-query")))
                        .mut_arg("author", |a| a.help(l.t("arg-hf-author")))
                        .mut_arg("filter", |a| a.help(l.t("arg-hf-filter")))
                        .mut_arg("pipeline", |a| a.help(l.t("arg-hf-pipeline")))
                        .mut_arg("sort", |a| a.help(l.t("arg-hf-sort")))
                        .mut_arg("limit", |a| a.help(l.t("arg-hf-limit")))
                })
                .mut_subcommand("info", |s| {
                    s.about(l.t("cmd-hf-info"))
                        .mut_arg("repo", |a| a.help(l.t("arg-hf-repo")))
                        .mut_arg("revision", |a| a.help(l.t("arg-hf-revision")))
                })
                .mut_subcommand("download", |s| {
                    s.about(l.t("cmd-hf-download"))
                        .mut_arg("repo", |a| a.help(l.t("arg-hf-repo")))
                        .mut_arg("file", |a| a.help(l.t("arg-hf-file")))
                        .mut_arg("include", |a| a.help(l.t("arg-hf-include")))
                        .mut_arg("revision", |a| a.help(l.t("arg-hf-revision")))
                        .mut_arg("dir", |a| a.help(l.t("arg-hf-dir")))
                        .mut_arg("yes", |a| a.help(l.t("arg-hf-yes")))
                })
                .mut_subcommand("infer", |s| {
                    s.about(l.t("cmd-hf-infer"))
                        .mut_arg("model", |a| a.help(l.t("arg-hf-repo")))
                        .mut_arg("task", |a| a.help(l.t("arg-hf-task")))
                        .mut_arg("input", |a| a.help(l.t("arg-hf-input")))
                        .mut_arg("endpoint", |a| a.help(l.t("arg-hf-endpoint")))
                })
        })
        .mut_subcommand("agents", |c| c.about(l.t("cmd-agents")))
        .mut_subcommand("update", |c| {
            c.about(l.t("cmd-update"))
                .mut_arg("from", |a| a.help(l.t("arg-update-from")))
                .mut_subcommand("check", |s| s.about(l.t("cmd-update-check")))
                .mut_subcommand("channel", |s| s.about(l.t("cmd-update-channel")))
                .mut_subcommand("rollback", |s| s.about(l.t("cmd-update-rollback")))
        })
        .mut_subcommand("component", |c| {
            c.about(l.t("cmd-component"))
                .mut_subcommand("list", |s| s.about(l.t("cmd-component-list")))
                .mut_subcommand("add", |s| {
                    s.about(l.t("cmd-component-add"))
                        .mut_arg("name", |a| a.help(l.t("arg-component-name")))
                        .mut_arg("from", |a| a.help(l.t("arg-component-from")))
                        .mut_arg("yes", |a| a.help(l.t("arg-component-yes")))
                })
                .mut_subcommand("remove", |s| {
                    s.about(l.t("cmd-component-remove"))
                        .mut_arg("name", |a| a.help(l.t("arg-component-name")))
                })
                .mut_subcommand("install", |s| {
                    s.about(l.t("cmd-component-install"))
                        .mut_arg("all", |a| a.help(l.t("arg-component-all")))
                        .mut_arg("yes", |a| a.help(l.t("arg-component-yes")))
                })
                .mut_subcommand("update", |s| {
                    s.about(l.t("cmd-component-update"))
                        .mut_arg("yes", |a| a.help(l.t("arg-component-yes")))
                })
                .mut_subcommand("verify", |s| {
                    s.about(l.t("cmd-component-verify"))
                        .mut_arg("name", |a| a.help(l.t("arg-component-name")))
                })
        })
        .mut_subcommand("memory", |c| memory_cmd::localize(c, l))
        .mut_subcommand("connections", |c| c.about(l.t("cmd-connections")))
        .mut_subcommand("cost", |c| {
            c.about(l.t("cmd-cost"))
                .mut_arg("days", |a| a.help(l.t("arg-cost-days")))
                .mut_arg("export", |a| a.help(l.t("arg-cost-export")))
                .mut_subcommand("why", |w| {
                    w.about(l.t("cmd-cost-why"))
                        .mut_arg("last", |a| a.help(l.t("arg-cost-why-last")))
                })
        })
        .mut_subcommand("quota", |c| c.about(l.t("cmd-quota")))
        .mut_subcommand("routines", |c| {
            c.about(l.t("cmd-routines"))
                .mut_subcommand("list", |s| s.about(l.t("cmd-routines-list")))
                .mut_subcommand("add", |s| s.about(l.t("cmd-routines-add")))
                .mut_subcommand("remove", |s| s.about(l.t("cmd-routines-remove")))
                .mut_subcommand("enable", |s| s.about(l.t("cmd-routines-enable")))
                .mut_subcommand("disable", |s| s.about(l.t("cmd-routines-disable")))
        })
        .mut_subcommand("undo", |c| {
            c.about(l.t("cmd-undo"))
                .mut_arg("list", |a| a.help(l.t("arg-undo-list")))
                .mut_arg("session", |a| a.help(l.t("arg-undo-session")))
        })
        .mut_subcommand("auth", |c| {
            c.about(l.t("cmd-auth"))
                .mut_subcommand("login", |s| provider(s.about(l.t("cmd-auth-login"))))
                .mut_subcommand("status", |s| provider(s.about(l.t("cmd-auth-status"))))
                .mut_subcommand("logout", |s| provider(s.about(l.t("cmd-auth-logout"))))
        })
        .mut_subcommand("config", |c| {
            c.about(l.t("cmd-config"))
                .mut_subcommand("explain", |s| {
                    s.about(l.t("cmd-config-explain"))
                        .mut_arg("section", |a| a.help(l.t("arg-config-section")))
                })
                .mut_subcommand("paths", |s| s.about(l.t("cmd-config-paths")))
                .mut_subcommand("set", |s| s.about(l.t("cmd-config-set")))
        })
        .mut_subcommand("providers", |c| privacy_cmd::localize(c, l))
        .mut_subcommand("provider", |c| {
            let id = |s: clap::Command| s.mut_arg("id", |a| a.help(l.t("arg-provider")));
            c.about(l.t("cmd-provider"))
                .mut_subcommand("test", |s| {
                    id(s.about(l.t("cmd-provider-test")))
                        .mut_arg("model", |a| a.help(l.t("arg-provider-model")))
                })
                .mut_subcommand("status", |s| id(s.about(l.t("cmd-provider-status"))))
                .mut_subcommand("add", |s| {
                    id(s.about(l.t("cmd-provider-add")))
                        .mut_arg("url", |a| a.help(l.t("arg-provider-url")))
                        .mut_arg("anthropic", |a| a.help(l.t("arg-provider-anthropic")))
                        .mut_arg("no_key", |a| a.help(l.t("arg-provider-no-key")))
                        .mut_arg("proxy", |a| a.help(l.t("arg-provider-proxy")))
                })
                .mut_subcommand("remove", |s| id(s.about(l.t("cmd-provider-remove"))))
                .mut_subcommand("discover", |s| {
                    s.about(l.t("cmd-provider-discover"))
                        .mut_arg("save", |a| a.help(l.t("arg-provider-save")))
                })
                .mut_subcommand("keys", |s| id(s.about(l.t("cmd-provider-keys"))))
                .mut_subcommand("add-key", |s| id(s.about(l.t("cmd-provider-add-key"))))
                .mut_subcommand("remove-key", |s| {
                    id(s.about(l.t("cmd-provider-remove-key")))
                        .mut_arg("slot", |a| a.help(l.t("arg-provider-slot")))
                })
        })
        .mut_subcommand("proxy", |c| {
            let at = |s: clap::Command| {
                s.mut_arg("port", |a| a.help(l.t("arg-proxy-port")))
                    .mut_arg("host", |a| a.help(l.t("arg-proxy-host")))
            };
            c.about(l.t("cmd-proxy"))
                .mut_subcommand("serve", |s| {
                    at(s.about(l.t("cmd-proxy-serve")))
                        .mut_arg("config", |a| a.help(l.t("arg-proxy-config")))
                        .mut_arg("no_auth", |a| a.help(l.t("arg-proxy-no-auth")))
                        .mut_arg("background", |a| a.help(l.t("arg-proxy-background")))
                })
                .mut_subcommand("status", |s| at(s.about(l.t("cmd-proxy-status"))))
                .mut_subcommand("key", |s| {
                    s.about(l.t("cmd-proxy-key"))
                        .mut_arg("rotate", |a| a.help(l.t("arg-proxy-rotate")))
                        .mut_arg("show", |a| a.help(l.t("arg-proxy-show")))
                })
                .mut_subcommand("auth", |s| {
                    s.about(l.t("cmd-proxy-auth"))
                        .mut_subcommand("login", |s| {
                            s.about(l.t("cmd-proxy-auth-login"))
                                .mut_arg("provider", |a| {
                                    a.help(l.t("arg-proxy-auth-provider")).required(true)
                                })
                        })
                        .mut_subcommand("list", |s| s.about(l.t("cmd-proxy-auth-list")))
                        .mut_subcommand("refresh", |s| {
                            s.about(l.t("cmd-proxy-auth-refresh")).mut_arg("file", |a| {
                                a.help(l.t("arg-proxy-auth-file")).required(true)
                            })
                        })
                        .mut_subcommand("remove", |s| {
                            s.about(l.t("cmd-proxy-auth-remove")).mut_arg("file", |a| {
                                a.help(l.t("arg-proxy-auth-file")).required(true)
                            })
                        })
                })
        })
        .mut_subcommand("models", |c| {
            c.about(l.t("cmd-models"))
                .mut_arg("provider", |a| a.help(l.t("arg-provider-filter")))
                .mut_arg("refresh", |a| a.help(l.t("arg-refresh")))
                .mut_arg("offline", |a| a.help(l.t("arg-offline")))
        })
        .mut_subcommand("chat", |c| {
            c.about(l.t("cmd-chat"))
                .mut_arg("model", |a| a.help(l.t("arg-model")))
                .mut_arg("system", |a| a.help(l.t("arg-system")))
                .mut_arg("max_tokens", |a| a.help(l.t("arg-max-tokens")))
                .mut_arg("prompt", |a| a.help(l.t("arg-prompt")))
        })
}

/// `--lang` must be known before clap renders localized help, so scan for it first.
fn prescan_lang(argv: &[String]) -> Option<Lang> {
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            break;
        }
        if let Some(v) = a.strip_prefix("--lang=") {
            return v.parse().ok();
        }
        if a == "--lang" {
            return it.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

struct Ctx {
    l: Localizer,
    paths: Paths,
    config: Result<Loaded, ConfigError>,
}

fn main() -> ExitCode {
    // The clap tree (~100 commands) overflows Windows' 1 MiB main-thread stack in debug builds.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(real_main)
        .map_or(ExitCode::FAILURE, |h| h.join().unwrap_or(ExitCode::FAILURE))
}

fn real_main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let flag = prescan_lang(&argv);

    let cwd = std::env::current_dir().unwrap_or_default();
    let paths = Paths::discover(&cwd);
    let mut cli_overrides: Vec<(&str, kor_config::Value)> = flag
        .map(|l| vec![("ui.language", kor_config::Value::String(l.code().into()))])
        .unwrap_or_default();
    // Read before parsing, like `--lang`: the config is loaded first.
    if argv
        .iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--privacy-mode")
    {
        cli_overrides.push(("privacy.mode", kor_config::Value::Boolean(true)));
    }
    let config = kor_config::load(&paths, std::env::vars(), &cli_overrides);

    // A broken config must not hide the language the user asked for.
    let lang = match &config {
        Ok(c) => c.config.ui.language.resolve(),
        Err(_) => flag
            .or_else(|| std::env::var("KOR_LANG").ok().and_then(|v| v.parse().ok()))
            .or_else(Lang::from_system)
            .unwrap_or(Lang::En),
    };
    let l = Localizer::new(lang);
    if let Ok(c) = &config {
        kor_runtime::apply_network_policy(&l, &c.config);
    }

    let matches = localized_command(&l).get_matches_from(&argv);
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };

    let ctx = Ctx { l, paths, config };
    tui::term::set_json(cli.json);
    kor_runtime::register_extra_tools(kor_web::site::tools::all);
    kor_tools::chrome::set_panel_handler(kor_web::site::panels::handler);
    if cli.command.is_none() && cli.prompt.is_empty() && !cli.print {
        update::on_interactive_start(&ctx);
    }
    // SSH remote mode (ADR 0044): the session runs on the server.
    if let Some(target) = cli.remote.as_deref() {
        let prompt = (!cli.prompt.is_empty()).then(|| cli.prompt.join(" "));
        let project = cli.project.as_deref();
        let r = match cli.command {
            None if cli.print => remote_cmd::run(
                &ctx,
                target,
                project,
                remote_cmd::Mode::Print(prompt.unwrap_or_default()),
            ),
            None => remote_cmd::run(&ctx, target, project, remote_cmd::Mode::Chat(prompt)),
            Some(Cmd::Attach { task }) => {
                remote_cmd::run(&ctx, target, project, remote_cmd::Mode::Attach(task))
            }
            Some(Cmd::Discover(a)) => discover_cmd::run(&ctx, &a, Some(target)),
            Some(_) => Err(anyhow::anyhow!(ctx.l.t("remote-only-session"))),
        };
        return match r {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!(
                    "{}",
                    ctx.l
                        .t_args("error-generic", &args!("error" => format!("{e:#}")))
                );
                ExitCode::FAILURE
            }
        };
    }
    let result = match cli.command {
        None => repl::run(
            &ctx,
            &repl::RunArgs {
                prompt: (!cli.prompt.is_empty()).then(|| cli.prompt.join(" ")),
                print: cli.print,
                model: cli.model,
                mode: cli.mode,
                resume: None,
            },
        ),
        Some(Cmd::Resume { id, model, mode }) => repl::run(
            &ctx,
            &repl::RunArgs {
                model,
                mode,
                resume: Some(id.map_or(repl::Resume::Latest, repl::Resume::Id)),
                ..repl::RunArgs::default()
            },
        ),
        Some(Cmd::Tui { model, mode }) => repl::run_tui(&ctx, model, mode),
        Some(cmd) => run_cmd(&ctx, cmd),
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!(
                "{}",
                ctx.l
                    .t_args("error-generic", &args!("error" => format!("{e:#}")))
            );
            ExitCode::FAILURE
        }
    }
}

/// One non-interactive command. Shared by `kor <command>` and the interactive screen's
/// `/<command>` (parity.toml): both surfaces run exactly the same code.
#[allow(clippy::too_many_lines)] // one dispatch arm per command; splitting it would only scatter the table
fn run_cmd(ctx: &Ctx, cmd: Cmd) -> anyhow::Result<bool> {
    match cmd {
        Cmd::Resume { .. } | Cmd::Tui { .. } => Ok(false),
        Cmd::Sessions(a) => sessions_cmd::run(ctx, &a),
        Cmd::Completions(a) => completions_cmd::run(ctx, localized_command(&ctx.l), &a),
        Cmd::Plan { command } => plan_cmd::run(ctx, command.as_ref()),
        Cmd::Compat {
            command: CompatCmd::Status { all },
        } => compat::status(ctx, all),
        Cmd::Compat {
            command: CompatCmd::Conflicts,
        } => compat::conflicts(ctx),
        Cmd::Compat {
            command: CompatCmd::Export { to },
        } => compat::export(ctx, &to),
        Cmd::Trust { revoke } => compat::trust(ctx, revoke),
        Cmd::Doctor { perf: true, .. } => bench_cmd::doctor_perf(ctx),
        Cmd::Doctor {
            ipc_trace: Some(p), ..
        } => Ok(doctor::ipc_trace::run(
            ctx,
            (!p.as_os_str().is_empty()).then_some(p.as_path()),
        )),
        Cmd::Doctor { fix, yes, .. } => Ok(doctor::run_with(
            ctx,
            fix,
            yes,
            Some(&|| localized_command(&ctx.l)),
        )),
        Cmd::Catalog {
            model,
            refresh,
            offline,
        } => catalog_cmd::run(ctx, model.as_deref(), refresh, offline),
        Cmd::Hooks { command } => hooks_cmd::run(ctx, &command),
        Cmd::Media { command } => media_cmd::run(ctx, &command),
        Cmd::Hf { command } => hf_cmd::run(ctx, &command),
        Cmd::Agents => Ok(extra::agents(ctx)),
        Cmd::Memory { command } => Ok(memory_cmd::memory(ctx, command.as_ref())),
        Cmd::Recall(a) => Ok(memory_cmd::recall(ctx, &a)),
        Cmd::Forget { id } => Ok(memory_cmd::forget(ctx, id)),
        Cmd::Knowledge { command } => Ok(memory_cmd::knowledge(ctx, &command)),
        Cmd::Workspace { command } => Ok(memory_cmd::workspace(ctx, &command)),
        Cmd::Packages { command } => packages_cmd::run(ctx, command.as_ref()),
        Cmd::Graph(a) => graph_cmd::run(ctx, &a),
        Cmd::Db { command } => db_cmd::run(ctx, command.as_ref()),
        Cmd::Vault { command } => vault_cmd::run(ctx, command),
        Cmd::Audit(a) => audit_cmd::dispatch(ctx, a),
        Cmd::Sandbox { command } => sandbox_cmd::run(ctx, &command),
        Cmd::Offline { command } => offline_cmd::run(ctx, command),
        Cmd::Status(a) => status_cmd::status(ctx, &a),
        Cmd::Approve(a) => status_cmd::answer(ctx, &a, true),
        Cmd::Deny(a) => status_cmd::answer(ctx, &a, false),
        Cmd::Approvals(a) => status_cmd::approvals(ctx, &a),
        Cmd::Notifications { command } => notify_cmd::run(ctx, command),
        Cmd::Integrate(a) => integrate_cmd::run(ctx, &a),
        Cmd::Files(a) => files_cmd::run(ctx, &a),
        Cmd::Folder { command } => folder_cmd::run(ctx, command),
        Cmd::Mount(a) => folder_cmd::mount(ctx, &a),
        Cmd::OsIntegration(a) => osint_cmd::run(ctx, &a),
        Cmd::Host { command } => web_cmd::host(ctx, command),
        Cmd::Wp { command } => web_cmd::wp(ctx, command),
        Cmd::Redirects { command } => web_cmd::redirects_run(ctx, command),
        Cmd::Proposal { command } => web_cmd::proposal_run(ctx, command),
        Cmd::Client { command } => web_cmd::client_run(ctx, command),
        Cmd::Tr { command } => web_cmd::tr_run(ctx, command),
        Cmd::Seo(a) => seo_cmd::run(ctx, a),
        Cmd::Intents { command } => intent_cmd::run(ctx, &command),
        Cmd::Tool { command } => tool_cmd::run(ctx, &command),
        Cmd::Peer { command } => peer_cmd::run(ctx, command),
        Cmd::Acp(a) => acp_cmd::run(ctx, a),
        Cmd::Chrome { command } => chrome_cmd::run(ctx, command),
        Cmd::Computer { command } => computer_cmd::run(ctx, command),
        Cmd::System { command } => system_cmd::run(ctx, command),
        Cmd::Clean(a) => clean_cmd::run(ctx, &a),
        Cmd::Layout { command } => layout_cmd::run(ctx, command),
        Cmd::Prereq { command } => prereq_cmd::run(ctx, &command),
        Cmd::InstallHooks { command } => install_hooks_cmd::run(ctx, &command),
        Cmd::Export(a) => export_cmd::run(ctx, &a),
        Cmd::Know(a) => know_cmd::run(ctx, &a),
        Cmd::Estimate(a) => estimate_cmd::run(ctx, &a),
        Cmd::Rules { command } => rules_cmd::run(ctx, &command),
        Cmd::Docs { command } => docs_cmd::run(ctx, &command),
        Cmd::Task(a) => task_cmd::run(ctx, a),
        Cmd::Pr { command } => pr_cmd::run(ctx, command),
        Cmd::Review(a) => review_cmd::run(ctx, &a),
        Cmd::Daemon { command } => daemon_cmd::run(ctx, command),
        Cmd::Triggers { command } => triggers_cmd::run(ctx, command),
        Cmd::Deploy(a) => deploy_cmd::run(ctx, &a),
        Cmd::Watch { command } => watch_cmd::run(ctx, &command),
        Cmd::Find(a) => find_cmd::run(ctx, &a),
        Cmd::Deps { command } => deps_cmd::run(ctx, command),
        Cmd::Explain(a) => explain_cmd::run(ctx, &a),
        Cmd::Quality(a) => quality_cmd::quality(ctx, &a),
        Cmd::Debt(a) => quality_cmd::debt(ctx, &a),
        Cmd::Style(a) => quality_cmd::style(ctx, &a),
        Cmd::Stats(a) => quality_cmd::stats_cmd(ctx, &a),
        Cmd::Impact(a) => quality_cmd::impact_cmd(ctx, &a),
        Cmd::DeadCode(a) => deadcode_cmd::run(ctx, &a),
        Cmd::Screenshot(a) => screenshot_cmd::run(ctx, &a),
        Cmd::Diagram(a) => diagram_cmd::run(ctx, &a),
        Cmd::Visualize(a) => visualize_cmd::run(ctx, &a),
        Cmd::Voice(a) => Ok(voice_cmd::run(ctx, &a)),
        Cmd::Perf(a) => perf_cmd::run(ctx, &a),
        Cmd::Replay(a) => replay_cmd::run(ctx, &a),
        Cmd::Env { command } => env_cmd::run(ctx, &command),
        Cmd::RemoteServe { project, worker } => {
            remote_cmd::serve(ctx, project.as_deref(), worker.as_deref())
        }
        Cmd::Attach { .. } => {
            eprintln!("{}", ctx.l.t("remote-attach-needs-remote"));
            Ok(false)
        }
        Cmd::Discover(a) => discover_cmd::run(ctx, &a, None),
        Cmd::Connections => Ok(extra::connections(ctx)),
        Cmd::Cost {
            days,
            export,
            command,
        } => Ok(cost_cmd::cost(ctx, days, export, command)),
        Cmd::Quota => cost_cmd::quota(ctx),
        Cmd::Context { history, command } => {
            Ok(memory_cmd::context(ctx, history, command.as_ref()))
        }
        Cmd::Bench(a) => bench_cmd::run(ctx, &a),
        Cmd::Shell { command } => Ok(shell_cmd::run(ctx, command)),
        Cmd::Fix { model } => shell_cmd::fix(ctx, model.as_deref(), false),
        Cmd::Why { model } => shell_cmd::fix(ctx, model.as_deref(), true),
        Cmd::Comment(a) => comment_cmd::run(ctx, &a),
        Cmd::Consensus(a) => consensus_cmd::run(ctx, &a),
        Cmd::Commit(a) => commit_cmd::commit(ctx, &a),
        Cmd::Changelog(a) => commit_cmd::changelog(ctx, &a),
        Cmd::Templates { command } => templates_cmd::run(ctx, &command),
        Cmd::Run { target, inputs } => templates_cmd::run_template(ctx, &target, &inputs),
        Cmd::Routines { command } => Ok(extra::routines(ctx, command)),
        Cmd::Update { from, command } => Ok(update::run(ctx, from.as_deref(), command)),
        Cmd::Component { command } => Ok(component_cmd::run(ctx, command)),
        Cmd::Undo { list, session } => Ok(undo::run(ctx, list, session.as_deref())),
        Cmd::ChromeNativeHost => chrome::run_stdio().map(|()| true).map_err(Into::into),
        Cmd::Mcp { command } => mcp_cmd::run(ctx, command),
        Cmd::Plugin { command } => Ok(plugin_cmd(&plugin_root(), command)),
        Cmd::Auth { command } => auth::run(ctx, command),
        Cmd::Config { command } => Ok(config_cmd(ctx, &command)),
        Cmd::Providers { command: None } => Ok(providers::list_providers(ctx)),
        Cmd::Providers { command: Some(c) } => Ok(privacy_cmd::run(ctx, &c)),
        Cmd::Provider { command } => provider_cmd::run(ctx, command),
        Cmd::Proxy { command } => proxy_cmd::run(ctx, command),
        Cmd::Models {
            provider,
            refresh,
            offline,
        } => providers::list_models(ctx, provider.as_deref(), refresh, offline),
        Cmd::Chat {
            model,
            system,
            max_tokens,
            prompt,
        } => providers::chat(
            ctx,
            providers::ChatArgs {
                model,
                prompt: prompt.join(" "),
                system,
                max_tokens,
            },
        ),
    }
}

/// `/config explain` on the interactive screen: the same command as `kor config explain`.
/// Interactive and process-level commands (`resume`, the Chrome host, `mcp serve`) are refused.
pub(crate) fn run_slash(ctx: &Ctx, line: &str) -> Option<bool> {
    let words = split_words(line.trim_start_matches('/'))?;
    let argv = std::iter::once("kor".to_owned()).chain(words);
    let cli = Cli::try_parse_from(argv).ok()?;
    let cmd = cli.command?;
    if matches!(&cmd, Cmd::Peer { command } if peer_cmd::blocks_tui(command))
        || matches!(&cmd, Cmd::Acp(a) if acp_cmd::serves_stdio(a))
    {
        return None;
    }
    if matches!(
        cmd,
        Cmd::Resume { .. }
            | Cmd::Run { .. }
            | Cmd::Tui { .. }
            | Cmd::ChromeNativeHost
            | Cmd::Status(status_cmd::StatusArgs { watch: true, .. })
            | Cmd::Approvals(status_cmd::ApprovalsArgs {
                list: false,
                json: false
            })
            | Cmd::Mcp {
                command: Some(mcp_cmd::McpCmd::Serve { .. } | mcp_cmd::McpCmd::Run { .. })
            }
            | Cmd::RemoteServe { .. }
            | Cmd::Attach { .. }
            | Cmd::Daemon {
                command: daemon_cmd::DaemonCmd::Run { .. }
                    | daemon_cmd::DaemonCmd::Logs { follow: true, .. }
            }
            | Cmd::Proxy {
                command: proxy_cmd::ProxyCmd::Serve {
                    background: false,
                    ..
                }
            }
    ) {
        return None;
    }
    // Commands may start their own async runtime; run them off the interactive screen's runtime.
    let result = std::thread::scope(|s| s.spawn(|| run_cmd(ctx, cmd)).join());
    Some(match result {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => {
            eprintln!(
                "{}",
                ctx.l
                    .t_args("error-generic", &args!("error" => format!("{e:#}")))
            );
            false
        }
        Err(_) => false,
    })
}

/// Shell-like words: whitespace separated, `"…"` and `'…'` keep spaces. `None` on an open quote.
fn split_words(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Some(out)
}

fn config_cmd(ctx: &Ctx, cmd: &ConfigCmd) -> bool {
    let l = &ctx.l;
    match cmd {
        ConfigCmd::Set { key, value } => {
            let Some(file) = ctx.paths.user.clone() else {
                eprintln!("{}", l.t("config-set-no-user-dir"));
                return false;
            };
            match kor_config::set_scalar(&file, key, value) {
                Ok(()) => {
                    // §28.14: every settings change goes to the audit log.
                    kor_runtime::prefs::log_config_change(key, value, &file);
                    println!(
                        "{}",
                        l.t_args(
                            "config-set-done",
                            &args!("key" => key.clone(), "value" => value.clone(), "path" => file.display().to_string())
                        )
                    );
                    true
                }
                Err(e) => {
                    eprintln!("{}", config_error(l, &e));
                    false
                }
            }
        }
        ConfigCmd::Paths => {
            let show = |p: &Option<std::path::PathBuf>| {
                p.as_ref()
                    .map_or_else(|| l.t("config-paths-none"), |p| p.display().to_string())
            };
            println!(
                "{}",
                l.t_args(
                    "config-paths-system",
                    &args!("path" => show(&ctx.paths.system))
                )
            );
            println!(
                "{}",
                l.t_args("config-paths-user", &args!("path" => show(&ctx.paths.user)))
            );
            println!(
                "{}",
                l.t_args(
                    "config-paths-project",
                    &args!("path" => show(&ctx.paths.project))
                )
            );
            true
        }
        ConfigCmd::Explain { section: Some(s) } => prefs_cmd::explain(ctx, s),
        ConfigCmd::Explain { section: None } => {
            let loaded = match &ctx.config {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{}", config_error(l, e));
                    return false;
                }
            };
            println!("{}", l.t("config-explain-header"));
            for (key, s) in &loaded.settings {
                let source = match s.layer {
                    Layer::Default => l.t("config-source-default"),
                    Layer::Env => {
                        l.t_args("config-source-env", &args!("origin" => s.origin.clone()))
                    }
                    Layer::Cli => l.t("config-source-cli"),
                    Layer::System | Layer::User | Layer::Project => {
                        l.t_args("config-source-file", &args!("origin" => s.origin.clone()))
                    }
                };
                println!(
                    "  {}",
                    l.t_args(
                        "config-explain-line",
                        &args!("key" => key.clone(), "value" => s.value.to_string(), "source" => source)
                    )
                );
            }
            true
        }
    }
}

/// Kor's own plugin directory (`~/.kor/plugins`, `KOR_PLUGIN_CACHE_DIR`), shared with the desktop.
/// Never another tool's directory.
fn plugin_root() -> std::path::PathBuf {
    std::env::var_os("KOR_PLUGIN_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| kor_agent::plugins::plugins_root(&h)))
        .unwrap_or_else(|| std::path::PathBuf::from(".kor-plugins"))
}

fn split_plugin(spec: &str) -> Option<(&str, &str)> {
    spec.split_once('@')
}

fn plugin_cmd(root: &std::path::Path, cmd: PluginCmd) -> bool {
    let report = |r: Result<(), String>| match r {
        Ok(()) => true,
        Err(e) => {
            eprintln!("{e}");
            false
        }
    };
    match cmd {
        PluginCmd::Marketplace { command } => match command {
            MarketCmd::Add { name, source } => {
                kor_agent::plugins::add_marketplace(root, &name, &source)
                    .map(|_path| ())
                    .map_or_else(
                        |e| {
                            eprintln!("{e}");
                            false
                        },
                        |()| true,
                    )
            }
            MarketCmd::List => {
                for m in kor_agent::plugins::list_marketplaces(root) {
                    println!("{}  {}", m.name, m.plugins.join(", "));
                }
                true
            }
            MarketCmd::Remove { name } => {
                report(kor_agent::plugins::remove_marketplace(root, &name))
            }
        },
        PluginCmd::Install { plugin } => {
            let Some((name, market)) = split_plugin(&plugin) else {
                eprintln!("use name@marketplace");
                return false;
            };
            kor_agent::plugins::install(root, market, name)
                .map(|_path| ())
                .map_or_else(
                    |e| {
                        eprintln!("{e}");
                        false
                    },
                    |()| true,
                )
        }
        PluginCmd::Uninstall { plugin } => {
            let Some((name, market)) = split_plugin(&plugin) else {
                eprintln!("use name@marketplace");
                return false;
            };
            report(kor_agent::plugins::remove(root, market, name))
        }
        PluginCmd::List => {
            println!("{}", root.display());
            true
        }
        PluginCmd::Enable { plugin } | PluginCmd::Update { plugin } => {
            report(kor_agent::plugins::set_enabled(root, &plugin, true))
        }
        PluginCmd::Disable { plugin } => {
            report(kor_agent::plugins::set_enabled(root, &plugin, false))
        }
        PluginCmd::Doctor { plugin } => {
            let path = root.join("cache").join(plugin.replace('@', "/"));
            report(kor_agent::plugins::doctor(&path))
        }
    }
}

/// Configured MCP servers from every source the agent loads (see `kor_agent::mcp_tools`).
pub(crate) fn print_mcp_servers(root: &std::path::Path, trusted: bool) {
    use kor_agent::mcp_tools::{Skip, Sources, discover};
    let home = dirs::home_dir();
    let kor_dir = kor_config::user_dir();
    let (servers, skipped) = discover(
        &Sources {
            project_root: root,
            project_trusted: trusted,
            home: home.as_deref(),
            kor_config_dir: kor_dir.as_deref(),
        },
        &|k| std::env::var(k).ok(),
    );
    if servers.is_empty() && skipped.is_empty() {
        println!("no MCP servers configured");
    }
    for s in servers {
        println!(
            "{}  {} {}  ({})",
            s.name,
            s.command,
            s.args.join(" "),
            s.origin
        );
    }
    for skip in skipped {
        match skip {
            Skip::Untrusted { count } => {
                println!(
                    "{count} server(s) in .mcp.json skipped: project not trusted (`kor trust`)"
                );
            }
            Skip::Remote { name, origin } => {
                println!("{name}  remote (HTTP/SSE), not supported yet  ({origin})");
            }
            Skip::Invalid {
                name,
                origin,
                reason,
            } => println!("{name}  {reason}  ({origin})"),
        }
    }
}

pub(crate) fn config_error(l: &Localizer, e: &ConfigError) -> String {
    l.t_args(
        "config-invalid",
        &args!("origin" => e.origin().to_owned(), "error" => e.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_runs_cli_commands_and_refuses_interactive_ones() {
        assert_eq!(
            split_words(r#"config set agent.model "venice/x y" 'a b'"#).unwrap(),
            ["config", "set", "agent.model", "venice/x y", "a b"]
        );
        assert!(split_words("config \"open").is_none());
        let ctx = Ctx {
            l: Localizer::new(Lang::En),
            paths: Paths {
                system: None,
                user: None,
                project: None,
            },
            config: kor_config::load(
                &Paths {
                    system: None,
                    user: None,
                    project: None,
                },
                std::iter::empty(),
                &[],
            ),
        };
        assert_eq!(run_slash(&ctx, "/config paths"), Some(true));
        assert_eq!(
            run_slash(&ctx, "/resume"),
            None,
            "interactive: not via slash"
        );
        assert_eq!(run_slash(&ctx, "/mcp serve"), None);
        assert_eq!(run_slash(&ctx, "/no-such-command"), None);
    }

    /// `parity.toml` (repo root) must name every CLI command, and open gaps may only go down.
    #[test]
    fn parity_manifest_covers_every_command_and_gaps_do_not_grow() {
        fn paths(c: &clap::Command, prefix: &str, out: &mut Vec<String>) {
            for s in c.get_subcommands().filter(|s| s.get_name() != "help") {
                let p = format!("{prefix} {}", s.get_name());
                paths(s, &p, out);
                // A pure group (`kor auth`) is not a feature; its subcommands are.
                if !s.is_subcommand_required_set() {
                    out.push(p);
                }
            }
        }
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../parity.toml");
        let text = std::fs::read_to_string(&file).expect("parity.toml at the repository root");
        let doc: toml::Table = text.parse().expect("valid parity.toml");
        let features = doc["feature"].as_array().expect("[[feature]] entries");
        let field =
            |f: &toml::Value, k: &str| f.get(k).and_then(toml::Value::as_str).map(str::to_owned);
        let cli: Vec<String> = features.iter().filter_map(|f| field(f, "cli")).collect();

        let mut commands = Vec::new();
        paths(&Cli::command(), "kor", &mut commands);
        let missing: Vec<&String> = commands.iter().filter(|c| !cli.contains(c)).collect();
        assert!(
            missing.is_empty(),
            "add these commands to parity.toml: {missing:?}"
        );

        let gaps = features
            .iter()
            .filter(|f| f.get("internal").and_then(toml::Value::as_bool) != Some(true))
            .map(|f| {
                ["cli", "tui", "desktop"]
                    .iter()
                    .filter(|k| field(f, k).is_none_or(|v| v.trim() == "-"))
                    .count()
            })
            .sum::<usize>();
        let allowed = doc["meta"]["allowed_gaps"].as_integer().unwrap_or(0);
        assert!(
            i64::try_from(gaps).unwrap_or(i64::MAX) <= allowed,
            "parity gaps grew: {gaps} > allowed {allowed}; add the missing surface instead"
        );
        assert_eq!(
            i64::try_from(gaps).unwrap_or(i64::MAX),
            allowed,
            "parity gaps went down to {gaps}: lower `allowed_gaps` in parity.toml"
        );
    }

    #[test]
    fn prescan_finds_both_forms_and_stops_at_double_dash() {
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(prescan_lang(&v(&["kor", "--lang", "tr"])), Some(Lang::Tr));
        assert_eq!(
            prescan_lang(&v(&["kor", "doctor", "--lang=fr"])),
            Some(Lang::Fr)
        );
        assert_eq!(prescan_lang(&v(&["kor", "--", "--lang", "tr"])), None);
        assert_eq!(prescan_lang(&v(&["kor", "--lang", "de"])), None);
    }

    #[test]
    fn plugin_marketplace_round_trip_uses_a_local_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        let plug = origin.join("demo");
        std::fs::create_dir_all(plug.join(".claude-plugin")).unwrap();
        std::fs::write(
            origin.join("marketplace.json"),
            r#"{"name":"local","plugins":[{"name":"demo"}]}"#,
        )
        .unwrap();
        std::fs::write(
            plug.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"demo"}"#,
        )
        .unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&origin)
                    .env("GIT_AUTHOR_NAME", "kor")
                    .env("GIT_AUTHOR_EMAIL", "kor@example.com")
                    .env("GIT_COMMITTER_NAME", "kor")
                    .env("GIT_COMMITTER_EMAIL", "kor@example.com")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init"]);
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);
        let cache = dir.path().join("plugins");
        assert!(plugin_cmd(
            &cache,
            PluginCmd::Marketplace {
                command: MarketCmd::Add {
                    name: "local".into(),
                    source: origin.display().to_string(),
                },
            }
        ));
        assert!(plugin_cmd(
            &cache,
            PluginCmd::Install {
                plugin: "demo@local".into(),
            }
        ));
        assert!(
            cache
                .join("cache")
                .join("local")
                .join("demo")
                .join(".claude-plugin")
                .join("plugin.json")
                .is_file()
        );
        assert!(plugin_cmd(
            &cache,
            PluginCmd::Uninstall {
                plugin: "demo@local".into(),
            }
        ));
        assert!(plugin_cmd(
            &cache,
            PluginCmd::Marketplace {
                command: MarketCmd::Remove {
                    name: "local".into(),
                },
            }
        ));
    }

    #[test]
    fn clap_definition_is_valid_in_every_language() {
        for lang in Lang::ALL {
            localized_command(&Localizer::new(lang)).debug_assert();
        }
    }
}
