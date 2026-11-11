//! Agent session assembly shared by the `kor` CLI and the desktop app.
//!
//! Everything here used to live in `kor-cli`. Functions return localized messages instead of
//! printing them: the CLI writes `notes` and errors to stderr in order, the desktop app shows them
//! in the conversation. Behavior (trust rules, permission fallbacks, key handling) is unchanged.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_agent::{Agent, HookRunner, Permissions, ScopedRule, SkillTool, memory, prompt, session};
use kor_compat::{Compat, CustomProvider, LoadOptions};
use kor_core::{Message, ModelRef, PermissionMode, ProviderError, Usage};
use kor_i18n::{Localizer, args};
use kor_providers::{Provider, ProviderManifest, Registry};
use kor_sandbox::SandboxKind;
use kor_secrets::{KeySource, KeychainStore, Redactor, SecretString, resolve_key};
use kor_tools::{Shell, ToolCtx, default_tools};

pub mod acp;
pub mod analysis;
pub mod bench;
// §32 performance contract: budgets, bench history, perf.log, low-power mode (ADR 0062).
pub mod budgets;
pub mod context;
pub mod cost;
pub mod dbops;
pub mod deploy;
pub mod devenv;
// E2E scenarios: mock provider, isolated home, checker (§16.4, §17.2, ADR 0061).
pub mod e2e;
pub mod forge;
// Conversation forks, line diffs (§27.3, ADR 0063).
pub mod fork;
pub mod textdiff;
// Live diagrams from the graph (§27.14, ADR 0063).
pub mod diagram_render;
pub mod diagram_sync;
// Time and cost estimate before a task (§27.16, ADR 0063).
pub mod estimate;
// Rule compliance: rule sets, `kor rules check`, model judgment (§28.3, ADR 0063).
pub mod compliance;
// Browser extension side-panel assistants: docs reader, form filler, page → code (§27.11–13, ADR 0058).
pub mod browser_assist;
// Private documents for the agent: `kor docs add` (§28.7, ADR 0063).
pub mod docs;
// `kor export --all` (§22.2): all user data in one zip.
pub mod comment;
pub mod consensus;
pub mod export;
// Hook editor backend (§28.12, ADR 0063).
pub mod hooks_edit;
// MCP developer mode: `kor mcp dev` (§28.15, ADR 0063).
pub mod mcp_dev;
// PATH tool catalog and typed tool definitions in a session (§26.2, ADR 0056).
pub mod clitools;
// Natural-language command routing (§30, ADR 0056).
pub mod intent;
pub mod offline;
// Kor-to-Kor bridge (§31.1, ADR 0057).
pub mod peer;
// OS integration (ADR 0052): desktop argv / `kor://` links, `~/Kor/` folder, batch file edits.
pub mod autointegrate;
pub mod batch;
pub mod korfolder;
pub mod media;
pub mod osint;
pub mod perf;
// Tray, notifications, approval queue, ETA, "call me", window layouts (ADR 0051).
pub mod builtin_skills;
pub mod prefs;
pub mod presence;
pub mod privacycard;
pub mod quota;
pub mod remote;
pub mod replay;
pub mod review;
pub mod routines;
pub mod sandbox;
pub mod secure;
pub mod session_import;
pub mod session_tree;
pub mod shell;
pub mod task;
pub mod triggers;
pub mod update;
pub mod usage;
pub mod visual;
pub mod watch;

/// Which transcript to continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    Latest,
    Id(String),
}

/// Provider, parsed model and the key it was built with (registered with the redactor).
pub type Built = (Arc<dyn Provider>, ModelRef, Option<SecretString>);

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

/// Transcripts, shared by `kor resume` and the desktop app.
pub fn sessions_dir() -> Option<PathBuf> {
    // `<kor home>/sessions`: `%LOCALAPPDATA%\kor\sessions` unless `KOR_HOME` moves Kor's data (ADR 0061).
    kor_home().map(|d| d.join("sessions"))
}

static PRIVACY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Privacy mode of the running session (`kor --privacy-mode`, config `privacy.mode`); offline
/// mode implies it.
pub fn privacy_mode() -> bool {
    PRIVACY.load(std::sync::atomic::Ordering::Relaxed) || offline_mode()
}

/// Offline mode (§6.5, ADR 0035): `kor --offline`, `KOR_OFFLINE=1` or config `[offline] enabled`.
/// Everything privacy mode turns off, and no connection beyond this computer (loopback only).
pub fn offline_mode() -> bool {
    OFFLINE.load(std::sync::atomic::Ordering::Relaxed) || offline_env()
}

fn offline_env() -> bool {
    std::env::var("KOR_OFFLINE").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Offline mode: agent commands (git, npm, pip, curl, …) send outbound HTTP(S) to a closed
/// loopback port, so they fail at once instead of reaching the internet. Best effort for
/// programs that honor proxy variables; the sandbox's network rule is the hard boundary.
pub const OFFLINE_COMMAND_ENV: &[(&str, &str)] = &[
    ("HTTP_PROXY", "http://127.0.0.1:9"),
    ("HTTPS_PROXY", "http://127.0.0.1:9"),
    ("ALL_PROXY", "http://127.0.0.1:9"),
    ("http_proxy", "http://127.0.0.1:9"),
    ("https_proxy", "http://127.0.0.1:9"),
    ("all_proxy", "http://127.0.0.1:9"),
    ("NO_PROXY", "localhost,127.0.0.1,::1"),
    ("no_proxy", "localhost,127.0.0.1,::1"),
    ("KOR_OFFLINE", "1"),
];

/// Hosts of the configured local providers (built-in local category and the user's own
/// loopback/LAN manifests): the only non-loopback hosts privacy mode lets through.
fn local_provider_hosts(l: &Localizer) -> Vec<String> {
    registry(l, &mut Vec::new())
        .map(|r| {
            r.iter()
                .filter(|m| is_local_provider(m))
                .filter_map(|m| kor_core::net::url_host(&m.base_url))
                .collect()
        })
        .unwrap_or_default()
}

/// Set the process-wide network policy (privacy, offline, the network guard of every HTTP
/// client) from `config`. Called by [`prepare`]; the CLI calls it after loading the config and
/// the desktop app at start and whenever privacy/offline is toggled.
pub fn apply_network_policy(l: &Localizer, config: &kor_config::Config) {
    use std::sync::atomic::Ordering::Relaxed;
    let offline = config.offline.enabled || offline_env();
    let privacy = offline || config.privacy.mode;
    OFFLINE.store(offline, Relaxed);
    PRIVACY.store(privacy, Relaxed);
    REQUIRE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone_from(&config.privacy.require);
    kor_components::set_privacy(privacy);
    kor_core::net::set_messages(
        l.t_args("net-blocked-privacy", &args!("host" => "{host}")),
        l.t_args("net-blocked-offline", &args!("host" => "{host}")),
    );
    if privacy {
        kor_core::net::set_local_hosts(local_provider_hosts(l));
    }
    kor_core::net::set_policy(if offline {
        kor_core::net::Policy::Offline
    } else if privacy {
        kor_core::net::Policy::Privacy
    } else {
        kor_core::net::Policy::Open
    });
}

static REQUIRE: std::sync::RwLock<Vec<kor_core::PrivacyRequirement>> =
    std::sync::RwLock::new(Vec::new());

/// The §31.2 policy filter in force: `privacy.require`, pulled to the strictest set by privacy
/// mode (ADR 0057). Set by [`apply_network_policy`].
pub fn privacy_requirements() -> Vec<kor_core::PrivacyRequirement> {
    let configured = REQUIRE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    kor_providers::privacy_card::effective_requirements(&configured, privacy_mode())
}

/// The model's privacy card meets every requirement in force; else the localized reason.
pub fn check_privacy_card(l: &Localizer, m: &ProviderManifest, model: &str) -> Result<(), String> {
    let req = privacy_requirements();
    if req.is_empty() {
        return Ok(());
    }
    let unmet = kor_providers::privacy_card::card(m, Some(model)).unmet(&req);
    if unmet.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = unmet.iter().map(|r| r.as_str()).collect();
    Err(l.t_args(
        "privcard-refused",
        &args!("model" => format!("{}/{model}", m.id), "unmet" => list.join(", ")),
    ))
}

/// Privacy mode refuses a provider that is not local, offline mode one that is not on this
/// computer; both before any connection.
pub fn check_provider_allowed(l: &Localizer, m: &ProviderManifest) -> Result<(), String> {
    if privacy_mode() && !is_local_provider(m) {
        return Err(l.t_args(
            "privacy-remote-provider",
            &args!("provider" => m.id.clone()),
        ));
    }
    if m.base_url.is_empty() {
        return Ok(());
    }
    kor_core::net::check_url(&m.base_url).map_err(|b| b.message)
}

/// Runs on this machine: the `local` category or a loopback endpoint (custom local servers).
pub fn is_local_provider(m: &ProviderManifest) -> bool {
    if m.category == kor_providers::Category::Local {
        return true;
    }
    reqwest_host(&m.base_url).is_some_and(|h| {
        h == "localhost" || h.ends_with(".localhost") || h == "[::1]" || h.starts_with("127.")
    })
}

fn reqwest_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split_once(']').map(|(h, _)| format!("{h}]"))?
    } else {
        host.split(':').next()?.to_owned()
    };
    Some(host.to_lowercase())
}

/// Tools that reach the network themselves; removed in privacy mode.
fn needs_network(name: &str) -> bool {
    matches!(
        name,
        "WebFetch" | "WebSearch" | "weather" | "search_places" | "file_search"
    ) || ["browser_", "chrome_", "ssh_", "db_", "mcp__", "web_"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Agent tools from crates that depend on this one (kor-web, §29), registered once at start-up.
/// Builds the extra tools of one session.
pub type ToolsFn = fn() -> Vec<Arc<dyn kor_tools::Tool>>;

static EXTRA_TOOLS: std::sync::OnceLock<ToolsFn> = std::sync::OnceLock::new();

/// Add tools to every agent session (`kor_web::site::tools::all`). The first call wins.
pub fn register_extra_tools(f: ToolsFn) {
    let _ = EXTRA_TOOLS.set(f);
}

/// Undo journals (`kor undo`), one directory per session id.
pub fn undo_dir() -> Option<PathBuf> {
    kor_home().map(|d| d.join("undo"))
}

pub fn new_session_path() -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(format!("{}.jsonl", session::new_id())))
}

/// Session ids are file stems; only `[A-Za-z0-9-]` so a path cannot escape the directory.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn trust_store() -> Option<PathBuf> {
    kor_home().map(|d| d.join("trusted.json"))
}

pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

pub fn project_root() -> std::io::Result<PathBuf> {
    std::env::current_dir()?.canonicalize()
}

pub fn is_trusted(root: &Path) -> bool {
    trust_store().is_some_and(|s| kor_compat::trust::is_trusted(&s, root))
}

/// A Venice client when a Venice key is configured (media, catalog, account limits).
pub fn venice_client(l: &Localizer) -> Option<kor_providers::media::VeniceMedia> {
    let mut ignored = Vec::new();
    let reg = registry(l, &mut ignored).ok()?;
    let venice = reg.get("venice")?;
    let (key, _) = key_for(venice).ok()??;
    kor_providers::media::VeniceMedia::new(venice, key).ok()
}

/// A fal.ai client when a fal key is configured (media queue, model search), with its key.
pub fn fal_client(l: &Localizer) -> Option<(kor_providers::fal::Fal, SecretString)> {
    let reg = registry(l, &mut Vec::new()).ok()?;
    let m = reg.get("fal")?;
    let (key, _) = key_for(m).ok()??;
    Some((kor_providers::fal::Fal::new(m, key.clone()).ok()?, key))
}

/// A Hugging Face client (Hub works without a token; inference needs one) and its token.
pub fn hf_client(
    l: &Localizer,
    endpoint: Option<&str>,
) -> Result<(kor_providers::huggingface::Hf, Option<SecretString>), String> {
    let reg = registry(l, &mut Vec::new())?;
    let m = reg
        .get("huggingface")
        .ok_or("huggingface manifest missing")?;
    let key = key_for(m).ok().flatten().map(|(k, _)| k);
    let hf = match endpoint {
        Some(url) => kor_providers::huggingface::Hf::endpoint(m, key.clone(), url),
        None => kor_providers::huggingface::Hf::new(m, key.clone()),
    };
    hf.map(|h| (h, key)).map_err(|e| e.message)
}

/// Claude Code's config home. Kor only reads it (a one-time memory import), never writes it.
pub fn claude_base() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

/// Kor's own home for auto memory, session notes and search indexes: `KOR_HOME`, else
/// `%LOCALAPPDATA%\kor` (next to the sessions).
pub fn kor_home() -> Option<PathBuf> {
    std::env::var_os("KOR_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|d| d.join("kor")))
}

/// §19 (ADR 0059): per-app desktop tiers, computer actions into the audit log, `[health]` for the
/// write stop, the CLI's "Kor controls the desktop" line, and one health line on stderr when a
/// terminal session starts.
fn computer_and_health_setup(
    config: &kor_config::Config,
    lang: kor_i18n::Lang,
    surface: prompt::Surface,
) {
    use std::io::IsTerminal as _;
    kor_computer::policy::set_policy(kor_computer::policy::Policy::from_config(&config.computer));
    kor_computer::set_audit_sink(Arc::new(|action, fields| {
        kor_agent::audit::record(action, fields);
    }));
    kor_health::set_config(config.health.clone());
    if surface == prompt::Surface::Terminal {
        let l = Arc::new(Localizer::new(lang));
        kor_computer::indicator::set_listener_if_none(kor_computer::indicator::stderr_listener(
            l.clone(),
        ));
        if std::io::stderr().is_terminal()
            && let Ok(cwd) = std::env::current_dir()
            && let Some(line) = kor_health::quick_line(&config.health, &cwd, &l)
        {
            eprintln!("{line}");
        }
    }
}

/// Local JSON-lines log of per-request context breakdowns (§5.3, `kor context`).
pub fn context_log_path() -> Option<PathBuf> {
    kor_home().map(|h| h.join("context.jsonl"))
}

/// Context window of `model`: the model catalog cache (no network), else the provider's
/// default, else [`kor_agent::compact::DEFAULT_CONTEXT_WINDOW`].
pub fn context_window_for(model: &ModelRef, manifest: &ProviderManifest) -> u64 {
    let cached = dirs::cache_dir().and_then(|d| {
        kor_providers::model_catalog::read_cache(&d.join("kor").join("model-catalog.json"))
    });
    cached
        .and_then(|c| {
            let models = &c.catalog.models;
            kor_providers::model_catalog::lookup(models, &model.to_string())
                .or_else(|| kor_providers::model_catalog::lookup(models, model.model()))
                .and_then(|m| m.context)
        })
        .or(manifest.max_context_default)
        .filter(|w| *w > 0)
        .unwrap_or(kor_agent::compact::DEFAULT_CONTEXT_WINDOW)
}

/// `[context] summarize_model` as a provider; in privacy mode only a local one.
fn summarizer(
    l: &Localizer,
    spec: &str,
    privacy: bool,
    redactor: &mut Redactor,
    notes: &mut Vec<String>,
) -> Option<(Arc<dyn Provider>, String)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    match build_provider(l, spec, notes) {
        Ok((p, m, _))
            if (privacy && !is_local_provider(p.manifest()))
                || check_provider_allowed(l, p.manifest()).is_err()
                || check_privacy_card(l, p.manifest(), m.model()).is_err() =>
        {
            notes.push(l.t_args(
                "context-summarizer-privacy",
                &args!("model" => spec.to_owned()),
            ));
            None
        }
        Ok((p, m, key)) => {
            if let Some(k) = key {
                redactor.register(k);
            }
            Some((p, m.model().to_owned()))
        }
        Err(e) => {
            notes.push(e);
            None
        }
    }
}

/// Copy `from` into the missing `to` (files and subfolders, no links), so memories written
/// before Kor had its own home stay available. Returns how many files were copied.
fn import_dir(from: &Path, to: &Path, depth: u8) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let ty = e.file_type()?;
        let dst = to.join(e.file_name());
        if ty.is_file() && !dst.exists() {
            std::fs::copy(e.path(), &dst)?;
            n += 1;
        } else if ty.is_dir() && depth > 0 {
            n += import_dir(&e.path(), &dst, depth - 1)?;
        }
    }
    Ok(n)
}

/// The project's memory layout under [`kor_home`], imported once from Claude Code's.
pub fn memory_layout(root: &Path) -> Option<memory::Layout> {
    let lay = memory::layout(&kor_home()?, root);
    if !lay.auto_dir.exists()
        && let Some(claude) = claude_base()
    {
        let old = memory::layout(&claude, root);
        if old.auto_dir.is_dir() {
            let _ = import_dir(&old.auto_dir, &lay.auto_dir, 2);
        }
    }
    Some(lay)
}

// ---------------------------------------------------------------------------------------------
// Compat
// ---------------------------------------------------------------------------------------------

pub fn load_compat(root: &Path) -> Compat {
    let trusted = is_trusted(root);
    let home = dirs::home_dir();
    let mut c = kor_compat::load(&LoadOptions {
        root: root.to_owned(),
        home: home.clone(),
        codex_home: codex_home(),
        trusted,
    });
    for dir in plugin_dirs(root, home.as_deref(), trusted) {
        kor_compat::add_plugin(&mut c, &dir);
    }
    // Skills shipped with Kor, last so the user's and the project's own skills win.
    if let Some(dir) = kor_home().and_then(|h| builtin_skills::ensure(&h)) {
        kor_compat::add_plugin(&mut c, &dir);
    }
    // §15.1: Kor's own `hooks.toml` (global, and the project's when trusted).
    kor_agent::hooks::add_kor_hooks(&mut c, root, kor_platform::config_dir().as_deref());
    c
}

/// Enabled plugins: the user's (`~/.kor/plugins`) and, in a trusted project, `<root>/.kor/plugins`.
pub fn plugin_dirs(root: &Path, home: Option<&Path>, trusted: bool) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if trusted {
        dirs.extend(kor_agent::plugins::enabled_dirs(
            &root.join(".kor").join("plugins"),
        ));
    }
    if let Some(home) = home {
        dirs.extend(kor_agent::plugins::enabled_dirs(
            &kor_agent::plugins::plugins_root(home),
        ));
    }
    dirs
}

/// A Codex `[model_providers.<id>]` entry as a Kor manifest (validated like any other).
pub fn manifest_from_codex(p: &CustomProvider) -> Result<ProviderManifest, String> {
    let mut t = toml::Table::new();
    t.insert("id".into(), p.id.clone().into());
    t.insert("name".into(), p.name.clone().into());
    t.insert("category".into(), "standard".into());
    let adapter = if p.wire_api == "responses" {
        "openai"
    } else {
        "openai_compat"
    };
    t.insert("adapter".into(), adapter.into());
    t.insert("base_url".into(), p.base_url.clone().into());
    let mut auth = toml::Table::new();
    match &p.env_key {
        Some(env) => {
            auth.insert("kind".into(), "bearer".into());
            auth.insert("env".into(), env.clone().into());
        }
        None => {
            auth.insert("kind".into(), "none".into());
        }
    }
    t.insert("auth".into(), auth.into());
    t.insert("tool_calling".into(), true.into());
    let mut privacy = toml::Table::new();
    privacy.insert("tier".into(), "unknown".into());
    privacy.insert(
        "retention".into(),
        "Declared in ~/.codex/config.toml; not reviewed by Kor.".into(),
    );
    t.insert("privacy".into(), privacy.into());
    ProviderManifest::parse(
        &toml::to_string(&t).map_err(|e| e.to_string())?,
        &p.source.path.display().to_string(),
    )
    .map_err(|e| e.to_string())
}

/// Add Codex user-level providers to `reg` without overriding existing ids.
pub fn add_codex_providers(reg: &mut Registry) -> Vec<String> {
    let Some(home) = codex_home() else {
        return Vec::new();
    };
    let (providers, mut warnings) = kor_compat::codex_user_providers(&home);
    for p in providers {
        match manifest_from_codex(&p) {
            Ok(m) => {
                reg.add_if_absent(m);
            }
            Err(e) => warnings.push(e),
        }
    }
    warnings
}

/// Rule strings from compat, dropping (and reporting) ones Kor cannot parse.
pub fn rules(entries: &[kor_compat::RuleEntry], warnings: &mut Vec<String>) -> Vec<String> {
    entries
        .iter()
        .filter(|e| match kor_agent::Rule::parse(&e.rule) {
            Ok(_) => true,
            Err(err) => {
                warnings.push(format!("{}: {err}", e.source.path.display()));
                false
            }
        })
        .map(|e| e.rule.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------------------------

/// Built-in manifests plus the user's `<config>/providers/*.toml`, and Codex user providers.
///
/// Project directories are deliberately not consulted: a cloned repository must not be
/// able to point a provider's `base_url` elsewhere and receive the user's key.
/// Codex manifests that fail validation are skipped and reported in `notes`.
pub fn registry(l: &Localizer, notes: &mut Vec<String>) -> Result<Registry, String> {
    let mut r = Registry::builtin();
    if let Some(dir) = kor_config::user_dir()
        && let Err(e) = r.load_dir(&dir.join("providers"))
    {
        return Err(l.t_args(
            "manifest-invalid",
            &args!("origin" => e.origin.clone(), "error" => e.message.clone()),
        ));
    }
    for w in add_codex_providers(&mut r) {
        notes.push(l.t_args("error-generic", &args!("error" => w)));
    }
    Ok(r)
}

pub fn key_for(
    m: &ProviderManifest,
) -> Result<Option<(SecretString, KeySource)>, kor_secrets::SecretError> {
    if !m.needs_key() {
        return Ok(None);
    }
    Ok(resolve_key(
        &m.id,
        &m.key_env(),
        |v| std::env::var(v).ok(),
        &KeychainStore,
    )?
    .map(|k| (k.secret, k.source)))
}

pub fn provider_error(l: &Localizer, e: &ProviderError) -> String {
    let kind = serde_json::to_value(e.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    l.t_args(
        "provider-error",
        &args!("provider" => e.provider.clone(), "kind" => kind, "message" => e.message.clone()),
    )
}

/// Provider for `provider/model`, with its key from the environment or the OS keychain.
pub fn build_provider(
    l: &Localizer,
    model: &str,
    notes: &mut Vec<String>,
) -> Result<Built, String> {
    let Ok(model) = model.parse::<ModelRef>() else {
        return Err(l.t_args("chat-bad-model", &args!("model" => model.to_owned())));
    };
    let reg = registry(l, notes)?;
    let Some(m) = reg.get(model.provider()) else {
        return Err(l.t_args(
            "chat-unknown-provider",
            &args!("provider" => model.provider().to_owned()),
        ));
    };
    // Key pool (§4.5): env key, then keychain `provider:<id>`, `provider:<id>#2`…
    let keys = kor_providers::pool::keys_for(m, &KeychainStore)
        .map_err(|e| l.t_args("error-generic", &args!("error" => e.to_string())))?;
    let key = keys.first().cloned();
    if let Some(dir) = kor_platform::cache_dir() {
        kor_providers::health::set_store(dir.join("provider-health.json"));
        kor_providers::quota::set_store(dir.join("provider-quota.json"));
    }
    match kor_providers::build_pool(m, keys) {
        Ok(p) => Ok((p, model, key)),
        Err(e) => Err(provider_error(l, &e)),
    }
}

/// Shadow mode (§5.1, `[routing] shadow`): the cheap model answering finished tasks in the
/// background; `None` when off or when the model cannot be used (a note says why).
fn shadow_runner(
    l: &Localizer,
    config: &kor_config::Config,
    notes: &mut Vec<String>,
) -> Option<Arc<dyn kor_agent::shadow::Shadow>> {
    let m = config.routing.shadow.trim();
    if m.is_empty() {
        return None;
    }
    let built = build_provider(l, m, notes).and_then(|(p, model, _)| {
        check_provider_allowed(l, p.manifest())?;
        check_privacy_card(l, p.manifest(), model.model())?;
        Ok((p, model.model().to_owned()))
    });
    match (built, kor_home()) {
        (Ok((p, model)), Some(home)) => {
            let threshold = if config.routing.shadow_threshold == 0 {
                85
            } else {
                config.routing.shadow_threshold.min(100)
            };
            let min = if config.routing.shadow_min_samples == 0 {
                5
            } else {
                config.routing.shadow_min_samples
            };
            Some(Arc::new(consensus::ShadowRunner::new(
                p,
                model,
                home.join("shadow.jsonl"),
                threshold,
                min as usize,
                Localizer::new(l.lang()),
            )))
        }
        (Err(e), _) => {
            notes.push(format!("shadow: {e}"));
            None
        }
        (_, None) => None,
    }
}

/// `[routing] repair`: the cheap model for repairing malformed tool calls, under the same
/// provider and privacy checks as the main model; `None` when off or unusable (a note says why).
fn repair_model(
    l: &Localizer,
    config: &kor_config::Config,
    notes: &mut Vec<String>,
) -> Option<(Arc<dyn Provider>, String)> {
    let m = config.routing.repair.trim();
    if m.is_empty() {
        return None;
    }
    let built = build_provider(l, m, notes).and_then(|(p, model, _)| {
        check_provider_allowed(l, p.manifest())?;
        check_privacy_card(l, p.manifest(), model.model())?;
        Ok((p, model.model().to_owned()))
    });
    built.map_err(|e| notes.push(format!("repair: {e}"))).ok()
}

/// Provider and model id for one request outside a session (`kor fix`, `kor commit`): `model`,
/// else `agent.model`. Privacy mode (config or `--privacy-mode`) refuses non-local providers.
pub fn oneshot_provider(
    l: &Localizer,
    loaded: &kor_config::Loaded,
    model: Option<&str>,
) -> Result<(Arc<dyn Provider>, String), String> {
    let model = model
        .map(str::to_owned)
        .or_else(|| {
            let m = loaded.config.agent.model.trim();
            (!m.is_empty()).then(|| m.to_owned())
        })
        .ok_or_else(|| l.t("no-model"))?;
    apply_network_policy(l, &loaded.config);
    kor_agent::cost::set_usage_sink(usage::record_oneshot);
    let (provider, model, _) = build_provider(l, &model, &mut Vec::new())?;
    check_provider_allowed(l, provider.manifest())?;
    check_privacy_card(l, provider.manifest(), model.model())?;
    Ok((provider, model.model().to_owned()))
}

fn tool_mode_for(
    cfg: kor_config::ToolCalling,
    provider_tools: bool,
) -> kor_agent::toolcall::ToolMode {
    match cfg {
        kor_config::ToolCalling::Native => kor_agent::toolcall::ToolMode::Native,
        kor_config::ToolCalling::Text => kor_agent::toolcall::ToolMode::Text,
        kor_config::ToolCalling::Auto => {
            kor_agent::toolcall::ToolMode::for_provider(provider_tools)
        }
    }
}

/// Router, ledger and budget for a session on `provider`/`model` (§5.1). Every model named in
/// `[routing]` is built up front (no network); in privacy mode only local ones are kept.
pub fn cost_control(
    l: &Localizer,
    config: &kor_config::Config,
    provider: &Arc<dyn Provider>,
    model: &str,
    privacy: bool,
    notes: &mut Vec<String>,
) -> Arc<cost::Control> {
    let r = &config.routing;
    let mut refs: Vec<&str> = [
        &r.simple_edit,
        &r.complex_refactor,
        &r.test_generation,
        &r.docs,
        &r.review,
        &r.planning,
        &r.verify_with,
    ]
    .into_iter()
    .map(|s| s.trim())
    .chain(r.fallback.iter().map(|s| s.trim()))
    .filter(|s| !s.is_empty())
    .collect();
    refs.sort_unstable();
    refs.dedup();
    let mut others = Vec::new();
    if r.enabled || config.budget.on_limit == kor_config::OnLimit::Downgrade {
        for m in refs {
            match build_provider(l, m, notes) {
                Ok((p, mref, _))
                    if (!privacy || is_local_provider(p.manifest()))
                        && check_provider_allowed(l, p.manifest()).is_ok() =>
                {
                    // §31.2: the router skips models whose privacy card misses a requirement.
                    if let Err(e) = check_privacy_card(l, p.manifest(), mref.model()) {
                        notes.push(e);
                        continue;
                    }
                    let p = with_routing_fallback(l, p, &r.fallback, privacy, notes);
                    others.push(cost::Target {
                        tool_mode: tool_mode_for(config.agent.tool_mode, p.manifest().tool_calling),
                        provider: p,
                        model: mref.model().to_owned(),
                    });
                }
                Ok(_) => {}
                Err(e) => notes.push(e),
            }
        }
    }
    let default = cost::Target {
        provider: provider.clone(),
        model: model.to_owned(),
        tool_mode: tool_mode_for(config.agent.tool_mode, provider.manifest().tool_calling),
    };
    Arc::new(cost::Control::new(
        l.lang(),
        r.clone(),
        config.budget.clone(),
        default,
        others,
        cost::Prices::load(),
        cost::Files::default_paths(),
    ))
}

/// `[routing] fallback` models behind `primary` (§4.5). In privacy mode only local fallbacks
/// are kept; unusable entries are reported in `notes` and skipped.
pub fn with_routing_fallback(
    l: &Localizer,
    primary: Arc<dyn Provider>,
    fallback: &[String],
    privacy: bool,
    notes: &mut Vec<String>,
) -> Arc<dyn Provider> {
    let mut chain = Vec::new();
    for f in fallback {
        let mut sub = Vec::new();
        match build_provider(l, f, &mut sub) {
            Ok((p, mref, _))
                if (!privacy || is_local_provider(p.manifest()))
                    && check_provider_allowed(l, p.manifest()).is_ok() =>
            {
                match check_privacy_card(l, p.manifest(), mref.model()) {
                    Ok(()) => chain.push((p, mref.model().to_owned())),
                    Err(e) => notes.push(e),
                }
            }
            Ok(_) => {}
            Err(e) => notes.push(e),
        }
    }
    kor_providers::with_fallbacks(primary, chain)
}

// ---------------------------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------------------------

/// Point tools at the project's memory directory and put the index in the prompt.
pub fn attach_memory(tctx: &mut ToolCtx, system: &mut String) {
    if !memory::auto_memory_enabled(None) {
        return;
    }
    let Some(lay) = memory_layout(&tctx.root) else {
        return;
    };
    if std::fs::create_dir_all(&lay.auto_dir).is_ok()
        && let Ok(canon) = lay.auto_dir.canonicalize()
    {
        tctx.extra_roots.push(canon.clone());
        tctx.team_memory = Some(canon.join("team"));
    }
    let raw = memory::read_entrypoint(&lay).unwrap_or_default();
    system.push_str(&memory::prompt_section(&raw, &lay.auto_dir));
}

pub fn session_notes_for(root: &Path, session: &Path) -> Option<PathBuf> {
    let base = kor_home()?;
    let id = session.file_stem()?.to_string_lossy();
    let lay = memory::layout(&base, root);
    Some(memory::session_summary_path(&lay, &id))
}

/// Re-read `MEMORY.md` so a memory saved on the previous turn is in the next prompt.
pub fn refresh_memory(agent: &mut Agent) {
    if !memory::auto_memory_enabled(None) {
        return;
    }
    let Some(lay) = memory_layout(&agent.ctx.root) else {
        return;
    };
    let raw = memory::read_entrypoint(&lay).unwrap_or_default();
    let section = memory::prompt_section(&raw, &lay.auto_dir);
    if let Some(i) = agent.system.find("\n# Auto memory\n") {
        agent.system.truncate(i);
    }
    agent.system.push_str(&section);
}

// ---------------------------------------------------------------------------------------------
// Session assembly
// ---------------------------------------------------------------------------------------------

/// A cloned repository must not grant itself permissions. Unless the project is trusted
/// (`kor trust`), project-level `.kor/config.toml` may only restrict, never widen.
/// Returns the note to show when something was ignored.
pub fn untrusted_project_permissions(
    l: &Localizer,
    loaded: &kor_config::Loaded,
    cfg: &mut kor_config::Config,
) -> Option<String> {
    let from_project = |key: &str| {
        loaded
            .settings
            .get(key)
            .is_some_and(|s| s.layer == kor_config::Layer::Project)
    };
    let mut ignored = Vec::new();
    if from_project("permissions.allow") && !cfg.permissions.allow.is_empty() {
        cfg.permissions.allow.clear();
        ignored.push("permissions.allow");
    }
    if from_project("permissions.mode") && cfg.permissions.mode.widens() {
        cfg.permissions.mode = PermissionMode::Default;
        ignored.push("permissions.mode");
    }
    (!ignored.is_empty()).then(|| {
        l.t_args(
            "project-permissions-ignored",
            &args!("keys" => ignored.join(", ")),
        )
    })
}

/// Messages and transcript path for a new or resumed session.
///
/// `new_id` names a new transcript (the desktop app passes the conversation id it generated);
/// otherwise a sortable time-based id is used.
pub fn load_session(
    l: &Localizer,
    resume: Option<&Resume>,
    new_id: Option<&str>,
) -> Result<(Vec<Message>, Option<PathBuf>), String> {
    let Some(resume) = resume else {
        let path = match new_id {
            Some(id) if valid_session_id(id) => {
                sessions_dir().map(|d| d.join(format!("{id}.jsonl")))
            }
            Some(_) => return Err(l.t("session-none")),
            None => new_session_path(),
        };
        return Ok((Vec::new(), path));
    };
    let dir = sessions_dir().unwrap_or_default();
    let path = match resume {
        Resume::Id(id) if valid_session_id(id) => Some(dir.join(format!("{id}.jsonl"))),
        Resume::Id(_) => None,
        Resume::Latest => session::list(&dir).into_iter().next(),
    };
    let Some(path) = path else {
        return Err(l.t("session-none"));
    };
    match session::load(&path) {
        Ok(m) => Ok((m, Some(path))),
        Err(e) => Err(l.t_args(
            "session-load-failed",
            &args!("id" => path.display().to_string(), "error" => e.to_string()),
        )),
    }
}

/// What to assemble.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Project root (canonical). Tools are confined to it.
    pub root: PathBuf,
    /// `provider/model`; falls back to `agent.model` in config.
    pub model: Option<String>,
    /// Explicit mode (`--mode` or a desktop choice); falls back to config and compat settings.
    pub mode: Option<PermissionMode>,
    pub resume: Option<Resume>,
    /// Id for a new transcript; see [`load_session`].
    pub new_id: Option<String>,
    /// Where replies are shown: the desktop renders visual blocks, a terminal does not.
    pub surface: prompt::Surface,
}

/// An assembled session, ready for [`Agent::send`].
pub struct Prepared {
    pub agent: Agent,
    /// Transcript file; `None` when no data directory exists.
    pub path: Option<PathBuf>,
    pub compat: Compat,
    pub model: ModelRef,
    pub mode: PermissionMode,
    pub sandbox: SandboxKind,
    /// Cost router, ledger and budget of this session (also held by the agent).
    pub cost: Arc<cost::Control>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("agent", &self.agent)
            .field("path", &self.path)
            .field("model", &self.model)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Assemble an agent: provider and key, permissions (with trust and sandbox fallbacks), tools,
/// system prompt with memory, hooks, rules, and the transcript.
///
/// Non-fatal messages go to `notes` in the order the CLI used to print them; a fatal problem is
/// returned as the localized message (after any notes collected so far).
#[allow(clippy::too_many_lines)] // linear assembly of one struct; splitting hides the order
pub fn prepare(
    l: &Localizer,
    loaded: &kor_config::Loaded,
    opts: &Options,
    notes: &mut Vec<String>,
) -> Result<Prepared, String> {
    // One-shot model calls made during the session (summaries, commit messages) go to the ledger.
    kor_agent::cost::set_usage_sink(usage::record_oneshot);
    // `agent.status` verbs follow the user's language (§16.2).
    kor_agent::status::set_lang(l.lang().code());
    // §19.3: every real surface stops writes on a critically full disk.
    kor_health::enforce_write_guard();
    let root = opts.root.clone();
    let compat = load_compat(&root);
    let mut config = loaded.config.clone();
    if !compat.trusted
        && let Some(n) = untrusted_project_permissions(l, loaded, &mut config)
    {
        notes.push(n);
    }
    let model = opts
        .model
        .clone()
        .or_else(|| offline::default_model(&config))
        .or_else(|| (!config.agent.model.trim().is_empty()).then(|| config.agent.model.clone()));
    let Some(model) = model else {
        return Err(l.t("no-model"));
    };
    // Before any client is built: clients take the policy's proxy rule at build time.
    apply_network_policy(l, &config);
    let privacy = privacy_mode();
    let (provider, model, key) = build_provider(l, &model, notes)?;
    check_provider_allowed(l, provider.manifest())?;
    check_privacy_card(l, provider.manifest(), model.model())?;
    let provider = with_routing_fallback(l, provider, &config.routing.fallback, privacy, notes);
    // §32.2 (ADR 0062): open the connection now (inside a runtime; the CLI does it when its
    // runtime starts), so the first token does not wait for DNS/TCP/TLS.
    provider.preconnect();
    let provider_tool_calling = provider.manifest().tool_calling;

    let mut compat_warnings = compat.warnings.clone();
    // §6.4 `[sandbox]`: mode, backend, network allowlist, limits (ADR 0033).
    let sandbox_mode = config.sandbox.mode;
    let sbx = sandbox::setup(l, &config.sandbox, offline_mode());
    let sandbox = sbx.kind;
    if sandbox.is_enforcing() {
        notes.extend(sbx.notes.iter().cloned());
    }
    // Explicit mode, then an explicit Kor setting, then Claude/Codex settings, then the default.
    let kor_mode_set = loaded
        .settings
        .get("permissions.mode")
        .is_some_and(|s| s.layer != kor_config::Layer::Default);
    let explicit = opts
        .mode
        .or(kor_mode_set.then_some(config.permissions.mode));
    let mut mode = explicit
        .or(compat.permissions.mode.as_ref().map(|(m, _)| *m))
        .unwrap_or(config.permissions.mode);
    // Another tool's "bypass"/"never ask" setting must not make Kor unusable without a
    // sandbox; fall back to asking. An explicit Kor choice still fails loudly below.
    if mode == PermissionMode::Auto && explicit.is_none() && !sandbox.is_enforcing() {
        notes.push(l.t("compat-auto-downgraded"));
        mode = PermissionMode::Default;
    }
    if mode == PermissionMode::Auto && !sandbox.is_enforcing() {
        return Err(l.t("mode-auto-needs-sandbox"));
    }
    // Never silently unsandboxed: with `ask`, or `on` but no backend, commands keep asking.
    if sandbox_mode != kor_config::SandboxMode::Off && !sandbox.is_enforcing() {
        if sandbox_mode == kor_config::SandboxMode::On {
            notes.push(l.t("sandbox-missing-ask"));
        }
        if mode == PermissionMode::BypassPermissions {
            mode = PermissionMode::AcceptEdits;
        }
    }
    let join =
        |kor: &[String], extra: Vec<String>| kor.iter().cloned().chain(extra).collect::<Vec<_>>();
    let allow = join(
        &config.permissions.allow,
        rules(&compat.permissions.allow, &mut compat_warnings),
    );
    let mut ask = join(
        &config.permissions.ask,
        rules(&compat.permissions.ask, &mut compat_warnings),
    );
    // §15.7: the project's environment (devcontainer, compose, nix, mise); a container one cannot
    // be confined by the OS sandbox, so its commands ask (ADR 0043).
    let (dev_env, env_notes) = devenv::session(l, &config.env, &root, sandbox);
    notes.extend(env_notes);
    if dev_env.as_ref().is_some_and(|e| devenv::asks(e, sandbox)) {
        ask.push("Bash".into());
    }
    let mut deny = join(
        &config.permissions.deny,
        rules(&compat.permissions.deny, &mut compat_warnings),
    );
    // §26.2 `[tools] deny`: the shell refuses those programs as well.
    deny.extend(clitools::deny_rules(&config.tools));
    // §28.2 `[plan] lock`: the session starts in plan mode; only an approved plan opens it.
    if config.plan.lock {
        mode = PermissionMode::Plan;
    }
    let permissions = Permissions::new(mode, &allow, &ask, &deny)
        .map_err(|e| l.t_args("error-generic", &args!("error" => e.to_string())))?;

    let mut tctx = ToolCtx::new(&root).map_err(|e| format!("project root: {e}"))?;
    tctx.sandbox = sandbox;
    tctx.network = sbx.network;
    tctx.sandbox_options = sbx.options;
    // Keys of every known provider, plus anything shaped like one, stay out of commands.
    let mut ignored_notes = Vec::new();
    tctx.scrub_env = registry(l, &mut ignored_notes)
        .map(|r| r.iter().map(ProviderManifest::key_env).collect())
        .unwrap_or_default();
    tctx.scrub_env.extend(
        std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| k.ends_with("_API_KEY")),
    );
    tctx.env = compat
        .env
        .iter()
        .map(|(k, v, _)| (k.clone(), v.clone()))
        .collect();
    if offline_mode() {
        // A sandboxed command gets no network at all; the proxy variables cover the rest.
        tctx.network = false;
        tctx.env.retain(|(k, _)| {
            !OFFLINE_COMMAND_ENV
                .iter()
                .any(|(o, _)| k.eq_ignore_ascii_case(o))
        });
        tctx.env.extend(
            OFFLINE_COMMAND_ENV
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        notes.push(l.t("net-offline-command"));
    }
    compat_warnings.extend(tctx.load_ai_ignore());

    let mut redactor = Redactor::default();
    if let Some(k) = key {
        redactor.register(k);
    }
    // §6.6: vault secrets reach agent commands as environment variables, never files or prompts;
    // `[vault.scope]` limits a secret to some programs (ADR 0036).
    let vault = match secure::vault_store(&config.vault, None) {
        Ok(store) => kor_secrets::vault::entries(store.as_ref()).unwrap_or_default(),
        Err(e) => {
            if config.vault.backend == kor_config::VaultBackend::File {
                notes.push(l.t_args("vault-locked-note", &args!("error" => e.to_string())));
            }
            Vec::new()
        }
    };
    for (name, value) in vault {
        tctx.env.retain(|(k, _)| *k != name);
        let v = kor_secrets::ExposeSecret::expose_secret(&value).to_owned();
        match config.vault.scope.get(&name) {
            Some(programs) => tctx.scoped_env.push((name, v, programs.clone())),
            None => tctx.env.push((name, v)),
        }
        redactor.register(value);
    }
    // §6.7: every agent action goes to the hash-chained audit log (unit tests keep the real one clean).
    if !cfg!(test) {
        kor_agent::audit::set_path(kor_home().map(|h| h.join(kor_agent::audit::FILE)));
    }

    let reply_language = config.agent.reply_language.fixed().map(|l| match l {
        kor_i18n::Lang::Tr => "Turkish",
        kor_i18n::Lang::Fr => "French",
        kor_i18n::Lang::En => "English",
    });
    let shell = if sandbox == SandboxKind::Docker {
        Shell::container()
    } else {
        Shell::detect()
    };
    let mut system = prompt::build(
        &tctx.root,
        shell.name,
        reply_language,
        &compat,
        opts.surface,
    );
    // §28.13 `agent.ask_threshold`: when the agent asks instead of proceeding.
    prompt::set_ask_level(
        &mut system,
        match config.agent.ask_threshold {
            kor_config::AskThreshold::Never => prompt::AskLevel::Never,
            kor_config::AskThreshold::Critical => prompt::AskLevel::Critical,
            kor_config::AskThreshold::Often => prompt::AskLevel::Often,
        },
    );
    if config.auto.memory_inject {
        attach_memory(&mut tctx, &mut system);
    }
    // `kor style` profile (ADR 0047): the agent writes in the developer's measured style.
    if let Some(home) = kor_home() {
        system.push_str(&analysis::metrics::style::prompt_section(&home, &tctx.root));
        // §19.2 (ADR 0059): this machine's profile from `system.json`, budgeted.
        system.push_str(&kor_health::prompt_section(
            &home,
            config.health.refresh_minutes,
            1200,
        ));
    }
    // §5.3: real window of the model, and a signature map instead of whole files.
    let context_window = context_window_for(&model, provider.manifest());
    let rules_tokens = kor_agent::context::tokens(
        compat
            .instructions
            .iter()
            .filter(|i| i.applies == kor_compat::Applies::Always)
            .map(|i| i.text.len())
            .sum(),
    );
    // Faz 3: files ranked by the stored project graph (no build here), else shallow-first.
    let rank = config
        .graph
        .enabled
        .then(|| kor_agent::graph::ranked_files(&tctx.root))
        .flatten();
    let repo_map = kor_agent::repomap::build_ranked(
        &tctx.root,
        tctx.ai_ignore.as_deref(),
        u64::from(config.context.repo_map_tokens).min(context_window / 16),
        rank.as_deref(),
    );
    system.push_str(&repo_map);
    // §27.5: files dropped into `~/Kor/Gelen/` join the next new interactive session (CLI REPL or
    // desktop; not triggers, bench or file tasks, which pass their own id) as context (ADR 0052).
    if opts.resume.is_none() && (opts.new_id.is_none() || opts.surface == prompt::Surface::Desktop)
    {
        system.push_str(&korfolder::take_inbox_context());
    }

    let resume = opts.resume.as_ref();
    let (messages, path) = load_session(l, resume, opts.new_id.as_deref())?;
    tctx.dev_env = dev_env.map(Arc::new);
    // §28.3: every proposed edit is checked against the loaded rules before it is shown.
    if config.agent.rules_check {
        tctx.rule_checks = Some(compliance::rule_set(&compat));
    }
    // §15.9: `[record] enabled` (or `KOR_RECORD__ENABLED=true` for one session).
    if config.record.enabled {
        let id = path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(session::new_id, |s| s.to_string_lossy().into_owned());
        match kor_tools::record::Recorder::create(&tctx.root, &id) {
            Ok(r) => {
                notes.push(l.t_args("record-on", &args!("id" => id.clone())));
                tctx.record = Some(Arc::new(r));
            }
            Err(e) => notes.push(l.t_args("record-failed", &args!("error" => e.to_string()))),
        }
    }
    // `kor undo`: per-session journal of the first state of every changed file.
    if let (Some(base), Some(id)) = (
        undo_dir(),
        path.as_deref()
            .and_then(Path::file_stem)
            .and_then(|s| s.to_str()),
    ) {
        match kor_tools::undo::Journal::open(&base, id, std::slice::from_ref(&tctx.root)) {
            Ok(j) => {
                *tctx
                    .undo
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(j);
            }
            Err(e) => compat_warnings.push(format!("undo journal: {e}")),
        }
    }
    // §32.3 (ADR 0062): plugin hooks run under a time budget.
    kor_agent::hooks::set_plugin_budget(
        loaded.config.perf.plugin_budget_ms,
        loaded.config.perf.log,
    );
    budgets::apply_low_power(&loaded.config);
    let hooks = (!compat.hooks.is_empty()).then(|| HookRunner {
        hooks: compat.hooks.clone(),
        shell: shell.clone(),
        root: tctx.root.clone(),
        session_id: path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or_else(session::new_id, |s| s.to_string_lossy().into_owned()),
        transcript: path.clone(),
        model: Some(model.to_string()),
    });
    let (rules, rule_errors) = ScopedRule::from_compat(&tctx.root, &compat);
    compat_warnings.extend(rule_errors);
    let flags = kor_agent::flags::Flags::load(
        kor_config::user_dir()
            .map(|d| d.join("flags.json"))
            .as_deref(),
    );
    tctx.background_tasks = flags.background_tasks;
    tctx.fetch.summarizer = Some(Arc::new(kor_agent::summarize::ProviderSummarizer {
        provider: provider.clone(),
        model: model.model().to_owned(),
    }));
    kor_tools::set_computer_enabled(config.computer.enabled);
    computer_and_health_setup(&config, l.lang(), opts.surface);
    let mut tools = default_tools();
    if !compat.skills.is_empty() {
        tools.push(Arc::new(SkillTool {
            skills: compat.skills.clone(),
        }));
    }
    let (ant_tools, ant_prompt) =
        kor_agent::modes::extras(kor_agent::modes::ModeFlags::from_env(), &tools);
    if !ant_prompt.is_empty() {
        system.push('\n');
        system.push_str(&ant_prompt);
    }
    tools.extend(ant_tools);
    // Semantic file search with the local embedding model when installed (ADR 0023): nothing
    // leaves the machine, so it also runs in privacy mode and is preferred over remote ones.
    if let Some(local) = kor_agent::semantic::local_embedder()
        && let Some(home) = kor_home()
    {
        tools.push(Arc::new(kor_agent::semantic::FileSearch::new(
            local,
            &home.join("index"),
            &tctx.root,
        )));
    }
    // Image and video generation when a Venice key is configured, whatever the chat model is.
    let mut ignored = Vec::new();
    if !privacy
        && let Ok(reg) = registry(l, &mut ignored)
        && let Some(venice) = reg.get("venice")
        && let Ok(Some((key, _))) = key_for(venice)
        && let Ok(client) = kor_providers::media::VeniceMedia::new(venice, key.clone())
    {
        // Semantic file search shares the key; its index lives in Kor's home, not the project.
        if let Some(home) = kor_home()
            && !tools.iter().any(|t| t.name() == "file_search")
            && let Ok(embed) = kor_providers::media::VeniceMedia::new(venice, key.clone())
        {
            tools.push(Arc::new(kor_agent::semantic::FileSearch::new(
                embed,
                &home.join("index"),
                &tctx.root,
            )));
        }
        redactor.register(key);
        tools.extend(kor_agent::media::tools(&kor_agent::media::Media::with_env(
            client,
            media::env(Some(&config.media)),
        )));
    }
    // fal.ai / Hugging Face media tools; HF embeddings back file_search when Venice does not.
    if !privacy {
        let fal = fal_client(l).map(|(f, k)| {
            redactor.register(k);
            f
        });
        let hf = hf_client(l, None).ok().and_then(|(h, k)| {
            k.map(|k| {
                redactor.register(k);
                h
            })
        });
        if let Some(h) = &hf
            && !tools.iter().any(|t| t.name() == "file_search")
            && let Some(home) = kor_home()
        {
            tools.push(Arc::new(kor_agent::semantic::FileSearch::new(
                h.clone(),
                &home.join("index"),
                &tctx.root,
            )));
        }
        if let Some(s) =
            kor_agent::media::Studio::with_env(fal, hf, media::env(Some(&config.media)))
        {
            tools.extend(kor_agent::media::studio_tools(&s));
        }
        // §20.3: generated media and their derivation chain, once whichever provider made them.
        if tools
            .iter()
            .any(|t| t.name() == "generate_image" || t.name() == "image_generate")
        {
            tools.push(kor_agent::media::gallery_tool());
        }
    }
    // §28.8: configured ACP agents as a sub-agent tool (ADR 0057).
    if !config.acp.agents.is_empty() {
        tools.push(Arc::new(acp::client::AcpAgentTool {
            agents: config.acp.agents.clone(),
        }));
    }
    let memory = memory::auto_memory_enabled(None)
        .then(|| memory_layout(&tctx.root))
        .flatten();
    // Everyday assistant: calendar, reminders, weather, cards. Most load on demand (ToolSearch).
    if let Some(home) = kor_home() {
        let desktop = matches!(opts.surface, prompt::Surface::Desktop);
        let (assistant, deferred) =
            kor_agent::assistant::tools(home.join("assistant.json"), desktop);
        system.push_str(&kor_agent::assistant::deferred_section(
            &assistant, &deferred,
        ));
        if let Ok(mut d) = tctx.deferred.lock() {
            d.extend(deferred.iter().map(|n| (*n).to_owned()));
        }
        if let Ok(mut about) = tctx.deferred_about.lock() {
            for t in assistant.iter().filter(|t| deferred.contains(&t.name())) {
                about.insert(t.name().to_owned(), t.spec().description);
            }
        }
        tools.extend(assistant);
    }
    // The user's own browser through the Kor extension (§18.2, ADR 0058): `[browser] backend`
    // picks what `browser_*` drives; the bridge runs unless the backend is headless (or with the
    // legacy KOR_COMPUTER_CONTROL=1). Loaded on demand.
    kor_tools::chrome::set_backend(match config.browser.backend {
        kor_config::BrowserBackend::Auto => kor_tools::chrome::Backend::Auto,
        kor_config::BrowserBackend::Chrome => kor_tools::chrome::Backend::Extension,
        kor_config::BrowserBackend::Headless => kor_tools::chrome::Backend::Headless,
    });
    if !privacy
        && (config.browser.backend != kor_config::BrowserBackend::Headless
            || kor_tools::chrome::enabled())
    {
        kor_tools::chrome::spawn_start();
        let chrome = kor_tools::chrome::tools();
        if let Ok(mut d) = tctx.deferred.lock() {
            d.extend(chrome.iter().map(|t| t.name().to_owned()));
        }
        if let Ok(mut about) = tctx.deferred_about.lock() {
            for t in &chrome {
                about.insert(t.name().to_owned(), t.spec().description);
            }
        }
        tools.extend(chrome);
    }
    // SSH servers and databases the user defined in their own connections.toml (never the project's).
    tools.extend(kor_agent::remote::tools(
        kor_config::user_dir().map(|d| d.join("connections.toml")),
    ));
    if let Some(extra) = EXTRA_TOOLS.get() {
        tools.extend(extra());
    }
    tools.extend(kor_agent::knowledge::tools(
        memory,
        sessions_dir(),
        path.clone(),
    ));
    // §26.2: typed tool definitions, `tools_search`, and the PATH catalog in the prompt.
    compat_warnings.extend(clitools::attach(
        &mut tools,
        &mut system,
        &tctx,
        &config.tools,
        compat.trusted,
        privacy,
        context_window,
    ));
    if privacy {
        tools.retain(|t| !needs_network(t.name()));
    }
    // §7.5: graph tools; the graph builds in the background and stays fresh while the agent lives.
    if config.graph.enabled {
        kor_agent::graph::attach(
            &mut tools,
            &mut system,
            &tctx.root,
            kor_agent::graph::Options {
                strict: config.graph.strict,
                watch: config.graph.watch,
                skip_hotspots: !config.auto.hotspots,
                home: kor_home(),
            },
        );
    }
    let cost = cost_control(l, &config, &provider, model.model(), privacy, notes);
    let child_tools = tools.clone();
    tools.push(Arc::new(kor_agent::AgentTool {
        provider: provider.clone(),
        model: model.model().to_owned(),
        tools: child_tools,
        permissions: permissions.clone(),
        agents: kor_agent::builtin::with_builtins(compat.agents.clone()),
        explore_plan: true,
        cost: Some(cost.clone()),
    }));

    if !compat_warnings.is_empty() {
        notes.push(l.t_args(
            "compat-warnings-note",
            &args!("count" => compat_warnings.len()),
        ));
    }
    if !compat.untrusted_skipped.is_empty() {
        notes.push(l.t_args(
            "compat-untrusted-note",
            &args!("count" => compat.untrusted_skipped.len()),
        ));
    }

    // §28.2 plan mode state of this session (lock, intent phrases, approved plan).
    let gate = kor_tools::plans::Gate {
        lock: config.plan.lock,
        intent: config.auto.plan_intent,
        session: path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned()),
        ..kor_tools::plans::Gate::default()
    };
    if config.plan.lock {
        gate.enter();
        notes.push(l.t("plan-lock-note"));
    }
    tctx.plan_gate = Arc::new(gate);
    let session_notes = path.as_ref().and_then(|p| session_notes_for(&tctx.root, p));
    let summarizer = summarizer(
        l,
        &config.context.summarize_model,
        privacy,
        &mut redactor,
        notes,
    );
    // §31.3: knowledge base recall/learning; a missing or locked database just leaves it off.
    let recall = kor_agent::recall::Recall::open(
        kor_home().as_deref(),
        &tctx.root,
        config.memory.inject,
        u64::from(config.memory.max_tokens),
        config.memory.learn,
    );
    // §6.6: every key and vault value the session knows is masked in model requests as well.
    redactor.share_outbound();
    let agent = Agent {
        provider,
        model: model.model().to_owned(),
        tools,
        permissions,
        ctx: tctx,
        system,
        messages,
        usage: Usage::default(),
        max_turns: config.agent.max_turns.max(1),
        max_tokens: (config.agent.max_tokens > 0).then_some(config.agent.max_tokens),
        reasoning: config.agent.reasoning.map(kor_core::Reasoning::effort),
        redactor,
        hooks,
        rules,
        auto_compact: flags.auto_compact && config.auto.compact,
        context_window,
        memory_extract: flags.memory_extract && config.auto.memory_record,
        memory_base: kor_home(),
        session_notes,
        output_budget: None,
        dream: false,
        tool_mode: match config.agent.tool_mode {
            kor_config::ToolCalling::Native => kor_agent::toolcall::ToolMode::Native,
            kor_config::ToolCalling::Text => kor_agent::toolcall::ToolMode::Text,
            kor_config::ToolCalling::Auto => {
                kor_agent::toolcall::ToolMode::for_provider(provider_tool_calling)
            }
        },
        verify: kor_agent::verify::Verify::new(
            config.agent.verify == kor_config::VerifyMode::Auto,
            config.agent.write_max_lines,
        ),
        stats: kor_agent::repair::ToolStats::default(),
        context: kor_agent::context::Budget {
            warn_percent: config.context.warn_percent,
            auto_percent: config.context.auto_summarize_percent,
            summarizer,
            rules_tokens,
            repo_map_tokens: kor_agent::context::tokens(repo_map.len()),
            memory_tokens: 0,
            recall,
            log: config.context.log.then(context_log_path).flatten(),
            last: None,
            warned: false,
            compact_confirm: config.context.compact_confirm,
            compact_snooze: 0,
            pinned: Vec::new(),
        },
        cost: Some(cost.clone()),
        shadow: shadow_runner(l, &config, notes),
        repair: repair_model(l, &config, notes),
    };
    Ok(Prepared {
        agent,
        path,
        compat,
        model,
        mode,
        sandbox,
        cost,
    })
}

// ---------------------------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------------------------

/// Result of [`attach_mcp`].
#[derive(Debug, Default)]
pub struct McpAttached {
    /// Problems to show (failed or skipped servers).
    pub notes: Vec<String>,
    /// One line naming the connected servers and tool count; `None` when nothing connected.
    pub connected: Option<String>,
}

/// Above this many MCP tools, their schemas are deferred until `ToolSearch` or a direct call.
const MCP_EAGER_MAX: usize = 15;

/// System-prompt section naming the deferred MCP tools, grouped by server.
fn mcp_deferred_section(tools: &[Arc<dyn kor_tools::Tool>]) -> String {
    use std::fmt::Write as _;
    let mut by_server = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for t in tools {
        let name = t.name();
        let (server, tool) = name
            .strip_prefix("mcp__")
            .and_then(|r| r.split_once("__"))
            .unwrap_or(("other", name));
        by_server.entry(server).or_default().push(tool);
    }
    let mut s = String::from(
        "\n# MCP tools (load on demand)\nThese MCP server tools are available but their schemas are \
         not loaded. Call ToolSearch with a server name or keyword to load the ones you need, or \
         call one directly by its full name (`mcp__<server>__<tool>`).\n",
    );
    for (server, names) in by_server {
        let _ = writeln!(s, "- {server}: {}", names.join(", "));
    }
    s
}

/// Start the configured MCP servers (see [`kor_agent::mcp_tools`]) and add their tools to the
/// agent and to its subagents. Servers that fail are reported in `notes`; the session still runs.
pub async fn attach_mcp(l: &Localizer, agent: &mut Agent, compat: &Compat) -> McpAttached {
    use kor_agent::mcp_tools::{Skip, Sources, connect_all, discover};

    let home = dirs::home_dir();
    let kor_dir = kor_config::user_dir();
    let root = agent.ctx.root.clone();
    let sources = Sources {
        project_root: &root,
        project_trusted: compat.trusted,
        home: home.as_deref(),
        kor_config_dir: kor_dir.as_deref(),
    };
    let mut out = McpAttached::default();
    // MCP servers are arbitrary programs that may reach the network: off in privacy mode.
    if privacy_mode() {
        out.notes.push(l.t("privacy-mcp-off"));
        return out;
    }
    let (specs, skipped) = discover(&sources, &|k| std::env::var(k).ok());
    for s in skipped {
        out.notes.push(match s {
            Skip::Untrusted { count } => l.t_args("mcp-untrusted", &args!("count" => count)),
            Skip::Remote { name, .. } => l.t_args("mcp-remote", &args!("name" => name)),
            Skip::Invalid {
                name,
                origin,
                reason,
            } => l.t_args(
                "mcp-invalid",
                &args!("name" => name, "origin" => origin, "reason" => reason),
            ),
        });
    }
    if specs.is_empty() {
        return out;
    }
    let (mut tools, report) = connect_all(&specs, &root).await;
    // §28.15: the agent may restart a server it is developing (`mcp_restart`, asks first).
    let clients: Vec<_> = report.iter().filter_map(|c| c.client.clone()).collect();
    let restart: Option<Arc<dyn kor_tools::Tool>> = (!clients.is_empty()).then(|| {
        Arc::new(kor_agent::mcp_tools::McpRestart::new(clients)) as Arc<dyn kor_tools::Tool>
    });
    let mut names = Vec::new();
    for c in report {
        match c.result {
            Ok(_) => names.push(c.server),
            Err(e) => out
                .notes
                .push(l.t_args("mcp-failed", &args!("name" => c.server, "error" => e))),
        }
    }
    if names.is_empty() {
        return out;
    }
    out.connected = Some(l.t_args(
        "mcp-connected",
        &args!("servers" => names.len(), "tools" => tools.len(), "names" => names.join(", ")),
    ));
    // Every tool schema is sent on every request; a large MCP set (hundreds of tools) would cost
    // far more than the conversation itself, so it loads on demand like the deferred built-ins.
    if tools.len() > MCP_EAGER_MAX {
        agent.system.push_str(&mcp_deferred_section(&tools));
        if let Ok(mut d) = agent.ctx.deferred.lock() {
            d.extend(tools.iter().map(|t| t.name().to_owned()));
        }
        if let Ok(mut about) = agent.ctx.deferred_about.lock() {
            for t in &tools {
                about.insert(t.name().to_owned(), t.spec().description);
            }
        }
    }
    tools.extend(restart);
    agent.tools.extend(tools);
    // The subagent tool was built with the tool list of the time; rebuild it so Explore/Plan and
    // general-purpose agents can use the server tools too.
    if let Some(pos) = agent.tools.iter().position(|t| t.name() == "Agent") {
        let child_tools = agent
            .tools
            .iter()
            .filter(|t| t.name() != "Agent")
            .cloned()
            .collect();
        agent.tools[pos] = Arc::new(kor_agent::AgentTool {
            provider: agent.provider.clone(),
            model: agent.model.clone(),
            tools: child_tools,
            permissions: agent.permissions.clone(),
            agents: kor_agent::builtin::with_builtins(compat.agents.clone()),
            explore_plan: true,
            cost: agent.cost.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_providers_and_network_tools_are_recognized() {
        assert_eq!(
            reqwest_host("http://localhost:11434/v1").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            reqwest_host("https://u:p@API.x.dev:8/a").as_deref(),
            Some("api.x.dev")
        );
        assert_eq!(reqwest_host("http://[::1]:8080").as_deref(), Some("[::1]"));
        let reg = kor_providers::Registry::builtin();
        assert!(is_local_provider(reg.get("local").unwrap()));
        assert!(!is_local_provider(reg.get("venice").unwrap()));
        assert!(
            needs_network("WebFetch") && needs_network("mcp__rea__x") && needs_network("ssh_exec")
        );
        assert!(!needs_network("Read") && !needs_network("Bash"));
    }

    #[test]
    fn mcp_section_groups_by_server() {
        let tools =
            kor_agent::assistant::tools(std::env::temp_dir().join("kor-none.json"), false).0;
        let s = mcp_deferred_section(&tools[..2]);
        assert!(s.contains("# MCP tools (load on demand)"));
        assert!(s.contains("- other: "));
    }

    #[test]
    fn session_ids_cannot_escape() {
        assert!(valid_session_id("20260101-000000-12"));
        assert!(valid_session_id("3f2a9c1e-7b4d-4e8a-9f00-1a2b3c4d5e6f"));
        assert!(!valid_session_id(""));
        assert!(!valid_session_id("../x"));
        assert!(!valid_session_id("a/b"));
        assert!(!valid_session_id("a.b"));
    }

    #[test]
    fn new_session_uses_given_id() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let (msgs, path) = load_session(&l, None, Some("abc-123")).unwrap();
        assert!(msgs.is_empty());
        if let Some(p) = path {
            assert_eq!(p.file_name().unwrap(), "abc-123.jsonl");
        }
        assert!(load_session(&l, None, Some("../evil")).is_err());
    }
}
