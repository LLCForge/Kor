//! Offline and air-gap preparation (Master Build Prompt §6.5, ADR 0034): `kor offline
//! init|status|run|bundle|import` and the desktop "Çevrimdışı" panel share this module.
//!
//! - Runtime: a running Ollama / LM Studio / llama.cpp server found on its loopback port, or the
//!   pinned `llama-server` component (official llama.cpp build, verified download).
//! - Models: pinned GGUF components (`model-*`) served by `llama-server`, or `ollama pull`.
//! - The local embedding model (ADR 0023) and a durable copy of the model catalog.
//! - `[offline]` config: `runtime`, `endpoint`, `model`; `enabled` is the network switch of
//!   ADR 0035 (`kor --offline`).
//! - Bundles: [`kor_components::bundle`] archives with the CLI, runtime, models and catalog.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use kor_components::bundle::{self, Kind, Source};
use kor_components::{Progress, Store};
use kor_providers::model_catalog as mc;
use serde::Serialize;

/// Provider ids of the local runtimes Kor knows.
pub const LLAMACPP: &str = "local-llamacpp";
pub const OLLAMA: &str = "local";
pub const LMSTUDIO: &str = "local-lmstudio";
/// The `llama.cpp` server component and its port (the `local-llamacpp` manifest default).
pub const RUNTIME_COMPONENT: &str = "llama-server";
pub const LLAMA_ENDPOINT: &str = "http://127.0.0.1:9931/v1";
pub const LLAMA_PORT: u16 = 9931;
/// Local embedding model component (ADR 0023) and its tokenizer.
pub const EMBEDDING: &str = "embed-e5-small";
const EMBEDDING_TOKENIZER: &str = "embed-e5-small-tokenizer";
/// Suggested Ollama tags (`ollama pull`), sizes come from the Ollama registry.
pub const OLLAMA_MODELS: &[&str] = &["qwen3:4b", "qwen3:8b", "qwen2.5-coder:7b"];
const MODEL_PREFIX: &str = "model-";
const PROBE: Duration = Duration::from_millis(1500);

/// `<kor data>/offline`: bundle cache, imports, catalog snapshot, server log.
pub fn dir() -> Option<PathBuf> {
    kor_platform::data_dir().map(|d| d.join("offline"))
}

/// The model catalog cache shared with `kor catalog` (disposable cache folder).
pub fn catalog_cache() -> Option<PathBuf> {
    kor_platform::cache_dir().map(|d| d.join("model-catalog.json"))
}

/// The durable copy in Kor's data folder (OS cache cleaners do not remove it).
pub fn catalog_snapshot() -> Option<PathBuf> {
    dir().map(|d| d.join("model-catalog.json"))
}

/// Put the snapshot back when the cache was cleaned. Returns whether a catalog is available.
pub fn restore_catalog() -> bool {
    let (Some(cache), Some(snap)) = (catalog_cache(), catalog_snapshot()) else {
        return false;
    };
    if cache.is_file() {
        return true;
    }
    snap.is_file()
        && cache
            .parent()
            .is_some_and(|p| std::fs::create_dir_all(p).is_ok())
        && std::fs::copy(&snap, &cache).is_ok()
}

fn catalog_age_secs() -> Option<u64> {
    let newest = [catalog_cache(), catalog_snapshot()]
        .into_iter()
        .flatten()
        .filter_map(|p| mc::read_cache(&p))
        .map(|c| c.fetched_at)
        .max()?;
    Some(kor_providers::catalog::now().saturating_sub(newest))
}

// ---------------------------------------------------------------------------------------------
// Models and runtimes
// ---------------------------------------------------------------------------------------------

/// A pinned GGUF model (`model-*` component).
#[derive(Debug, Clone, Serialize)]
pub struct ModelChoice {
    /// Component name (`model-qwen3-4b`).
    pub name: String,
    pub about: String,
    pub size: Option<u64>,
    pub installed: bool,
    /// Model reference for the agent (`local-llamacpp/qwen3-4b`).
    pub model_ref: String,
}

/// `model-qwen3-4b` → `qwen3-4b` (the name `llama-server` serves it under).
pub fn alias(component: &str) -> &str {
    component.strip_prefix(MODEL_PREFIX).unwrap_or(component)
}

/// `qwen3-4b` or `model-qwen3-4b` → `model-qwen3-4b`.
pub fn component_name(model: &str) -> String {
    if model.starts_with(MODEL_PREFIX) {
        model.to_owned()
    } else {
        format!("{MODEL_PREFIX}{model}")
    }
}

pub fn gguf_models() -> Vec<ModelChoice> {
    kor_components::list()
        .into_iter()
        .filter(|s| s.name.starts_with(MODEL_PREFIX))
        .map(|s| ModelChoice {
            model_ref: format!("{LLAMACPP}/{}", alias(&s.name)),
            installed: s.installed.is_some(),
            name: s.name,
            about: s.about,
            size: s.size,
        })
        .collect()
}

/// A local model server that answered on its port.
#[derive(Debug, Clone, Serialize)]
pub struct Server {
    pub provider: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
}

/// Probe the loopback ports of every local runtime (allowed in privacy/offline mode).
pub async fn servers(reg: &kor_providers::Registry) -> Vec<Server> {
    kor_providers::probe::discover(reg)
        .await
        .into_iter()
        .map(|f| Server {
            provider: f.provider,
            name: f.name,
            base_url: f.base_url,
            models: f.models,
        })
        .collect()
}

/// `llamacpp` / `ollama` / `lmstudio` or a provider id → provider id.
pub fn runtime_id(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "llamacpp" | "llama.cpp" | "llama-cpp" | "llama-server" => LLAMACPP.into(),
        "ollama" => OLLAMA.into(),
        "lmstudio" | "lm-studio" | "lm studio" => LMSTUDIO.into(),
        other => other.to_owned(),
    }
}

/// Default endpoint of a runtime from its manifest.
fn endpoint_of(reg: &kor_providers::Registry, runtime: &str) -> String {
    if runtime == LLAMACPP {
        return LLAMA_ENDPOINT.into();
    }
    reg.get(runtime)
        .map(|m| m.base_url.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // a status report: each flag is one fact
pub struct Status {
    pub enabled: bool,
    pub runtime: String,
    pub endpoint: String,
    pub model: String,
    /// Local servers answering right now.
    pub servers: Vec<Server>,
    /// The configured runtime answers (or `llama-server` can be started with the model).
    pub runtime_ready: bool,
    /// Installed `llama-server` executable.
    pub llama_server: Option<String>,
    pub models: Vec<ModelChoice>,
    pub embedding: bool,
    /// Seconds since the newest catalog copy was fetched; `None` when there is none.
    pub catalog_age_secs: Option<u64>,
    /// A full task can run offline now.
    pub ready: bool,
    /// What is missing: `model`, `runtime`, `model-file`.
    pub missing: Vec<&'static str>,
}

pub async fn status(cfg: &kor_config::Offline, reg: &kor_providers::Registry) -> Status {
    let servers = servers(reg).await;
    let models = gguf_models();
    let llama_server = kor_components::path(RUNTIME_COMPONENT);
    let (runtime_ready, missing) = readiness(cfg, &servers, &models, llama_server.is_some());
    Status {
        enabled: cfg.enabled,
        runtime: cfg.runtime.clone(),
        endpoint: cfg.endpoint.clone(),
        model: cfg.model.clone(),
        servers,
        runtime_ready,
        llama_server: llama_server.map(|p| p.display().to_string()),
        models,
        embedding: kor_components::path(EMBEDDING).is_some(),
        catalog_age_secs: catalog_age_secs(),
        ready: missing.is_empty(),
        missing,
    }
}

/// Whether the configured model can be served now, and what is missing.
fn readiness(
    cfg: &kor_config::Offline,
    servers: &[Server],
    models: &[ModelChoice],
    llama_installed: bool,
) -> (bool, Vec<&'static str>) {
    let Some((provider, model)) = cfg.model.split_once('/') else {
        let up = servers.iter().any(|s| s.provider == cfg.runtime);
        return (up, vec!["model"]);
    };
    let running = servers.iter().find(|s| s.provider == provider);
    if provider == LLAMACPP {
        let file = models
            .iter()
            .any(|m| m.installed && alias(&m.name) == model);
        // A running llama.cpp server serves whatever it was started with.
        if running.is_some() {
            return (true, Vec::new());
        }
        let mut missing = Vec::new();
        if !llama_installed {
            missing.push("runtime");
        }
        if !file {
            missing.push("model-file");
        }
        return (llama_installed && file, missing);
    }
    match running {
        None => (false, vec!["runtime"]),
        Some(s) if s.models.iter().any(|m| m == model) => (true, Vec::new()),
        Some(_) => (true, vec!["model-file"]),
    }
}

/// The model offline mode uses when none is given: `[offline] model` while offline mode is on.
pub fn default_model(config: &kor_config::Config) -> Option<String> {
    let m = config.offline.model.trim();
    ((config.offline.enabled || crate::offline_mode()) && !m.is_empty()).then(|| m.to_owned())
}

// ---------------------------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------------------------

/// One progress event (`kor:offline/progress` on the desktop).
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// `runtime`, `model`, `embedding`, `catalog`, `config`, `bundle`, `import`.
    pub step: &'static str,
    pub name: String,
    /// `start`, `progress`, `done`, `skip`, `error`.
    pub state: &'static str,
    pub done: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

impl Event {
    fn new(step: &'static str, name: &str, state: &'static str) -> Self {
        Self {
            step,
            name: name.to_owned(),
            state,
            done: 0,
            total: None,
            message: None,
        }
    }

    fn msg(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }
}

pub type Emit<'a> = &'a (dyn Fn(Event) + Send + Sync);

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    /// `llamacpp`, `ollama`, `lmstudio` or a provider id; `None` = configured, else a running
    /// server, else the `llama-server` component.
    pub runtime: Option<String>,
    /// GGUF component names (`qwen3-4b`) for llama.cpp, tags for Ollama, served ids otherwise.
    pub models: Vec<String>,
    /// Model for `[offline] model`; default: the first chosen one.
    pub default_model: Option<String>,
    pub embedding: bool,
    pub catalog: bool,
}

/// One download the user agrees to before [`init`] starts.
#[derive(Debug, Clone, Serialize)]
pub struct Download {
    /// `runtime`, `model`, `embedding`.
    pub step: &'static str,
    pub name: String,
    pub size: Option<u64>,
}

/// What [`init`] will do; shown to the user for consent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub runtime: String,
    pub endpoint: String,
    /// GGUF components or Ollama tags to fetch.
    pub models: Vec<String>,
    pub default_model: String,
    pub downloads: Vec<Download>,
    pub embedding: bool,
    pub catalog: bool,
}

impl Plan {
    pub fn total(&self) -> u64 {
        self.downloads.iter().filter_map(|d| d.size).sum()
    }
}

fn size_of(name: &str) -> Option<u64> {
    kor_components::info(name).and_then(|s| s.size)
}

fn missing_component(name: &str) -> bool {
    kor_components::path(name).is_none()
}

/// Work out the runtime, models and downloads. No download happens here.
///
/// # Errors
/// When the runtime or a model name is unknown.
pub async fn plan(
    opts: &InitOptions,
    cfg: &kor_config::Offline,
    reg: &kor_providers::Registry,
) -> Result<Plan, String> {
    let found = servers(reg).await;
    let runtime = match &opts.runtime {
        Some(r) => runtime_id(r),
        None if !cfg.runtime.is_empty() => cfg.runtime.clone(),
        None => [OLLAMA, LMSTUDIO, LLAMACPP]
            .into_iter()
            .find(|id| found.iter().any(|s| s.provider == *id))
            .or_else(|| found.first().map(|s| s.provider.as_str()))
            .unwrap_or(LLAMACPP)
            .to_owned(),
    };
    let m = reg
        .get(&runtime)
        .ok_or_else(|| format!("unknown local runtime `{runtime}`"))?;
    if m.category != kor_providers::Category::Local {
        return Err(format!("`{runtime}` is not a local runtime"));
    }
    let running = found.iter().find(|s| s.provider == runtime);
    let endpoint = running.map_or_else(|| endpoint_of(reg, &runtime), |s| s.base_url.clone());
    let mut downloads = Vec::new();
    let models: Vec<String> = if runtime == LLAMACPP {
        let names: Vec<String> = opts.models.iter().map(|m| component_name(m)).collect();
        for n in &names {
            if kor_components::info(n).is_none() {
                let all: Vec<_> = gguf_models().into_iter().map(|c| c.name).collect();
                return Err(format!(
                    "unknown model `{n}`; choose from {}",
                    all.join(", ")
                ));
            }
        }
        if running.is_none() && missing_component(RUNTIME_COMPONENT) {
            downloads.push(Download {
                step: "runtime",
                name: RUNTIME_COMPONENT.into(),
                size: size_of(RUNTIME_COMPONENT),
            });
        }
        for n in names.iter().filter(|n| missing_component(n)) {
            downloads.push(Download {
                step: "model",
                name: n.clone(),
                size: size_of(n),
            });
        }
        names
    } else if runtime == OLLAMA {
        let have = running.map(|s| s.models.as_slice()).unwrap_or_default();
        for tag in opts.models.iter().filter(|t| !have.contains(t)) {
            downloads.push(Download {
                step: "model",
                name: tag.clone(),
                size: ollama_size(tag).await,
            });
        }
        opts.models.clone()
    } else {
        // LM Studio and other servers download models themselves.
        let have = running.map(|s| s.models.as_slice()).unwrap_or_default();
        if let Some(m) = opts.models.iter().find(|m| !have.contains(m)) {
            return Err(format!(
                "`{m}` is not loaded in {runtime}; download it there, then run init again"
            ));
        }
        opts.models.clone()
    };
    if opts.embedding {
        for n in [EMBEDDING_TOKENIZER, EMBEDDING] {
            if missing_component(n) {
                downloads.push(Download {
                    step: "embedding",
                    name: n.into(),
                    size: size_of(n),
                });
            }
        }
    }
    let default_model = opts
        .default_model
        .clone()
        .or_else(|| models.first().cloned())
        .map(|m| model_ref(&runtime, &m))
        .or_else(|| (!cfg.model.is_empty()).then(|| cfg.model.clone()))
        .or_else(|| {
            running
                .and_then(|s| s.models.first())
                .map(|m| model_ref(&runtime, m))
        })
        .unwrap_or_default();
    Ok(Plan {
        runtime,
        endpoint,
        models,
        default_model,
        downloads,
        embedding: opts.embedding,
        catalog: opts.catalog,
    })
}

/// `qwen3-4b` / `model-qwen3-4b` on llama.cpp → `local-llamacpp/qwen3-4b`; others keep the id.
pub fn model_ref(runtime: &str, model: &str) -> String {
    if model.contains('/') && model.split_once('/').is_some_and(|(p, _)| p == runtime) {
        return model.to_owned();
    }
    let m = if runtime == LLAMACPP {
        alias(model)
    } else {
        model
    };
    format!("{runtime}/{m}")
}

fn progress_event<'a>(step: &'static str, emit: Emit<'a>) -> impl Fn(Progress) + Send + Sync + 'a {
    move |p: Progress| {
        emit(Event {
            step,
            name: p.name,
            state: "progress",
            done: p.done,
            total: p.total,
            message: None,
        });
    }
}

async fn ensure_component(step: &'static str, name: &str, emit: Emit<'_>) -> Result<(), String> {
    if !missing_component(name) {
        emit(Event::new(step, name, "skip"));
        return Ok(());
    }
    emit(Event::new(step, name, "start"));
    let progress = progress_event(step, emit);
    match kor_components::ensure(name, &progress).await {
        Ok(_) => {
            emit(Event::new(step, name, "done"));
            Ok(())
        }
        Err(e) => {
            emit(Event::new(step, name, "error").msg(e.clone()));
            Err(e)
        }
    }
}

/// Result of [`init`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub runtime: String,
    pub endpoint: String,
    pub model: String,
    /// Non-fatal problems (embedding or catalog step).
    pub warnings: Vec<String>,
}

/// Download and verify everything in `plan` (the user agreed), then write `[offline]` to
/// `user_config`. Every download resumes where an earlier attempt stopped.
///
/// # Errors
/// When the runtime or a model cannot be installed, or the config cannot be written.
pub async fn init(plan: &Plan, user_config: &Path, emit: Emit<'_>) -> Result<Report, String> {
    let mut warnings = Vec::new();
    if plan.runtime == LLAMACPP {
        if plan.downloads.iter().any(|d| d.step == "runtime") {
            ensure_component("runtime", RUNTIME_COMPONENT, emit).await?;
        }
        for m in &plan.models {
            ensure_component("model", m, emit).await?;
        }
    } else if plan.runtime == OLLAMA {
        for d in plan.downloads.iter().filter(|d| d.step == "model") {
            if crate::privacy_mode() {
                return Err("privacy/offline mode is on; `ollama pull` needs the internet".into());
            }
            ollama_pull(&plan.endpoint, &d.name, emit).await?;
        }
    }
    if plan.embedding {
        for n in [EMBEDDING_TOKENIZER, EMBEDDING] {
            if let Err(e) = ensure_component("embedding", n, emit).await {
                warnings.push(e);
                break;
            }
        }
    }
    if plan.catalog {
        emit(Event::new("catalog", "model-catalog", "start"));
        match snapshot_catalog().await {
            Ok(()) => emit(Event::new("catalog", "model-catalog", "done")),
            Err(e) => {
                emit(Event::new("catalog", "model-catalog", "error").msg(e.clone()));
                warnings.push(e);
            }
        }
    }
    emit(Event::new("config", "offline", "start"));
    write_config(
        user_config,
        &plan.runtime,
        &plan.endpoint,
        &plan.default_model,
    )?;
    emit(Event::new("config", "offline", "done"));
    Ok(Report {
        runtime: plan.runtime.clone(),
        endpoint: plan.endpoint.clone(),
        model: plan.default_model.clone(),
        warnings,
    })
}

/// `[offline] runtime/endpoint/model` in the user's config file.
///
/// # Errors
/// When the file cannot be written.
pub fn write_config(file: &Path, runtime: &str, endpoint: &str, model: &str) -> Result<(), String> {
    for (k, v) in [
        ("offline.runtime", runtime),
        ("offline.endpoint", endpoint),
        ("offline.model", model),
    ] {
        if !v.is_empty() {
            kor_config::set_string(file, k, v).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Refresh the catalog when it is old (needs the network) and keep a durable copy.
async fn snapshot_catalog() -> Result<(), String> {
    let (Some(cache), Some(snap)) = (catalog_cache(), catalog_snapshot()) else {
        return Err("no data folder".into());
    };
    let opts = mc::Options {
        offline: crate::privacy_mode(),
        refresh: false,
    };
    mc::load(Some(&cache), &mc::Endpoints::default(), opts)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(d) = snap.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&cache, &snap).map_err(|e| e.to_string())?;
    Ok(())
}

fn http(timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut b = kor_core::net::client_builder()
        .connect_timeout(Duration::from_secs(10))
        .user_agent(format!("kor/{}", kor_core::VERSION));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    b.build().map_err(|e| e.to_string())
}

/// Download size of an Ollama library tag (sum of its layers), from the public registry.
async fn ollama_size(tag: &str) -> Option<u64> {
    if crate::privacy_mode() {
        return None;
    }
    let (name, ver) = tag.split_once(':').unwrap_or((tag, "latest"));
    let path = if name.contains('/') {
        name.to_owned()
    } else {
        format!("library/{name}")
    };
    let url = format!("https://registry.ollama.ai/v2/{path}/manifests/{ver}");
    let v: serde_json::Value = http(Some(Duration::from_secs(15)))
        .ok()?
        .get(url)
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())?;
    let layers = v.get("layers")?.as_array()?;
    Some(layers.iter().filter_map(|l| l.get("size")?.as_u64()).sum())
}

/// `ollama pull <tag>` through the local Ollama API (`POST /api/pull`, NDJSON progress).
/// Ollama verifies the layer digests itself.
async fn ollama_pull(endpoint: &str, tag: &str, emit: Emit<'_>) -> Result<(), String> {
    let base = endpoint.trim_end_matches('/').trim_end_matches("/v1");
    emit(Event::new("model", tag, "start"));
    let res = http(None)?
        .post(format!("{base}/api/pull"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "model": tag, "stream": true }).to_string())
        .send()
        .await
        .map_err(|e| format!("Ollama: {e}"))?;
    if !res.status().is_success() {
        let e = format!("Ollama: HTTP {} for {tag}", res.status().as_u16());
        emit(Event::new("model", tag, "error").msg(e.clone()));
        return Err(e);
    }
    let mut stream = res.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else {
                continue;
            };
            if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                let e = format!("Ollama: {e}");
                emit(Event::new("model", tag, "error").msg(e.clone()));
                return Err(e);
            }
            let mut ev = Event::new("model", tag, "progress");
            ev.done = v
                .get("completed")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            ev.total = v.get("total").and_then(serde_json::Value::as_u64);
            ev.message = v.get("status").and_then(|s| s.as_str()).map(str::to_owned);
            let success = ev.message.as_deref() == Some("success");
            emit(ev);
            if success {
                emit(Event::new("model", tag, "done"));
                return Ok(());
            }
        }
    }
    Err(format!(
        "Ollama stopped before `{tag}` was complete; run init again to continue"
    ))
}

// ---------------------------------------------------------------------------------------------
// Serving with llama-server
// ---------------------------------------------------------------------------------------------

/// A `llama-server` Kor started; stopped when dropped.
#[derive(Debug)]
pub struct LocalServer {
    child: std::process::Child,
    pub model: String,
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn answers(url: &str) -> bool {
    let Ok(c) = http(Some(PROBE)) else {
        return false;
    };
    c.get(url)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Start `llama-server` with the configured model when llama.cpp is the runtime and nothing
/// answers on its port yet. `Ok(None)`: another runtime, or the server already runs.
///
/// # Errors
/// When the component or model is missing, or the server does not come up.
pub async fn serve(cfg: &kor_config::Offline) -> Result<Option<LocalServer>, String> {
    let Some((provider, model)) = cfg.model.split_once('/') else {
        return Ok(None);
    };
    if provider != LLAMACPP {
        return Ok(None);
    }
    let health = format!("http://127.0.0.1:{LLAMA_PORT}/health");
    if answers(&health).await {
        return Ok(None);
    }
    let exe = kor_components::path(RUNTIME_COMPONENT)
        .ok_or("llama-server is not installed; run `kor offline init` or import a bundle")?;
    let comp = component_name(model);
    let weights = kor_components::path(&comp).ok_or_else(|| {
        format!("model `{comp}` is not installed; run `kor offline init --models {model}`")
    })?;
    let log = dir().map(|d| {
        let _ = std::fs::create_dir_all(&d);
        d.join("llama-server.log")
    });
    let out = |p: &Option<PathBuf>| {
        p.as_ref()
            .and_then(|p| std::fs::File::create(p).ok())
            .map_or_else(std::process::Stdio::null, std::process::Stdio::from)
    };
    let err_log = log.as_ref().map(|p| p.with_extension("err.log"));
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("-m")
        .arg(&weights)
        .args(["--host", "127.0.0.1", "--port", &LLAMA_PORT.to_string()])
        .args(["-a", model, "--jinja", "-c", "32768"])
        .stdin(std::process::Stdio::null())
        .stdout(out(&log))
        .stderr(out(&err_log));
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    kor_platform::NoWindow::no_window(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("llama-server: {e}"))?;
    let mut server = LocalServer {
        child,
        model: model.to_owned(),
    };
    // Loading a few GB of weights takes a while on a slow disk.
    for _ in 0..240 {
        if let Ok(Some(st)) = server.child.try_wait() {
            return Err(format!(
                "llama-server stopped ({st}); see {}",
                err_log.map(|p| p.display().to_string()).unwrap_or_default()
            ));
        }
        if answers(&health).await {
            return Ok(Some(server));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("llama-server did not start within two minutes".into())
}

// ---------------------------------------------------------------------------------------------
// Air-gap bundle
// ---------------------------------------------------------------------------------------------

/// `win-x64` → `windows-x86_64`; component targets pass through.
pub fn platform(name: &str) -> Option<String> {
    let t = match name {
        "win-x64" | "windows-x64" => "windows-x86_64",
        "win-arm64" | "windows-arm64" => "windows-aarch64",
        "linux-x64" => "linux-x86_64",
        "linux-arm64" => "linux-aarch64",
        "macos-x64" | "mac-x64" => "macos-x86_64",
        "macos-arm64" | "mac-arm64" => "macos-aarch64",
        t if [
            "windows-x86_64",
            "windows-aarch64",
            "linux-x86_64",
            "linux-aarch64",
            "macos-x86_64",
            "macos-aarch64",
        ]
        .contains(&t) =>
        {
            t
        }
        _ => return None,
    };
    Some(t.to_owned())
}

fn cli_name(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "kor.exe"
    } else {
        "kor"
    }
}

/// This machine's target without `-musl` (bundles carry glibc/musl-neutral archives).
fn this_platform() -> String {
    kor_components::target()
        .trim_end_matches("-musl")
        .to_owned()
}

/// The `kor` CLI next to the running program (the desktop app ships it beside itself).
fn current_cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if exe.file_stem().is_some_and(|s| s == "kor") {
        return Some(exe);
    }
    let sib = exe.with_file_name(kor_platform::exe("kor"));
    sib.is_file().then_some(sib)
}

#[derive(Debug, Clone, Default)]
pub struct BundleOptions {
    pub out: PathBuf,
    /// `win-x64`, `linux-arm64`, … (default: this machine).
    pub platform: Option<String>,
    /// GGUF models (`qwen3-4b`); default: the configured llama.cpp model.
    pub models: Vec<String>,
    /// The `kor` executable to include (default: this one, same platform only).
    pub cli: Option<PathBuf>,
    /// A desktop installer to include as it is.
    pub desktop: Option<PathBuf>,
    pub embedding: bool,
}

/// Components a bundle carries, in install order.
fn bundle_components(models: &[String], embedding: bool) -> Vec<String> {
    let mut v = vec![RUNTIME_COMPONENT.to_owned()];
    v.extend(models.iter().map(|m| component_name(m)));
    if embedding {
        v.push(EMBEDDING_TOKENIZER.into());
        v.push(EMBEDDING.into());
    }
    v
}

/// The CLI (+ signature), a desktop installer and the catalog snapshot for a bundle.
fn extra_sources(
    opts: &BundleOptions,
    platform: &str,
    sources: &mut Vec<Source>,
) -> Result<(), String> {
    let cli = match &opts.cli {
        Some(p) => Some(p.clone()),
        None if platform == this_platform() => current_cli(),
        None => None,
    };
    if let Some(cli) = cli {
        let name = cli_name(platform);
        let sig = PathBuf::from(format!("{}.minisig", cli.display()));
        if sig.is_file() {
            sources.push(Source {
                path: format!("bin/{name}.minisig"),
                file: sig,
                kind: Kind::Signature,
                component: None,
            });
        }
        sources.push(Source {
            path: format!("bin/{name}"),
            file: cli,
            kind: Kind::Cli,
            component: None,
        });
    }
    if let Some(d) = &opts.desktop {
        let f = d
            .file_name()
            .ok_or("desktop installer path has no file name")?
            .to_string_lossy()
            .into_owned();
        sources.push(Source {
            path: format!("desktop/{f}"),
            file: d.clone(),
            kind: Kind::Desktop,
            component: None,
        });
    }
    if let Some(c) = [catalog_cache(), catalog_snapshot()]
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
    {
        sources.push(Source {
            path: "catalog/model-catalog.json".into(),
            file: c,
            kind: Kind::Catalog,
            component: None,
        });
    }
    Ok(())
}

/// Collect (downloading what is missing, verified) and write one air-gap archive.
///
/// # Errors
/// On unknown platform/model, a failed download, or a write error.
pub async fn create_bundle(
    opts: &BundleOptions,
    cfg: &kor_config::Offline,
    emit: Emit<'_>,
) -> Result<bundle::Manifest, String> {
    let platform = match &opts.platform {
        Some(p) => platform(p).ok_or_else(|| format!("unknown platform `{p}`"))?,
        None => this_platform(),
    };
    let mut models = opts.models.clone();
    if models.is_empty()
        && let Some((LLAMACPP, m)) = cfg.model.split_once('/')
    {
        models.push(m.to_owned());
    }
    if models.is_empty() {
        return Err("choose at least one model (--models qwen3-4b)".into());
    }
    let store = Store::open().ok_or("no data folder for Kor components")?;
    let cache = dir().ok_or("no data folder")?.join("cache");
    let mut sources = Vec::new();
    for name in bundle_components(&models, opts.embedding) {
        emit(Event::new("bundle", &name, "start"));
        let progress = progress_event("bundle", emit);
        let file = store
            .artifact_file(&name, &platform, &cache, &progress)
            .await
            .inspect_err(|e| emit(Event::new("bundle", &name, "error").msg(e.clone())))?;
        let fname = file
            .file_name()
            .map_or_else(|| name.clone(), |f| f.to_string_lossy().into_owned());
        sources.push(Source {
            path: format!("components/{name}/{fname}"),
            file,
            kind: Kind::Component,
            component: Some(name.clone()),
        });
        emit(Event::new("bundle", &name, "done"));
    }
    extra_sources(opts, &platform, &mut sources)?;
    let meta = bundle::Manifest {
        format: bundle::FORMAT,
        kor_version: kor_core::VERSION.to_owned(),
        platform,
        created: kor_providers::catalog::now(),
        runtime: LLAMACPP.into(),
        model: model_ref(LLAMACPP, &models[0]),
        files: Vec::new(),
    };
    let out = opts.out.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Progress>();
    let job = tokio::task::spawn_blocking(move || {
        bundle::write(&out, meta, &sources, &move |p| {
            let _ = tx.send(p);
        })
    });
    // Forward progress from the writer thread.
    let forward = async {
        loop {
            while let Ok(p) = rx.try_recv() {
                progress_event("bundle", emit)(p);
            }
            if job.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    forward.await;
    let m = job.await.map_err(|e| e.to_string())??;
    while let Ok(p) = rx.try_recv() {
        progress_event("bundle", emit)(p);
    }
    emit(Event::new(
        "bundle",
        &opts.out.display().to_string(),
        "done",
    ));
    Ok(m)
}

/// What an import did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub platform: String,
    pub kor_version: String,
    pub components: Vec<String>,
    pub catalog: bool,
    /// The bundled `kor` (install it with `kor update --from <bundle>`).
    pub cli: Option<String>,
    pub desktop: Option<String>,
    pub model: String,
}

/// A bundle's manifest for the consent prompt (no files are extracted).
///
/// # Errors
/// When the file is not a valid bundle.
pub fn inspect(file: &Path) -> Result<bundle::Manifest, String> {
    bundle::read_manifest(file)
}

/// Verify a bundle and install it into Kor's own folders (components, catalog, `[offline]`
/// config). Blocking; an interrupted import continues on the next call.
///
/// # Errors
/// On a damaged or altered file, another platform's bundle, or a write error.
pub fn import(file: &Path, user_config: Option<&Path>, emit: Emit<'_>) -> Result<Imported, String> {
    let m = bundle::read_manifest(file)?;
    let here = this_platform();
    if m.platform != here {
        return Err(format!(
            "this bundle is for {}, this computer is {here}",
            m.platform
        ));
    }
    let root = dir().ok_or("no data folder")?;
    let staging = root
        .join("import")
        .join(format!("{}-{}", m.platform, m.created));
    emit(Event::new("import", "verify", "start"));
    let progress = progress_event("import", emit);
    bundle::extract(file, &staging, &progress)?;
    emit(Event::new("import", "verify", "done"));
    let store = Store::open().ok_or("no data folder for Kor components")?;
    let target = kor_components::target();
    let local = |p: &str| p.split('/').fold(staging.clone(), |a, s| a.join(s));
    let mut out = Imported {
        platform: m.platform.clone(),
        kor_version: m.kor_version.clone(),
        components: Vec::new(),
        catalog: false,
        cli: None,
        desktop: None,
        model: m.model.clone(),
    };
    for e in &m.files {
        match (e.kind, &e.component) {
            (Kind::Component, Some(name)) => {
                emit(Event::new("import", name, "start"));
                store
                    .add_from(name, &target, &local(&e.path))
                    .inspect_err(|err| {
                        emit(Event::new("import", name, "error").msg(err.clone()));
                    })?;
                emit(Event::new("import", name, "done"));
                out.components.push(name.clone());
            }
            (Kind::Catalog, _) => {
                for dest in [catalog_snapshot(), catalog_cache()].into_iter().flatten() {
                    if let Some(d) = dest.parent() {
                        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                    }
                    std::fs::copy(local(&e.path), &dest).map_err(|e| e.to_string())?;
                }
                out.catalog = true;
            }
            (Kind::Cli | Kind::Signature | Kind::Desktop, _) => {
                let sub = if e.kind == Kind::Desktop {
                    "desktop"
                } else {
                    "bin"
                };
                let name = e.path.rsplit('/').next().unwrap_or(&e.path);
                let dest = root.join(sub).join(name);
                std::fs::create_dir_all(root.join(sub)).map_err(|e| e.to_string())?;
                std::fs::copy(local(&e.path), &dest).map_err(|e| e.to_string())?;
                let shown = Some(dest.display().to_string());
                match e.kind {
                    Kind::Cli => out.cli = shown,
                    Kind::Desktop => out.desktop = shown,
                    _ => {}
                }
            }
            (Kind::Component, None) => {}
        }
    }
    if let Some(f) = user_config {
        let endpoint = if m.runtime == LLAMACPP {
            LLAMA_ENDPOINT
        } else {
            ""
        };
        write_config(f, &m.runtime, endpoint, &m.model)?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    emit(Event::new("import", "done", "done"));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(model: &str) -> kor_config::Offline {
        kor_config::Offline {
            enabled: true,
            runtime: model.split('/').next().unwrap_or_default().into(),
            endpoint: String::new(),
            model: model.into(),
        }
    }

    fn server(provider: &str, models: &[&str]) -> Server {
        Server {
            provider: provider.into(),
            name: provider.into(),
            base_url: String::new(),
            models: models.iter().map(|m| (*m).to_owned()).collect(),
        }
    }

    fn gguf(name: &str, installed: bool) -> ModelChoice {
        ModelChoice {
            name: name.into(),
            about: String::new(),
            size: Some(1),
            installed,
            model_ref: String::new(),
        }
    }

    #[test]
    fn readiness_for_llamacpp_needs_runtime_and_file() {
        let c = cfg("local-llamacpp/qwen3-4b");
        let have = [gguf("model-qwen3-4b", true)];
        assert_eq!(readiness(&c, &[], &have, true), (true, vec![]));
        assert_eq!(readiness(&c, &[], &have, false), (false, vec!["runtime"]));
        let none = [gguf("model-qwen3-4b", false)];
        assert_eq!(
            readiness(&c, &[], &none, false),
            (false, vec!["runtime", "model-file"])
        );
        // An already running llama.cpp server is enough.
        let up = [server(LLAMACPP, &["x"])];
        assert_eq!(readiness(&c, &up, &none, false), (true, vec![]));
    }

    #[test]
    fn readiness_for_ollama_needs_the_model_served() {
        let c = cfg("local/qwen3:4b");
        assert_eq!(readiness(&c, &[], &[], false), (false, vec!["runtime"]));
        let up = [server(OLLAMA, &["qwen3:4b"])];
        assert_eq!(readiness(&c, &up, &[], false), (true, vec![]));
        let other = [server(OLLAMA, &["llama3:8b"])];
        assert_eq!(
            readiness(&c, &other, &[], false),
            (true, vec!["model-file"])
        );
        assert_eq!(readiness(&cfg(""), &up, &[], false).1, vec!["model"]);
    }

    #[test]
    fn names_and_platforms() {
        assert_eq!(component_name("qwen3-4b"), "model-qwen3-4b");
        assert_eq!(component_name("model-qwen3-4b"), "model-qwen3-4b");
        assert_eq!(alias("model-qwen3-4b"), "qwen3-4b");
        assert_eq!(
            model_ref(LLAMACPP, "model-qwen3-4b"),
            "local-llamacpp/qwen3-4b"
        );
        assert_eq!(model_ref(OLLAMA, "qwen3:4b"), "local/qwen3:4b");
        assert_eq!(model_ref(OLLAMA, "local/qwen3:4b"), "local/qwen3:4b");
        assert_eq!(runtime_id("Ollama"), OLLAMA);
        assert_eq!(runtime_id("llamacpp"), LLAMACPP);
        assert_eq!(platform("win-x64").as_deref(), Some("windows-x86_64"));
        assert_eq!(platform("macos-aarch64").as_deref(), Some("macos-aarch64"));
        assert!(platform("plan9").is_none());
        assert_eq!(cli_name("windows-aarch64"), "kor.exe");
        assert_eq!(
            bundle_components(&["qwen3-4b".into()], true),
            [
                RUNTIME_COMPONENT,
                "model-qwen3-4b",
                EMBEDDING_TOKENIZER,
                EMBEDDING
            ]
        );
    }

    #[test]
    fn default_model_only_in_offline_mode() {
        let mut c = kor_config::Config::default();
        c.offline.model = "local-llamacpp/qwen3-4b".into();
        if !crate::offline_mode() {
            assert_eq!(default_model(&c), None);
        }
        c.offline.enabled = true;
        assert_eq!(
            default_model(&c).as_deref(),
            Some("local-llamacpp/qwen3-4b")
        );
    }

    #[test]
    fn config_is_written_to_the_user_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        write_config(&f, LLAMACPP, LLAMA_ENDPOINT, "local-llamacpp/qwen3-4b").unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        let v: kor_config::Config = toml::from_str(&text).unwrap();
        assert_eq!(v.offline.runtime, LLAMACPP);
        assert_eq!(v.offline.endpoint, LLAMA_ENDPOINT);
        assert_eq!(v.offline.model, "local-llamacpp/qwen3-4b");
        assert!(!v.offline.enabled);
    }

    #[test]
    fn import_rejects_another_platform_before_extracting() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x"), b"x").unwrap();
        let other = if this_platform() == "linux-x86_64" {
            "windows-x86_64"
        } else {
            "linux-x86_64"
        };
        let meta = bundle::Manifest {
            format: bundle::FORMAT,
            kor_version: "0".into(),
            platform: other.into(),
            created: 1,
            runtime: LLAMACPP.into(),
            model: "local-llamacpp/x".into(),
            files: Vec::new(),
        };
        let out = dir.path().join("b.zip");
        bundle::write(
            &out,
            meta,
            &[Source {
                path: "catalog/model-catalog.json".into(),
                file: dir.path().join("x"),
                kind: Kind::Catalog,
                component: None,
            }],
            &|_| {},
        )
        .unwrap();
        let err = import(&out, None, &|_| {}).unwrap_err();
        assert!(err.contains("this bundle is for"), "{err}");
    }
}
