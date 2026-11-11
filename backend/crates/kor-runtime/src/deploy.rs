//! `kor deploy <env>` and `kor deploy risk-check <env>` (spec §8.6, ADR 0041).
//!
//! Reads `docker-compose.yml` / `compose.yaml`, Kubernetes manifests or a lone `Dockerfile`, plus an
//! optional `.kor/deploy.toml` (environments → compose files, project, SSH host from the user's
//! `connections.toml`, kube context/namespace, health URLs, blue-green, rollback policy). The
//! services changed since the commit recorded for the environment (`.kor/deploy/state.json`) are
//! the only ones rebuilt and restarted. Every command that changes something is a [`Step`] the
//! caller approves; destructive steps carry `destructive = true` (shown red, ADR 0030). Reads
//! (`docker compose ps`, `docker inspect`, `kubectl rollout status`) run without asking.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use kor_platform::NoWindow as _;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const K8S_DIRS: [&str; 4] = ["k8s", "kubernetes", "deploy/k8s", "manifests"];
const DEFAULT_PEAK: &str = "09:00-18:00";
const MAX_LOG: usize = 2000;
const MAX_FILES: usize = 500;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

// ---------------------------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------------------------

/// What happens when a health check fails after the new version started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rollback {
    /// Roll back without asking.
    Auto,
    /// Offer the rollback steps for approval.
    #[default]
    Ask,
    Off,
}

/// `.kor/deploy.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployFile {
    /// Daily `HH:MM-HH:MM` local window that `risk-check` warns about (default 09:00-18:00).
    #[serde(default)]
    pub peak_hours: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, EnvConfig>,
}

/// One `[env.<name>]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EnvConfig {
    /// Compose files (`-f`), relative to the project root. Empty: auto-detected.
    pub compose: Vec<String>,
    /// Compose project name (`-p`).
    pub project: Option<String>,
    /// SSH connection id from the user's `connections.toml`: `DOCKER_HOST=ssh://…`.
    pub ssh: Option<String>,
    /// `kubectl --context`.
    pub context: Option<String>,
    /// `kubectl -n`.
    pub namespace: Option<String>,
    /// Kubernetes manifest files or folders. Empty: `k8s/`, `kubernetes/`, … when no compose file.
    pub manifests: Vec<String>,
    /// Service → URL that must answer 2xx/3xx after the deploy.
    pub health: BTreeMap<String, String>,
    /// Seconds to wait for health (default 120).
    pub health_timeout: Option<u64>,
    /// Compose blue-green: two projects `<project>-blue` / `<project>-green`.
    pub blue_green: bool,
    /// Command run after the new color is healthy (point the proxy at it); gets
    /// `KOR_DEPLOY_COLOR` and `KOR_DEPLOY_PROJECT`.
    pub switch: Option<String>,
    pub rollback: Rollback,
    pub peak_hours: Option<String>,
}

fn valid_name(s: &str) -> bool {
    (1..=100).contains(&s.len())
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@' | b':' | b'/')
        })
}

fn valid_rel(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !p.starts_with('-')
        && path.is_relative()
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Read `.kor/deploy.toml` (missing file: defaults).
pub fn load(root: &Path) -> Result<DeployFile, String> {
    let path = root.join(".kor").join("deploy.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(DeployFile::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let f: DeployFile = toml::from_str(&text).map_err(|e| format!(".kor/deploy.toml: {e}"))?;
    for (name, e) in &f.env {
        let names = [&e.project, &e.ssh, &e.context, &e.namespace];
        if !valid_name(name)
            || names
                .iter()
                .any(|n| n.as_deref().is_some_and(|n| !valid_name(n)))
        {
            return Err(format!(
                ".kor/deploy.toml: env.{name}: invalid name, project, ssh, context or namespace"
            ));
        }
        if e.compose.iter().chain(&e.manifests).any(|p| !valid_rel(p)) {
            return Err(format!(
                ".kor/deploy.toml: env.{name}: files must be relative paths inside the project"
            ));
        }
        if e.health
            .values()
            .any(|u| !(u.starts_with("http://") || u.starts_with("https://")))
        {
            return Err(format!(
                ".kor/deploy.toml: env.{name}: health URLs must be http(s)"
            ));
        }
    }
    Ok(f)
}

/// Environment names: the configured ones, or `local` (auto-detected files) without a config.
pub fn envs(root: &Path) -> Result<Vec<String>, String> {
    let f = load(root)?;
    Ok(if f.env.is_empty() {
        vec!["local".to_owned()]
    } else {
        f.env.keys().cloned().collect()
    })
}

fn env_config(f: &DeployFile, env: &str) -> Result<EnvConfig, String> {
    match f.env.get(env) {
        Some(c) => Ok(c.clone()),
        None if env == "local" => Ok(EnvConfig::default()),
        None => Err(format!(
            "environment `{env}` is not in .kor/deploy.toml (known: {}); add [env.{env}]",
            if f.env.is_empty() {
                "local".to_owned()
            } else {
                f.env.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Compose and Kubernetes files
// ---------------------------------------------------------------------------------------------

/// A compose service and the project paths that feed it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Service {
    pub name: String,
    /// Build context relative to the project root (`""` = the root).
    pub context: Option<String>,
    pub dockerfile: Option<String>,
    pub image: Option<String>,
    /// Bind-mounted host paths relative to the root.
    pub mounts: Vec<String>,
    pub healthcheck: bool,
}

/// Lexically normalized `/`-path relative to the root (`a/./b/../c` → `a/c`).
fn norm(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in base.split('/').chain(rel.split(['/', '\\'])) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn yaml_str(v: &serde_yaml::Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

/// Services of a compose file located in `dir` (relative to the root).
pub fn parse_compose(text: &str, dir: &str) -> Result<Vec<Service>, String> {
    let doc: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    let Some(services) = doc.get("services").and_then(serde_yaml::Value::as_mapping) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (k, v) in services {
        let Some(name) = k.as_str() else { continue };
        let mut s = Service {
            name: name.to_owned(),
            image: v.get("image").and_then(yaml_str),
            healthcheck: v.get("healthcheck").is_some_and(|h| {
                !h.get("disable")
                    .and_then(serde_yaml::Value::as_bool)
                    .unwrap_or(false)
            }),
            ..Service::default()
        };
        match v.get("build") {
            Some(b) if b.is_string() => s.context = yaml_str(b).map(|c| norm(dir, &c)),
            Some(b) => {
                let ctx = b
                    .get("context")
                    .and_then(yaml_str)
                    .unwrap_or_else(|| ".".into());
                let ctx = norm(dir, &ctx);
                s.dockerfile = b
                    .get("dockerfile")
                    .and_then(yaml_str)
                    .map(|d| norm(&ctx, &d));
                s.context = Some(ctx);
            }
            None => {}
        }
        for vol in v
            .get("volumes")
            .and_then(serde_yaml::Value::as_sequence)
            .into_iter()
            .flatten()
        {
            let src = match vol {
                serde_yaml::Value::String(s) => s.split(':').next().map(str::to_owned),
                m => (m.get("type").and_then(serde_yaml::Value::as_str) == Some("bind"))
                    .then(|| m.get("source").and_then(yaml_str))
                    .flatten(),
            };
            if let Some(src) = src.filter(|s| s.starts_with('.')) {
                s.mounts.push(norm(dir, &src));
            }
        }
        out.push(s);
    }
    Ok(out)
}

/// `(kind, name)` of the rollout-able workloads in a manifest file.
pub fn workloads(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(text) {
        let Ok(v) = serde_yaml::Value::deserialize(doc) else {
            continue;
        };
        let kind = v
            .get("kind")
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or_default();
        let name = v
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(serde_yaml::Value::as_str);
        if let (true, Some(name)) = (
            matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet"),
            name,
        ) && valid_name(name)
        {
            out.push((kind.to_ascii_lowercase(), name.to_owned()));
        }
    }
    out
}

fn yaml_files(root: &Path, entries: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for e in entries {
        let p = root.join(e);
        if p.is_file() {
            out.insert(norm("", e));
        } else if p.is_dir() {
            for f in ignore::WalkBuilder::new(&p).hidden(false).build().flatten() {
                let fp = f.path();
                if fp.extension().is_some_and(|x| x == "yaml" || x == "yml")
                    && let Ok(rel) = fp.strip_prefix(root)
                {
                    out.insert(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Services touched by `changed` files. `None` (no recorded deploy) = all of them.
pub fn changed_services(
    services: &[Service],
    config_files: &[String],
    changed: Option<&[String]>,
) -> Vec<String> {
    let all = || services.iter().map(|s| s.name.clone()).collect();
    let Some(changed) = changed else { return all() };
    let under =
        |f: &str, dir: &str| dir.is_empty() || f == dir || f.starts_with(&format!("{dir}/"));
    let config_dirs: Vec<&str> = config_files
        .iter()
        .map(|c| c.rsplit_once('/').map_or("", |(d, _)| d))
        .collect();
    if changed
        .iter()
        .any(|f| config_files.contains(f) || config_dirs.iter().any(|d| *f == norm(d, ".env")))
    {
        return all();
    }
    services
        .iter()
        .filter(|s| {
            changed.iter().any(|f| {
                s.context.as_deref().is_some_and(|c| under(f, c))
                    || s.dockerfile.as_deref() == Some(f.as_str())
                    || s.mounts.iter().any(|m| under(f, m))
            })
        })
        .map(|s| s.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(kor_platform::exe("git"));
    cmd.arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    cmd.no_window();
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn valid_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `HEAD` commit, if `root` is a git work tree.
pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| valid_sha(s))
}

/// Files changed since `since` (committed, uncommitted and untracked). `None`: unknown (no
/// recorded deploy, commit gone, not a repository) — everything counts as changed.
pub fn changed_files(root: &Path, since: Option<&str>) -> Option<Vec<String>> {
    let since = since.filter(|s| valid_sha(s))?;
    git(root, &["cat-file", "-e", &format!("{since}^{{commit}}")])?;
    let diff = git(root, &["diff", "--name-only", since, "--"])?;
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    let mut set: BTreeSet<String> = BTreeSet::new();
    for l in diff
        .lines()
        .chain(untracked.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        set.insert(l.to_owned());
    }
    Some(set.into_iter().collect())
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub envs: BTreeMap<String, EnvState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvState {
    /// Commit of the last successful deploy.
    pub commit: Option<String>,
    /// Unix seconds.
    pub at: u64,
    #[serde(default)]
    pub services: Vec<String>,
    /// Live blue-green color.
    #[serde(default)]
    pub color: Option<String>,
    /// Last run, successful or not.
    #[serde(default)]
    pub last: Option<Report>,
}

fn state_path(root: &Path) -> PathBuf {
    root.join(".kor").join("deploy").join("state.json")
}

pub fn load_state(root: &Path) -> State {
    std::fs::read_to_string(state_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(root: &Path, s: &State) -> Result<(), String> {
    let p = state_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Remember the running image of a service (rollback).
    Snapshot,
    Build,
    Pull,
    Up,
    Apply,
    /// `docker compose ps` health / `kubectl rollout status`.
    Health,
    /// HTTP GET that must answer 2xx/3xx.
    HttpHealth,
    Switch,
    Stop,
    Rollback,
}

/// One command of a deploy. `approve`: it changes something and needs the user's (or a
/// trigger policy's) yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: usize,
    pub kind: StepKind,
    pub program: String,
    pub args: Vec<String>,
    /// Extra environment (`DOCKER_HOST`), shown in `display`.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub display: String,
    pub approve: bool,
    pub destructive: bool,
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub env: String,
    /// `compose`, `kubernetes`, `dockerfile` or `none`.
    pub target: String,
    pub head: Option<String>,
    /// Commit of the last deploy of this environment.
    pub since: Option<String>,
    pub dirty: bool,
    /// Changed files (capped); `None` = everything (first deploy).
    pub changed_files: Option<Vec<String>>,
    pub services: Vec<String>,
    pub all_services: Vec<String>,
    pub steps: Vec<Step>,
    /// Rollback commands used when a health check fails.
    pub rollback_steps: Vec<Step>,
    pub rollback: Rollback,
    pub health_timeout: u64,
    /// Blue-green: the color being deployed.
    pub color: Option<String>,
    pub notes: Vec<String>,
}

fn display(env: &[(String, String)], program: &str, args: &[String]) -> String {
    let q = |a: &str| {
        if a.is_empty() || a.contains([' ', '"', '\'', '|', '&', ';', '<', '>', '{', '}']) {
            format!("\"{}\"", a.replace('"', "\\\""))
        } else {
            a.to_owned()
        }
    };
    env.iter()
        .map(|(k, v)| format!("{k}={}", q(v)))
        .chain(std::iter::once(program.to_owned()))
        .chain(args.iter().map(|a| q(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

struct Steps {
    list: Vec<Step>,
    env: Vec<(String, String)>,
}

impl Steps {
    fn push(&mut self, kind: StepKind, program: &str, args: Vec<String>, service: Option<&str>) {
        let approve = !matches!(
            kind,
            StepKind::Snapshot | StepKind::Health | StepKind::HttpHealth
        );
        let destructive = matches!(kind, StepKind::Stop | StepKind::Rollback)
            && args.iter().any(|a| a == "down" || a == "undo" || a == "rm");
        let env = if program == "docker" {
            self.env.clone()
        } else {
            Vec::new()
        };
        self.list.push(Step {
            id: self.list.len() + 1,
            kind,
            display: if kind == StepKind::HttpHealth {
                format!("GET {}", args.join(" "))
            } else {
                display(&env, program, &args)
            },
            program: program.to_owned(),
            args,
            env,
            approve,
            destructive,
            service: service.map(str::to_owned),
        });
    }
}

fn docker_host(ssh: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let Some(id) = ssh else { return Ok(Vec::new()) };
    let file = kor_config::user_dir()
        .map(|d| d.join("connections.toml"))
        .ok_or("no user configuration directory")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let conns = kor_agent::remote::Connections::parse(&text)?;
    let h = conns
        .ssh
        .get(id)
        .ok_or_else(|| format!("no SSH connection `{id}` in {}", file.display()))?;
    let url = format!(
        "ssh://{}{}{}",
        h.user
            .as_deref()
            .map(|u| format!("{u}@"))
            .unwrap_or_default(),
        h.host,
        h.port.map(|p| format!(":{p}")).unwrap_or_default()
    );
    Ok(vec![("DOCKER_HOST".to_owned(), url)])
}

/// What `kor deploy <env>` would do, without running anything.
#[allow(clippy::too_many_lines)]
pub fn plan(root: &Path, env: &str) -> Result<Plan, String> {
    let file = load(root)?;
    let cfg = env_config(&file, env)?;
    let state = load_state(root);
    let since = state.envs.get(env).and_then(|e| e.commit.clone());
    let head = head(root);
    let dirty = git(root, &["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty());
    let changed = changed_files(root, since.as_deref());
    let mut notes = Vec::new();
    if since.is_some() && changed.is_none() {
        notes.push(
            "the recorded deploy commit is not in this repository: every service counts as changed"
                .to_owned(),
        );
    }
    if dirty {
        notes.push("uncommitted changes are deployed too; the recorded commit is HEAD".to_owned());
    }
    let compose: Vec<String> = if cfg.compose.is_empty() {
        COMPOSE_FILES
            .iter()
            .find(|f| root.join(f).is_file())
            .map(|f| vec![(*f).to_owned()])
            .unwrap_or_default()
    } else {
        cfg.compose.clone()
    };
    let manifests = if !cfg.manifests.is_empty() {
        yaml_files(root, &cfg.manifests)
    } else if compose.is_empty() {
        let dirs: Vec<String> = K8S_DIRS
            .iter()
            .filter(|d| root.join(d).is_dir())
            .map(|d| (*d).to_owned())
            .collect();
        yaml_files(root, &dirs)
    } else {
        Vec::new()
    };
    let mut steps = Steps {
        list: Vec::new(),
        env: docker_host(cfg.ssh.as_deref())?,
    };
    let mut rollback = Steps {
        list: Vec::new(),
        env: steps.env.clone(),
    };
    let timeout = cfg.health_timeout.unwrap_or(120).clamp(1, 3600);
    let mut p = Plan {
        env: env.to_owned(),
        target: "none".into(),
        head,
        since,
        dirty,
        changed_files: changed
            .as_ref()
            .map(|c| c.iter().take(MAX_FILES).cloned().collect()),
        services: Vec::new(),
        all_services: Vec::new(),
        steps: Vec::new(),
        rollback_steps: Vec::new(),
        rollback: cfg.rollback,
        health_timeout: timeout,
        color: None,
        notes,
    };
    if !compose.is_empty() {
        p.target = "compose".into();
        let mut services = Vec::new();
        for f in &compose {
            let text = std::fs::read_to_string(root.join(f)).map_err(|e| format!("{f}: {e}"))?;
            let dir = f.rsplit_once('/').map_or("", |(d, _)| d);
            services.extend(parse_compose(&text, dir).map_err(|e| format!("{f}: {e}"))?);
        }
        p.all_services = services.iter().map(|s| s.name.clone()).collect();
        let base_project = cfg.project.clone().unwrap_or_else(|| project_name(root));
        let files: Vec<String> = compose
            .iter()
            .flat_map(|f| ["-f".to_owned(), f.clone()])
            .collect();
        let dc = |project: &str, rest: &[&str]| -> Vec<String> {
            let mut a = vec!["compose".to_owned()];
            a.extend(files.iter().cloned());
            a.extend(["-p".to_owned(), project.to_owned()]);
            a.extend(rest.iter().map(|x| (*x).to_owned()));
            a
        };
        if cfg.blue_green {
            // A new color is a whole new stack: every service is built and started.
            let live = state.envs.get(env).and_then(|e| e.color.clone());
            let next = if live.as_deref() == Some("blue") {
                "green"
            } else {
                "blue"
            };
            let project = format!("{base_project}-{next}");
            p.color = Some(next.to_owned());
            p.services.clone_from(&p.all_services);
            steps.push(
                StepKind::Up,
                "docker",
                dc(&project, &["up", "-d", "--build", "--remove-orphans"]),
                None,
            );
            for svc in &services {
                steps.push(
                    StepKind::Health,
                    "docker",
                    dc(&project, &["ps", "--format", "json", &svc.name]),
                    Some(&svc.name),
                );
            }
            for (svc, url) in &cfg.health {
                steps.push(
                    StepKind::HttpHealth,
                    "",
                    vec![url.replace("{color}", next)],
                    Some(svc),
                );
            }
            if let Some(cmd) = &cfg.switch {
                let (sh, flag) = shell();
                let mut st = Steps {
                    list: Vec::new(),
                    env: Vec::new(),
                };
                st.push(
                    StepKind::Switch,
                    sh,
                    vec![flag.to_owned(), cmd.clone()],
                    None,
                );
                let mut step = st.list.remove(0);
                step.env = vec![
                    ("KOR_DEPLOY_COLOR".into(), next.into()),
                    ("KOR_DEPLOY_PROJECT".into(), project.clone()),
                ];
                step.display = display(&step.env, sh, &step.args);
                step.id = steps.list.len() + 1;
                steps.list.push(step);
            } else {
                p.notes.push(
                    "blue-green without `switch`: point your proxy at the new project yourself"
                        .into(),
                );
            }
            if let Some(old) = live {
                steps.push(
                    StepKind::Stop,
                    "docker",
                    dc(&format!("{base_project}-{old}"), &["down"]),
                    None,
                );
            }
            rollback.push(StepKind::Rollback, "docker", dc(&project, &["down"]), None);
        } else {
            p.services = changed_services(&services, &compose, changed.as_deref());
            let chosen: Vec<&Service> = services
                .iter()
                .filter(|s| p.services.contains(&s.name))
                .collect();
            let build: Vec<&str> = chosen
                .iter()
                .filter(|s| s.context.is_some())
                .map(|s| s.name.as_str())
                .collect();
            let pull: Vec<&str> = chosen
                .iter()
                .filter(|s| s.context.is_none() && s.image.is_some())
                .map(|s| s.name.as_str())
                .collect();
            if !chosen.is_empty() {
                if p.rollback != Rollback::Off {
                    for svc in &chosen {
                        steps.push(
                            StepKind::Snapshot,
                            "docker",
                            dc(&base_project, &["ps", "-q", &svc.name]),
                            Some(&svc.name),
                        );
                    }
                }
                if !build.is_empty() {
                    let mut a = vec!["build"];
                    a.extend(&build);
                    steps.push(StepKind::Build, "docker", dc(&base_project, &a), None);
                }
                if !pull.is_empty() {
                    let mut a = vec!["pull"];
                    a.extend(&pull);
                    steps.push(StepKind::Pull, "docker", dc(&base_project, &a), None);
                }
                let names: Vec<&str> = chosen.iter().map(|s| s.name.as_str()).collect();
                let mut a = vec!["up", "-d", "--no-deps"];
                a.extend(&names);
                steps.push(StepKind::Up, "docker", dc(&base_project, &a), None);
                for svc in &chosen {
                    steps.push(
                        StepKind::Health,
                        "docker",
                        dc(&base_project, &["ps", "--format", "json", &svc.name]),
                        Some(&svc.name),
                    );
                }
                for (svc, url) in cfg
                    .health
                    .iter()
                    .filter(|(k, _)| names.contains(&k.as_str()))
                {
                    steps.push(StepKind::HttpHealth, "", vec![url.clone()], Some(svc));
                }
                // Filled with the remembered image ids at run time (`docker tag <id> <image>`).
                let mut a = vec!["up", "-d", "--no-deps", "--no-build"];
                a.extend(&names);
                rollback.push(StepKind::Rollback, "docker", dc(&base_project, &a), None);
            }
        }
    } else if !manifests.is_empty() {
        p.target = "kubernetes".into();
        let kube = |rest: &[&str]| -> Vec<String> {
            let mut a = Vec::new();
            if let Some(c) = &cfg.context {
                a.extend(["--context".to_owned(), c.clone()]);
            }
            if let Some(n) = &cfg.namespace {
                a.extend(["-n".to_owned(), n.clone()]);
            }
            a.extend(rest.iter().map(|x| (*x).to_owned()));
            a
        };
        let chosen: Vec<&String> = manifests
            .iter()
            .filter(|m| changed.as_ref().is_none_or(|c| c.contains(m)))
            .collect();
        let mut rollouts = Vec::new();
        for m in &chosen {
            steps.push(StepKind::Apply, "kubectl", kube(&["apply", "-f", m]), None);
            let text = std::fs::read_to_string(root.join(m)).unwrap_or_default();
            rollouts.extend(workloads(&text));
        }
        p.all_services = manifests
            .iter()
            .flat_map(|m| workloads(&std::fs::read_to_string(root.join(m)).unwrap_or_default()))
            .map(|(_, n)| n)
            .collect();
        p.services = rollouts.iter().map(|(_, n)| n.clone()).collect();
        let t = format!("--timeout={timeout}s");
        for (kind, name) in &rollouts {
            let target = format!("{kind}/{name}");
            steps.push(
                StepKind::Health,
                "kubectl",
                kube(&["rollout", "status", &target, &t]),
                Some(name),
            );
            rollback.push(
                StepKind::Rollback,
                "kubectl",
                kube(&["rollout", "undo", &target]),
                Some(name),
            );
        }
        for (svc, url) in cfg.health.iter().filter(|(k, _)| p.services.contains(k)) {
            steps.push(StepKind::HttpHealth, "", vec![url.clone()], Some(svc));
        }
        if changed
            .as_ref()
            .is_some_and(|c| c.iter().any(|f| !manifests.contains(f)))
        {
            p.notes.push("source changes outside the manifests need new images: build and push them first (Kor applies manifests)".into());
        }
    } else if root.join("Dockerfile").is_file() {
        p.target = "dockerfile".into();
        let name = root.file_name().map_or_else(
            || "app".into(),
            |n| n.to_string_lossy().to_lowercase().replace(' ', "-"),
        );
        if changed.as_ref().is_none_or(|c| !c.is_empty()) {
            let tag = p
                .head
                .as_deref()
                .map_or("dev", |h| &h[..h.len().min(12)])
                .to_owned();
            p.services = vec![name.clone()];
            steps.push(
                StepKind::Build,
                "docker",
                vec![
                    "build".into(),
                    "-t".into(),
                    format!("{name}:{tag}"),
                    "-t".into(),
                    format!("{name}:latest"),
                    ".".into(),
                ],
                Some(&name),
            );
        }
        p.all_services = vec![name];
        p.notes.push(
            "only a Dockerfile: Kor builds the image; add a compose file or manifests to run it"
                .into(),
        );
    } else {
        return Err("no docker-compose.yml / compose.yaml, Kubernetes manifests or Dockerfile found (configure .kor/deploy.toml)".into());
    }
    p.steps = steps.list;
    p.rollback_steps = rollback.list;
    Ok(p)
}

/// Compose's default project name: the folder name with only `a-z0-9_-`, starting alphanumeric.
fn project_name(root: &Path) -> String {
    let raw = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let name = name.trim_start_matches(['_', '-']);
    if name.is_empty() {
        "app".into()
    } else {
        name.to_owned()
    }
}

fn shell() -> (&'static str, &'static str) {
    if kor_platform::os() == kor_platform::Os::Windows {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

// ---------------------------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------------------------

/// Progress of a deploy run, for the CLI and the desktop log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Start { step: Step },
    Output { id: usize, line: String },
    Done { id: usize, ok: bool, detail: String },
    Skipped { id: usize },
    Note { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResult {
    pub service: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub env: String,
    pub ok: bool,
    /// A step was declined.
    pub cancelled: bool,
    pub commit: Option<String>,
    pub services: Vec<String>,
    pub health: Vec<HealthResult>,
    pub rolled_back: bool,
    pub color: Option<String>,
    pub at: u64,
    pub log: Vec<String>,
}

/// `name` on PATH, honoring `PATHEXT` (`docker.cmd` shims on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, &std::env::var_os("PATH")?)
}

fn which_in(name: &str, search: &std::ffi::OsStr) -> Option<PathBuf> {
    let exts: Vec<String> = std::iter::once(String::new())
        .chain(
            std::env::var("PATHEXT")
                .unwrap_or_default()
                .split(';')
                .filter(|e| !e.is_empty())
                .map(str::to_ascii_lowercase),
        )
        .collect();
    std::env::split_paths(search)
        .flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}"))))
        .find(|p| p.is_file())
}

struct Runner<'a> {
    root: &'a Path,
    on: &'a (dyn Fn(&Event) + Send + Sync),
    log: Vec<String>,
    /// Program search path (`PATH`; tests put their shims first).
    search: std::ffi::OsString,
}

impl Runner<'_> {
    fn emit(&mut self, e: &Event) {
        let line = match e {
            Event::Start { step } => format!("$ {}", step.display),
            Event::Output { line, .. } => format!("  {line}"),
            Event::Done { ok, detail, .. } => {
                format!("  {} {detail}", if *ok { "ok" } else { "FAILED" })
            }
            Event::Skipped { id } => format!("  skipped step {id}"),
            Event::Note { text } => format!("# {text}"),
        };
        if self.log.len() < MAX_LOG {
            self.log.push(line);
        }
        (self.on)(e);
    }

    /// Run a command, streaming its output. `Ok(stdout)` on exit code 0.
    async fn exec(&mut self, step: &Step) -> Result<String, String> {
        let program = which_in(&step.program, &self.search)
            .ok_or_else(|| format!("`{}` is not installed or not on PATH", step.program))?;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&step.args)
            .envs(step.env.iter().map(|(k, v)| (k, v)))
            .current_dir(self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd.no_window();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", step.program))?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let err = child.stderr.take().ok_or("no stderr")?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();
        let pipe = |r: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
                    is_out: bool,
                    tx: tokio::sync::mpsc::UnboundedSender<(bool, String)>| async move {
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send((is_out, l));
            }
        };
        let t1 = tokio::spawn(pipe(Box::new(out), true, tx.clone()));
        let t2 = tokio::spawn(pipe(Box::new(err), false, tx));
        let mut stdout = String::new();
        let work = async {
            while let Some((is_out, l)) = rx.recv().await {
                let l = kor_tools::redact_secrets(&l);
                if is_out {
                    stdout.push_str(&l);
                    stdout.push('\n');
                }
                self.emit(&Event::Output {
                    id: step.id,
                    line: l,
                });
            }
            child.wait().await
        };
        let status = tokio::time::timeout(COMMAND_TIMEOUT, work)
            .await
            .map_err(|_| "timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        let _ = tokio::join!(t1, t2);
        if status.success() {
            Ok(stdout)
        } else {
            Err(format!("exit code {}", status.code().unwrap_or(-1)))
        }
    }

    async fn step(
        &mut self,
        step: &Step,
        approve: &mut (dyn FnMut(&Step) -> bool + Send),
        ask: bool,
    ) -> Option<Result<String, String>> {
        if step.approve && ask && !approve(step) {
            self.emit(&Event::Skipped { id: step.id });
            return None;
        }
        self.emit(&Event::Start { step: step.clone() });
        let r = self.exec(step).await;
        self.emit(&Event::Done {
            id: step.id,
            ok: r.is_ok(),
            detail: r.as_ref().err().cloned().unwrap_or_default(),
        });
        Some(r)
    }
}

/// Health of one service from `docker compose ps --format json` (array or one object per line).
/// `Some(true)` healthy / running without a healthcheck, `Some(false)` unhealthy / exited,
/// `None` not decided yet.
pub fn compose_health(json: &str) -> (Option<bool>, String) {
    let mut items: Vec<serde_json::Value> = Vec::new();
    match serde_json::from_str::<serde_json::Value>(json.trim()) {
        Ok(serde_json::Value::Array(a)) => items = a,
        Ok(v @ serde_json::Value::Object(_)) => items.push(v),
        _ => items.extend(json.lines().filter_map(|l| serde_json::from_str(l).ok())),
    }
    if items.is_empty() {
        return (Some(false), "no container".into());
    }
    let mut verdict = Some(true);
    let mut detail = Vec::new();
    for c in &items {
        let state = c
            .get("State")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let health = c
            .get("Health")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        detail.push(if health.is_empty() {
            state.clone()
        } else {
            format!("{state}/{health}")
        });
        let v = match (state.as_str(), health.as_str()) {
            ("exited" | "dead", _) | (_, "unhealthy") => Some(false),
            ("running", "healthy" | "") => Some(true),
            _ => None,
        };
        verdict = match (verdict, v) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (None, _) | (_, None) => None,
            _ => Some(true),
        };
    }
    (verdict, detail.join(", "))
}

async fn http_ok(url: &str) -> Result<String, String> {
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .get(url)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let st = r.status();
    if st.is_success() || st.is_redirection() {
        Ok(format!("HTTP {}", st.as_u16()))
    } else {
        Err(format!("HTTP {}", st.as_u16()))
    }
}

/// Run a plan. `approve` is asked for every changing step (the CLI prompts, the desktop passes
/// the steps the user accepted, a trigger policy may allow all). A failed command stops the
/// run; a failed health check starts the rollback (asked unless `rollback = "auto"`). A fully
/// successful run records `HEAD` as the environment's deployed commit.
pub async fn run(
    root: &Path,
    plan: &Plan,
    approve: &mut (dyn FnMut(&Step) -> bool + Send),
    on: &(dyn Fn(&Event) + Send + Sync),
) -> Report {
    let search = std::env::var_os("PATH").unwrap_or_default();
    run_in(root, plan, approve, on, search).await
}

#[allow(clippy::too_many_lines)]
async fn run_in(
    root: &Path,
    plan: &Plan,
    approve: &mut (dyn FnMut(&Step) -> bool + Send),
    on: &(dyn Fn(&Event) + Send + Sync),
    search: std::ffi::OsString,
) -> Report {
    let mut r = Runner {
        root,
        on,
        log: Vec::new(),
        search,
    };
    let mut report = Report {
        env: plan.env.clone(),
        commit: plan.head.clone(),
        services: plan.services.clone(),
        color: plan.color.clone(),
        at: now_secs(),
        ..Report::default()
    };
    if plan.steps.is_empty() {
        r.emit(&Event::Note {
            text: "nothing changed since the last deploy".into(),
        });
    }
    let mut previous: Vec<(String, String)> = Vec::new(); // (image id, image name)
    let mut started = false;
    let mut failed = false;
    for step in &plan.steps {
        match step.kind {
            StepKind::Snapshot => {
                let Some(Ok(out)) = r.step(step, approve, false).await else {
                    continue;
                };
                let Some(cid) = out.lines().map(str::trim).find(|l| !l.is_empty()) else {
                    continue;
                };
                let inspect = Step {
                    args: vec![
                        "inspect".into(),
                        "-f".into(),
                        "{{.Image}}|{{.Config.Image}}".into(),
                        cid.to_owned(),
                    ],
                    display: display(
                        &step.env,
                        "docker",
                        &[
                            "inspect".into(),
                            "-f".into(),
                            "{{.Image}}|{{.Config.Image}}".into(),
                            cid.to_owned(),
                        ],
                    ),
                    ..step.clone()
                };
                if let Some(Ok(out)) = r.step(&inspect, approve, false).await
                    && let Some((id, name)) = out.trim().split_once('|')
                    && !id.is_empty()
                    && valid_name(name)
                {
                    previous.push((id.to_owned(), name.to_owned()));
                }
            }
            StepKind::Health => {
                let svc = step.service.clone().unwrap_or_default();
                let res = if step.program == "kubectl" {
                    match r.step(step, approve, false).await {
                        Some(Ok(_)) => Ok("rolled out".to_owned()),
                        Some(Err(e)) => Err(e),
                        None => Err("skipped".into()),
                    }
                } else {
                    let deadline =
                        tokio::time::Instant::now() + Duration::from_secs(plan.health_timeout);
                    loop {
                        let out = r.step(step, approve, false).await;
                        let (verdict, detail) = match out {
                            Some(Ok(o)) => compose_health(&o),
                            Some(Err(e)) => (Some(false), e),
                            None => (Some(false), "skipped".into()),
                        };
                        match verdict {
                            Some(true) => break Ok(detail),
                            Some(false) => break Err(detail),
                            None if tokio::time::Instant::now() >= deadline => {
                                break Err(format!(
                                    "not healthy after {} s ({detail})",
                                    plan.health_timeout
                                ));
                            }
                            None => tokio::time::sleep(Duration::from_secs(2)).await,
                        }
                    }
                };
                failed |= res.is_err();
                report.health.push(HealthResult {
                    service: svc,
                    ok: res.is_ok(),
                    detail: res.unwrap_or_else(|e| e),
                });
            }
            StepKind::HttpHealth => {
                let url = step.args.first().cloned().unwrap_or_default();
                r.emit(&Event::Start { step: step.clone() });
                let deadline =
                    tokio::time::Instant::now() + Duration::from_secs(plan.health_timeout);
                let res = loop {
                    match http_ok(&url).await {
                        Ok(d) => break Ok(d),
                        Err(e) if tokio::time::Instant::now() >= deadline => break Err(e),
                        Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
                    }
                };
                r.emit(&Event::Done {
                    id: step.id,
                    ok: res.is_ok(),
                    detail: res.clone().unwrap_or_else(|e| e),
                });
                failed |= res.is_err();
                report.health.push(HealthResult {
                    service: step.service.clone().unwrap_or(url),
                    ok: res.is_ok(),
                    detail: res.unwrap_or_else(|e| e),
                });
            }
            _ => {
                if failed {
                    // Health failed: do not switch traffic or stop the old version.
                    r.emit(&Event::Skipped { id: step.id });
                    continue;
                }
                match r.step(step, approve, true).await {
                    None => {
                        report.cancelled = true;
                        break;
                    }
                    Some(Err(_)) => {
                        failed = true;
                        if !started {
                            break;
                        }
                    }
                    Some(Ok(_)) => started |= matches!(step.kind, StepKind::Up | StepKind::Apply),
                }
            }
        }
        if report.cancelled {
            break;
        }
    }
    if failed && started && plan.rollback != Rollback::Off && !plan.rollback_steps.is_empty() {
        r.emit(&Event::Note {
            text: "health check failed: rolling back".into(),
        });
        let ask = plan.rollback == Rollback::Ask;
        let mut ok = true;
        let mut rb: Vec<Step> = Vec::new();
        let next = plan.steps.len() + 1;
        for (id, name) in &previous {
            let args = vec!["tag".to_owned(), id.clone(), name.clone()];
            rb.push(Step {
                id: next + rb.len(),
                kind: StepKind::Rollback,
                display: display(&plan.rollback_steps[0].env, "docker", &args),
                program: "docker".into(),
                args,
                env: plan.rollback_steps[0].env.clone(),
                approve: true,
                destructive: false,
                service: None,
            });
        }
        for st in &plan.rollback_steps {
            rb.push(Step {
                id: next + rb.len(),
                ..st.clone()
            });
        }
        for st in &rb {
            if !matches!(r.step(st, approve, ask).await, Some(Ok(_))) {
                ok = false;
                break;
            }
        }
        report.rolled_back = ok;
    }
    report.ok = !failed && !report.cancelled;
    let mut state = load_state(root);
    let entry = state.envs.entry(plan.env.clone()).or_default();
    if report.ok {
        entry.commit.clone_from(&plan.head);
        entry.at = report.at;
        entry.services.clone_from(&plan.services);
        if plan.color.is_some() {
            entry.color.clone_from(&plan.color);
        }
    }
    report.log = std::mem::take(&mut r.log);
    entry.last = Some(report.clone());
    if let Err(e) = save_state(root, &state) {
        report
            .log
            .push(format!("# could not save .kor/deploy/state.json: {e}"));
    }
    report
}

// ---------------------------------------------------------------------------------------------
// Risk check
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    High,
    Medium,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Risk {
    pub level: Level,
    /// `migration`, `breaking`, `route`, `peak`, `dirty`, `first`.
    pub kind: String,
    pub title: String,
    pub file: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskReport {
    pub env: String,
    pub since: Option<String>,
    pub risks: Vec<Risk>,
}

/// `HH:MM-HH:MM` contains `now` (windows may wrap midnight).
pub fn in_window(window: &str, now: chrono::NaiveTime) -> Result<bool, String> {
    let parse = |t: &str| {
        chrono::NaiveTime::parse_from_str(t.trim(), "%H:%M")
            .map_err(|_| format!("peak_hours `{window}`: use HH:MM-HH:MM"))
    };
    let (a, b) = window
        .split_once('-')
        .ok_or_else(|| format!("peak_hours `{window}`: use HH:MM-HH:MM"))?;
    let (a, b) = (parse(a)?, parse(b)?);
    Ok(if a <= b {
        now >= a && now < b
    } else {
        now >= a || now < b
    })
}

pub fn is_migration(path: &str) -> bool {
    static FLYWAY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(^|/)V\d+(_\d+)*__[^/]+\.sql$").expect("flyway"));
    let l = path.to_ascii_lowercase();
    let name = l.rsplit('/').next().unwrap_or_default();
    (l.contains("migrat") || l.contains("alembic/versions/") || FLYWAY.is_match(path))
        && !name.contains("down")
        && !Path::new(path)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md"))
}

static IRREVERSIBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ix)
        drop\s+table\s+(?:if\s+exists\s+)?([\w."`\[\]]+)
        | truncate\s+(?:table\s+)?([\w."`\[\]]+)
        | alter\s+table\s+(?:only\s+)?([\w."`\[\]]+)[^;]*?\bdrop\b
        | drop\s+(?:column|schema|database|view|type)\b
        | delete\s+from\s+([\w."`\[\]]+)
        | \b(?:drop_table|remove_column|remove_columns|remove_reference|rename_column|rename_table|drop_join_table)\s*\(?\s*:(\w+)
        | \bop\.(?:drop_table|drop_column)\(\s*['"](\w+)
        | \bmigrations\.(?:DeleteModel|RemoveField|RenameField|RenameModel)\b
        | \bmigrationBuilder\.(?:DropTable|DropColumn|RenameColumn|RenameTable)\(\s*(?:name:\s*)?"?(\w*)
        | \b(?:dropTable|dropColumn|dropIfExists|renameColumn)\(\s*['"]?(\w*)
        | Schema::drop(?:IfExists)?\(\s*['"](\w+)
        "#,
    )
    .expect("irreversible migration regex")
});

/// Irreversible operations in a migration: `(matched text, table if named)`.
pub fn irreversible(text: &str) -> Vec<(String, Option<String>)> {
    IRREVERSIBLE
        .captures_iter(text)
        .map(|c| {
            let table = c
                .iter()
                .skip(1)
                .flatten()
                .map(|m| m.as_str())
                .find(|s| !s.is_empty())
                .map(kor_graph::db::table_key);
            let whole = c.get(0).map_or("", |m| m.as_str());
            (
                whole
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect(),
                table,
            )
        })
        .collect()
}

/// `(file, name)` pairs.
pub type Named = Vec<(String, String)>;

static PUBLIC: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static)\s+(\w+)",
        r"^\s*export\s+(?:default\s+)?(?:async\s+)?(?:function\*?|class|const|let|var|interface|type|enum)\s+(\w+)",
        r"^\s*public\s+(?:(?:static|abstract|virtual|override|async|sealed|final|readonly|partial)\s+)*(?:class|interface|record|struct|enum)\s+(\w+)",
        r"^\s*public\s+(?:(?:static|abstract|virtual|override|async|sealed|final)\s+)*[\w<>\[\],.?]+\s+(\w+)\s*\(",
        r"^\s*public\s+(?:static\s+)?function\s+(\w+)",
        r"^func\s+(?:\([^)]*\)\s*)?([A-Z]\w*)",
        r"^def\s+([A-Za-z]\w*)",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("public symbol regex"))
    .collect()
});

static ROUTE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"(?i)(?:\.|::)(get|post|put|patch|delete|route|api_route)\(\s*['"](/[^'"]*)['"]"#,
        r#"#\[(get|post|put|patch|delete)\("(/[^"]*)""#,
        r#"\[Http(Get|Post|Put|Patch|Delete)\("([^"]*)"\)"#,
        r#"Map(Get|Post|Put|Patch|Delete)\("([^"]+)""#,
        r#"\b(path|re_path)\(\s*r?['"]([^'"]+)['"]"#,
    ]
    .iter()
    .map(|p| Regex::new(p).expect("route regex"))
    .collect()
});

/// Removed public symbols and routes in a unified diff: `(file, name)` lists, minus those added
/// back anywhere in the diff (renames inside a file, moved code).
pub fn removed_api(diff: &str) -> (Named, Named) {
    let mut file = String::new();
    let mut gone_sym: Vec<(String, String)> = Vec::new();
    let mut gone_route: Vec<(String, String)> = Vec::new();
    let mut added_sym = BTreeSet::new();
    let mut added_route = BTreeSet::new();
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("+++ ") {
            f.trim_start_matches("b/").clone_into(&mut file);
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("diff ") {
            continue;
        }
        let (removed, body) = match line.as_bytes().first() {
            Some(b'-') => (true, &line[1..]),
            Some(b'+') => (false, &line[1..]),
            _ => continue,
        };
        for re in PUBLIC.iter() {
            if let Some(c) = re.captures(body).and_then(|c| c.get(1)) {
                if removed {
                    gone_sym.push((file.clone(), c.as_str().to_owned()));
                } else {
                    added_sym.insert(c.as_str().to_owned());
                }
                break;
            }
        }
        for re in ROUTE.iter() {
            if let Some(c) = re.captures(body) {
                let route = format!(
                    "{} {}",
                    c.get(1).map_or("", |m| m.as_str()).to_ascii_uppercase(),
                    c.get(2).map_or("", |m| m.as_str())
                );
                if removed {
                    gone_route.push((file.clone(), route));
                } else {
                    added_route.insert(route);
                }
            }
        }
    }
    gone_sym.retain(|(_, n)| !added_sym.contains(n));
    gone_sym.dedup();
    gone_route.retain(|(_, r)| !added_route.contains(r));
    gone_route.dedup();
    (gone_sym, gone_route)
}

/// `kor deploy risk-check <env>`: irreversible migrations (with the code reading/writing each
/// dropped table, from the graph's database layer), removed public symbols (confirmed by the
/// graph diff when it can be built) and routes, uncommitted changes, and the peak-hours window.
pub fn risk_check(root: &Path, env: &str, now: chrono::NaiveTime) -> Result<RiskReport, String> {
    let file = load(root)?;
    let cfg = env_config(&file, env)?;
    let state = load_state(root);
    let since = state.envs.get(env).and_then(|e| e.commit.clone());
    let changed = changed_files(root, since.as_deref());
    let mut risks = Vec::new();
    let window = cfg
        .peak_hours
        .or(file.peak_hours)
        .unwrap_or_else(|| DEFAULT_PEAK.to_owned());
    if in_window(&window, now)? {
        risks.push(Risk {
            level: Level::Medium,
            kind: "peak".into(),
            title: format!("inside peak hours ({window})"),
            file: None,
            detail: "deploying now affects the most users; prefer a quieter time".into(),
        });
    }
    if git(root, &["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty()) {
        risks.push(Risk {
            level: Level::Info,
            kind: "dirty".into(),
            title: "uncommitted changes".into(),
            file: None,
            detail: "they are deployed but not recorded in a commit".into(),
        });
    }
    let candidates: Vec<String> = if let Some(c) = &changed {
        c.clone()
    } else {
        {
            risks.push(Risk {
                level: Level::Info,
                kind: "first".into(),
                title: "no recorded deploy for this environment".into(),
                file: None,
                detail: "every migration in the project is checked".into(),
            });
            git(root, &["ls-files"])
                .map(|s| s.lines().map(str::to_owned).collect())
                .unwrap_or_default()
        }
    };
    let mut graph = kor_graph::Graph::open(root).ok();
    if let Some(g) = graph.as_mut()
        && g.update().is_err()
    {
        graph = None;
    }
    for f in candidates.iter().filter(|f| is_migration(f)) {
        let Ok(text) = std::fs::read_to_string(root.join(f)) else {
            continue;
        };
        for (what, table) in irreversible(&text) {
            let users = table
                .as_deref()
                .and_then(|t| graph.as_ref()?.table(t))
                .map_or(0, |t| t.readers.len() + t.writers.len());
            risks.push(Risk {
                level: Level::High,
                kind: "migration".into(),
                title: format!("irreversible migration: {what}"),
                file: Some(f.clone()),
                detail: match (&table, users) {
                    (Some(t), n) if n > 0 => format!(
                        "table {t} is still read or written by {n} code location(s) in the graph"
                    ),
                    (Some(t), _) => format!("table {t}: data is lost; back it up first"),
                    _ => "data is lost; back it up first".into(),
                },
            });
        }
    }
    if let Some(since) = since.as_deref().filter(|_| changed.is_some()) {
        let diff = git(root, &["diff", "-U0", since, "--"]).unwrap_or_default();
        let (symbols, routes) = removed_api(&diff);
        let removed_nodes: Option<BTreeSet<String>> = kor_graph::diff(root, since).ok().map(|d| {
            d.removed_nodes
                .into_iter()
                .map(|n| {
                    n.name
                        .rsplit(['.', ':'])
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        });
        for (f, name) in symbols {
            if removed_nodes
                .as_ref()
                .is_some_and(|set| !set.contains(&name))
            {
                continue;
            }
            risks.push(Risk {
                level: if removed_nodes.is_some() {
                    Level::High
                } else {
                    Level::Medium
                },
                kind: "breaking".into(),
                title: format!("public symbol removed: {name}"),
                file: Some(f),
                detail: "callers outside this repository break".into(),
            });
        }
        for (f, route) in routes {
            risks.push(Risk {
                level: Level::High,
                kind: "route".into(),
                title: format!("route removed: {route}"),
                file: Some(f),
                detail: "clients calling it get 404".into(),
            });
        }
    }
    risks.sort_by(|a, b| a.level.cmp(&b.level));
    Ok(RiskReport {
        env: env.to_owned(),
        since,
        risks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_services_and_change_mapping() {
        let yaml = "services:\n  web:\n    build: ./web\n    volumes:\n      - ./shared:/app/shared\n      - data:/var/data\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n  api:\n    build:\n      context: api\n      dockerfile: ../docker/api.Dockerfile\n  db:\n    image: postgres:16\n";
        let s = parse_compose(yaml, "").unwrap();
        assert_eq!(s.len(), 3);
        let web = s.iter().find(|x| x.name == "web").unwrap();
        assert_eq!(web.context.as_deref(), Some("web"));
        assert_eq!(web.mounts, vec!["shared"]);
        assert!(web.healthcheck);
        let api = s.iter().find(|x| x.name == "api").unwrap();
        assert_eq!(api.dockerfile.as_deref(), Some("docker/api.Dockerfile"));
        let cfg = vec!["docker-compose.yml".to_owned()];
        let ch = |f: &[&str]| {
            changed_services(
                &s,
                &cfg,
                Some(&f.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>()),
            )
        };
        assert_eq!(ch(&["web/index.js"]), vec!["web"]);
        assert_eq!(ch(&["shared/x.ts"]), vec!["web"]);
        assert_eq!(ch(&["docker/api.Dockerfile"]), vec!["api"]);
        assert!(ch(&["README.md", "webby/x"]).is_empty());
        assert_eq!(ch(&["docker-compose.yml"]).len(), 3);
        assert_eq!(ch(&[".env"]).len(), 3);
        assert_eq!(changed_services(&s, &cfg, None).len(), 3);
        assert_eq!(norm("a/b", "../c/./d"), "a/c/d");
    }

    #[test]
    fn workloads_and_health_parsing() {
        let m = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n---\nkind: Service\nmetadata:\n  name: web\n---\nkind: StatefulSet\nmetadata:\n  name: db\n";
        assert_eq!(
            workloads(m),
            vec![
                ("deployment".into(), "web".into()),
                ("statefulset".into(), "db".into())
            ]
        );
        assert_eq!(
            compose_health(r#"[{"State":"running","Health":"healthy"}]"#).0,
            Some(true)
        );
        assert_eq!(
            compose_health(r#"{"State":"running","Health":""}"#).0,
            Some(true)
        );
        assert_eq!(
            compose_health("{\"State\":\"running\",\"Health\":\"starting\"}\n").0,
            None
        );
        assert_eq!(
            compose_health(r#"{"State":"running","Health":"unhealthy"}"#).0,
            Some(false)
        );
        assert_eq!(compose_health("").0, Some(false));
    }

    #[test]
    fn peak_window_and_migrations() {
        let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert!(in_window("09:00-18:00", t(12, 0)).unwrap());
        assert!(!in_window("09:00-18:00", t(18, 0)).unwrap());
        assert!(in_window("22:00-02:00", t(1, 0)).unwrap());
        assert!(!in_window("22:00-02:00", t(12, 0)).unwrap());
        assert!(in_window("nope", t(1, 0)).is_err());
        assert!(is_migration("db/migrate/2024_drop_users.rb"));
        assert!(is_migration("src/main/resources/db/V2__drop.sql"));
        assert!(!is_migration("migrations/0002.down.sql"));
        assert!(!is_migration("src/app.rs"));
        let found = irreversible(
            "ALTER TABLE users ADD x int;\nDROP TABLE IF EXISTS public.sessions;\nclass M < ActiveRecord::Migration\n  remove_column :orders, :total\nend\nop.drop_table('carts')\nSchema::dropIfExists('logs');",
        );
        let tables: Vec<Option<String>> = found.iter().map(|f| f.1.clone()).collect();
        assert_eq!(
            tables,
            vec![
                Some("sessions".into()),
                Some("orders".into()),
                Some("carts".into()),
                Some("logs".into())
            ]
        );
        assert!(irreversible("CREATE TABLE a (id int);").is_empty());
    }

    #[test]
    fn removed_api_from_diff() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +0,0 @@\n-pub fn login(u: &str) {}\n-pub fn moved() {}\n+pub fn moved() -> u8 { 1 }\n--- a/app.py\n+++ b/app.py\n@@ -3 +3 @@\n-@app.get(\"/users\")\n+@app.get(\"/users/v2\")\n-@app.post(\"/kept\")\n+@app.post(\"/kept\")\n";
        let (syms, routes) = removed_api(diff);
        assert_eq!(syms, vec![("src/lib.rs".into(), "login".into())]);
        assert_eq!(routes, vec![("app.py".into(), "GET /users".into())]);
    }

    fn fake_bin(dir: &Path, name: &str) {
        // Shims record their arguments and answer from `reply_<a>_<b>.txt` / `fail_<a>_<b>`
        // (first two arguments after `compose` and the -f/-p/--context/-n options).
        if kor_platform::os() == kor_platform::Os::Windows {
            let body = format!(
                "@echo off\r\nset \"D=%~dp0\"\r\necho {name} %*>>\"%D%calls.log\"\r\n:skip\r\nif \"%1\"==\"compose\" (shift & goto skip)\r\nif \"%1\"==\"-f\" (shift & shift & goto skip)\r\nif \"%1\"==\"-p\" (shift & shift & goto skip)\r\nif \"%1\"==\"--context\" (shift & shift & goto skip)\r\nif \"%1\"==\"-n\" (shift & shift & goto skip)\r\nif exist \"%D%reply_%1_%2.txt\" type \"%D%reply_%1_%2.txt\"\r\nif exist \"%D%fail_%1_%2\" exit /b 1\r\nexit /b 0\r\n"
            );
            std::fs::write(dir.join(format!("{name}.cmd")), body).unwrap();
        } else {
            let body = format!(
                "#!/bin/sh\nd=$(dirname \"$0\")\necho \"{name} $*\" >> \"$d/calls.log\"\nwhile :; do case \"$1\" in compose) shift;; -f|-p|--context|-n) shift; shift;; *) break;; esac; done\n[ -f \"$d/reply_$1_$2.txt\" ] && cat \"$d/reply_$1_$2.txt\"\n[ -f \"$d/fail_$1_$2\" ] && exit 1\nexit 0\n"
            );
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            kor_platform::make_executable(&p).unwrap();
        }
    }

    fn sh(root: &Path, args: &[&str]) {
        let ok = std::process::Command::new(kor_platform::exe("git"))
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// PATH is process-wide: one test drives every scenario with the shims.
    #[tokio::test]
    async fn compose_and_kubernetes_with_fake_clis() {
        let bin = tempfile::tempdir().unwrap();
        fake_bin(bin.path(), "docker");
        fake_bin(bin.path(), "kubectl");
        // The shims come first on the search path (PATH itself is process-wide: not changed).
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin.path().to_path_buf()];
        paths.extend(std::env::split_paths(&old_path));
        let search = std::env::join_paths(paths).unwrap();
        let calls = || std::fs::read_to_string(bin.path().join("calls.log")).unwrap_or_default();
        let reply = |k: &str, v: &str| {
            std::fs::write(bin.path().join(format!("reply_{k}.txt")), v).unwrap()
        };

        // --- compose: first deploy builds everything, health ok, commit recorded.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::fs::write(root.join("web/a.js"), "1").unwrap();
        std::fs::write(root.join("api/b.py"), "1").unwrap();
        std::fs::write(
            root.join("docker-compose.yml"),
            "services:\n  web:\n    build: ./web\n  api:\n    build: ./api\n",
        )
        .unwrap();
        sh(root, &["init", "-q"]);
        sh(root, &["add", "-A"]);
        sh(root, &["commit", "-qm", "one"]);
        reply("ps_-q", "cid123\n");
        reply("inspect_-f", "sha256:old|app-web\n");
        reply(
            "ps_--format",
            "{\"State\":\"running\",\"Health\":\"healthy\"}\n",
        );
        let p = plan(root, "local").unwrap();
        assert_eq!(p.target, "compose");
        assert_eq!(p.services, vec!["web", "api"]);
        let mut asked = Vec::new();
        let rep = run_in(
            root,
            &p,
            &mut |s: &Step| {
                asked.push(s.kind);
                true
            },
            &|_| {},
            search.clone(),
        )
        .await;
        assert!(rep.ok, "{:?}", rep.log);
        assert_eq!(asked, vec![StepKind::Build, StepKind::Up]);
        let c = calls();
        assert!(c.contains("docker compose -f docker-compose.yml -p"), "{c}");
        assert!(c.contains("build web api"), "{c}");
        assert!(c.contains("up -d --no-deps web api"), "{c}");
        assert_eq!(load_state(root).envs["local"].commit, head(root));

        // --- second deploy: only `web` changed; its health fails → rollback to the old image.
        std::fs::write(root.join("web/a.js"), "2").unwrap();
        sh(root, &["commit", "-qam", "two"]);
        reply(
            "ps_--format",
            "{\"State\":\"running\",\"Health\":\"unhealthy\"}\n",
        );
        std::fs::write(bin.path().join("calls.log"), "").unwrap();
        let p = plan(root, "local").unwrap();
        assert_eq!(p.services, vec!["web"]);
        let rep = run_in(root, &p, &mut |_| true, &|_| {}, search.clone()).await;
        assert!(!rep.ok && rep.rolled_back, "{:?}", rep.log);
        let c = calls();
        assert!(c.contains("docker tag sha256:old app-web"), "{c}");
        assert!(c.contains("up -d --no-deps --no-build web"), "{c}");
        assert!(!c.contains("api"), "{c}");
        // The failed deploy is not recorded: the next plan still sees `web` as changed.
        assert_eq!(plan(root, "local").unwrap().services, vec!["web"]);

        // Declining a step cancels without running it.
        std::fs::write(bin.path().join("calls.log"), "").unwrap();
        let rep = run_in(
            root,
            &p,
            &mut |s: &Step| s.kind != StepKind::Up,
            &|_| {},
            search.clone(),
        )
        .await;
        assert!(rep.cancelled && !rep.ok);
        assert!(!calls().contains(" up "));

        // --- kubernetes: changed manifests are applied, rollout failure → rollout undo.
        let k = tempfile::tempdir().unwrap();
        let kr = k.path();
        std::fs::create_dir_all(kr.join(".kor")).unwrap();
        std::fs::create_dir_all(kr.join("k8s")).unwrap();
        std::fs::write(
            kr.join("k8s/web.yaml"),
            "kind: Deployment\nmetadata:\n  name: web\n",
        )
        .unwrap();
        std::fs::write(kr.join(".kor/deploy.toml"), "[env.prod]\ncontext = \"prod-ctx\"\nnamespace = \"shop\"\nmanifests = [\"k8s\"]\nrollback = \"auto\"\nhealth_timeout = 5\n").unwrap();
        assert!(plan(kr, "staging").unwrap_err().contains("prod"));
        std::fs::write(bin.path().join("fail_rollout_status"), "").unwrap();
        std::fs::write(bin.path().join("calls.log"), "").unwrap();
        let p = plan(kr, "prod").unwrap();
        assert_eq!(p.target, "kubernetes");
        let mut asked = 0;
        let rep = run_in(
            kr,
            &p,
            &mut |_| {
                asked += 1;
                true
            },
            &|_| {},
            search.clone(),
        )
        .await;
        assert!(!rep.ok && rep.rolled_back, "{:?}", rep.log);
        assert_eq!(asked, 1, "auto rollback does not ask");
        let c = calls();
        assert!(
            c.contains("kubectl --context prod-ctx -n shop apply -f k8s/web.yaml"),
            "{c}"
        );
        assert!(c.contains("rollout undo deployment/web"), "{c}");
        assert!(p.rollback_steps[0].destructive);

        // --- blue-green: deploy blue, then green stops blue (destructive).
        let b = tempfile::tempdir().unwrap();
        let br = b.path();
        std::fs::create_dir_all(br.join(".kor")).unwrap();
        std::fs::write(
            br.join("compose.yaml"),
            "services:\n  web:\n    image: nginx\n",
        )
        .unwrap();
        std::fs::write(
            br.join(".kor/deploy.toml"),
            "[env.prod]\nproject = \"shop\"\nblue_green = true\nswitch = \"echo switched\"\n",
        )
        .unwrap();
        reply("ps_--format", "{\"State\":\"running\",\"Health\":\"\"}\n");
        let p = plan(br, "prod").unwrap();
        assert_eq!(p.color.as_deref(), Some("blue"));
        assert!(
            run_in(br, &p, &mut |_| true, &|_| {}, search.clone())
                .await
                .ok
        );
        let p = plan(br, "prod").unwrap();
        assert_eq!(p.color.as_deref(), Some("green"));
        let stop = p.steps.iter().find(|s| s.kind == StepKind::Stop).unwrap();
        assert!(
            stop.destructive && stop.display.contains("shop-blue") && stop.display.contains("down")
        );
    }
}
