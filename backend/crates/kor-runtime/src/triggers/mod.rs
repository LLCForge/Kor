//! Event-driven agent (§8.4, ADR 0039): `[[triggers]]` in the user or project config, run by
//! `kor daemon`. Every fire is a queued job (stored in `<kor home>/daemon/jobs/<id>.json`,
//! transcript saved, cost tracked, audited) that the normal agent runtime runs in a fresh git
//! worktree, with the trigger's own model, budget and approval policy.
//!
//! Interval routines (`kor routines`) share the scheduling engine (`routines::mark_due`); while
//! the daemon runs it also runs them, and the desktop app leaves them to it.

use std::path::{Path, PathBuf};

use kor_config::{Approval, Trigger, TriggerEvent};
use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod cron;
pub mod daemon;
pub mod hook;
pub mod logwatch;
pub mod notify;
pub mod runner;
pub mod smtp;
pub mod webhook;

pub const MAX_TASK: usize = 10_000;
const MAX_JOBS_KEPT: usize = 500;

pub(crate) fn now_ms() -> u64 {
    crate::cost::now_ms()
}

/// Canonical path without the Windows `\?\` prefix (git and the tools take plain paths).
pub(crate) fn canon(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p)
        .ok()
        .map(|c| kor_tools::without_verbatim(&c))
}

/// `<kor home>/daemon`.
pub fn dir() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("daemon"))
}

pub fn event_name(e: TriggerEvent) -> &'static str {
    match e {
        TriggerEvent::GitPush => "git_push",
        TriggerEvent::Cron => "cron",
        TriggerEvent::Webhook => "webhook",
        TriggerEvent::LogMatch => "log_match",
    }
}

/// A trigger with where it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Defined {
    pub name: String,
    /// Project the task runs in.
    pub root: PathBuf,
    /// Config file that defines it.
    pub origin: PathBuf,
    pub trigger: Trigger,
    /// Why it cannot run (shown by `kor triggers list`); `None` when valid.
    pub error: Option<String>,
}

/// Problems with a trigger, localized; empty when it can run.
pub fn validate(l: &Localizer, t: &Trigger) -> Vec<String> {
    let mut e = Vec::new();
    let bad = |key: &str, field: &str| l.t_args(key, &args!("field" => field.to_owned()));
    if t.task.trim().is_empty() || t.task.chars().count() > MAX_TASK {
        e.push(bad("trigger-bad-field", "task"));
    }
    if !t.budget_usd.is_finite() || t.budget_usd < 0.0 {
        e.push(bad("trigger-bad-field", "budget_usd"));
    }
    if t.name.chars().count() > 80 || t.name.contains(['/', '\\', '\n']) {
        e.push(bad("trigger-bad-field", "name"));
    }
    match t.event {
        TriggerEvent::GitPush => {}
        TriggerEvent::Cron => {
            if let Err(msg) = cron::Schedule::parse(&t.schedule) {
                e.push(l.t_args("trigger-bad-schedule", &args!("error" => msg)));
            }
        }
        TriggerEvent::Webhook => {
            if !t.path.starts_with('/')
                || t.path.len() > 200
                || t.path == "/push"
                || t.path.starts_with("/control")
                || t.path.starts_with("/approve")
                || !t
                    .path
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
            {
                e.push(bad("trigger-bad-field", "path"));
            }
            if t.secret.trim().is_empty() {
                e.push(l.t("trigger-webhook-needs-secret"));
            }
        }
        TriggerEvent::LogMatch => {
            if logwatch::Source::parse(&t.source, Path::new(".")).is_err() {
                e.push(bad("trigger-bad-field", "source"));
            }
            if pattern(&t.pattern).is_err() {
                e.push(bad("trigger-bad-field", "pattern"));
            }
        }
    }
    if !t.secret.is_empty() && !kor_secrets::vault::valid_name(&t.secret) {
        e.push(bad("trigger-bad-field", "secret"));
    }
    e
}

/// A `log_match` pattern, size-limited (no catastrophic regexes: the `regex` crate is linear).
pub fn pattern(p: &str) -> Result<regex::Regex, String> {
    if p.is_empty() || p.len() > 1000 {
        return Err("empty or too long".into());
    }
    regex::RegexBuilder::new(p)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| e.to_string())
}

/// `[[triggers]]` of one config file (only that file: arrays of different layers do not merge).
fn file_triggers(path: &Path) -> Result<Vec<Trigger>, String> {
    #[derive(Deserialize)]
    struct Only {
        #[serde(default)]
        triggers: Vec<Trigger>,
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let only = table.get("triggers").cloned().map_or(Ok(Vec::new()), |v| {
        Only::deserialize(toml::Value::Table(toml::Table::from_iter([(
            "triggers".to_owned(),
            v,
        )])))
        .map(|o| o.triggers)
        .map_err(|e| e.to_string())
    })?;
    Ok(only)
}

/// A project's own config file.
pub fn project_config(root: &Path) -> PathBuf {
    root.join(".kor").join("config.toml")
}

/// Every trigger: the user config's (each needs `root`) and each registered project's.
pub fn collect(l: &Localizer, user_config: Option<&Path>, projects: &[PathBuf]) -> Vec<Defined> {
    let mut out: Vec<Defined> = Vec::new();
    let add = |out: &mut Vec<Defined>, origin: &Path, root: Option<PathBuf>, list: Vec<Trigger>| {
        for (i, t) in list.into_iter().enumerate() {
            let name = if t.name.trim().is_empty() {
                format!("{}-{}", event_name(t.event), i + 1)
            } else {
                t.name.trim().to_owned()
            };
            let root = root
                .clone()
                .or_else(|| (!t.root.is_empty()).then(|| PathBuf::from(&t.root)))
                .and_then(|r| canon(&r));
            let mut errors = validate(l, &t);
            if root.is_none() {
                errors.push(l.t("trigger-no-root"));
            }
            if out.iter().any(|d| d.name == name) {
                errors.push(l.t_args("trigger-duplicate", &args!("name" => name.clone())));
            }
            out.push(Defined {
                name,
                root: root.unwrap_or_default(),
                origin: origin.to_owned(),
                trigger: t,
                error: (!errors.is_empty()).then(|| errors.join("; ")),
            });
        }
    };
    if let Some(u) = user_config {
        match file_triggers(u) {
            Ok(list) => add(&mut out, u, None, list),
            Err(e) => out.push(broken(u, &e)),
        }
    }
    for p in projects {
        let file = project_config(p);
        match file_triggers(&file) {
            Ok(list) => add(&mut out, &file, Some(p.clone()), list),
            Err(e) => out.push(broken(&file, &e)),
        }
    }
    out
}

fn broken(origin: &Path, e: &str) -> Defined {
    Defined {
        name: origin.display().to_string(),
        root: PathBuf::new(),
        origin: origin.to_owned(),
        trigger: Trigger {
            event: TriggerEvent::Cron,
            task: String::new(),
            name: String::new(),
            enabled: false,
            branch: String::new(),
            schedule: String::new(),
            path: String::new(),
            secret: String::new(),
            source: String::new(),
            pattern: String::new(),
            debounce_secs: 0,
            model: String::new(),
            budget_usd: 0.0,
            root: String::new(),
            approval: Approval::Notify,
            pr: false,
        },
        error: Some(e.to_owned()),
    }
}

/// Runnable: valid and enabled.
pub fn active(all: &[Defined]) -> impl Iterator<Item = &Defined> {
    all.iter()
        .filter(|d| d.error.is_none() && d.trigger.enabled)
}

// ---------------------------------------------------------------------------------------------
// Projects the daemon watches
// ---------------------------------------------------------------------------------------------

fn projects_file(dir: &Path) -> PathBuf {
    dir.join("projects.json")
}

pub fn projects(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(projects_file(dir))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

/// Watch `root`'s `.kor/config.toml` triggers; `false` when already watched.
///
/// # Errors
/// If the list cannot be written.
pub fn register(dir: &Path, root: &Path) -> Result<bool, String> {
    let root = canon(root).ok_or_else(|| format!("{}: not found", root.display()))?;
    let mut list = projects(dir);
    if list.contains(&root) {
        return Ok(false);
    }
    list.push(root);
    write_json(&projects_file(dir), &list)?;
    Ok(true)
}

/// Stop watching `root`.
///
/// # Errors
/// If the list cannot be written.
pub fn unregister(dir: &Path, root: &Path) -> Result<bool, String> {
    let root = canon(root).unwrap_or_else(|| root.to_owned());
    let mut list = projects(dir);
    let before = list.len();
    list.retain(|p| *p != root);
    if list.len() == before {
        return Ok(false);
    }
    write_json(&projects_file(dir), &list)?;
    Ok(true)
}

pub(crate) fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("kor-tmp");
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

// ---------------------------------------------------------------------------------------------
// Editing `[[triggers]]` (desktop add/edit/remove)
// ---------------------------------------------------------------------------------------------

/// Add `t` to `file`, or replace the entry with the same name. Comments and other settings stay;
/// the result is validated by loading it before it replaces the file.
///
/// # Errors
/// An invalid trigger or file.
pub fn save_trigger(l: &Localizer, file: &Path, t: &Trigger) -> Result<(), String> {
    let errors = validate(l, t);
    if t.name.trim().is_empty() {
        return Err(l.t_args("trigger-bad-field", &args!("field" => "name")));
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let one: toml_edit::DocumentMut = toml::to_string(t)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut table = one.as_table().clone();
    // Defaults stay out of the file.
    for (k, empty) in [
        ("branch", ""),
        ("schedule", ""),
        ("path", ""),
        ("secret", ""),
        ("source", ""),
        ("pattern", ""),
        ("model", ""),
        ("root", ""),
    ] {
        if table.get(k).and_then(|v| v.as_str()) == Some(empty) {
            table.remove(k);
        }
    }
    for k in ["debounce_secs", "budget_usd"] {
        let zero = table.get(k).is_some_and(|v| {
            v.as_integer() == Some(0) || v.as_float().is_some_and(|f| f.abs() < f64::EPSILON)
        });
        if zero {
            table.remove(k);
        }
    }
    if t.enabled {
        table.remove("enabled");
    }
    if !t.pr {
        table.remove("pr");
    }
    if t.approval == Approval::Notify {
        table.remove("approval");
    }
    let arr = doc
        .entry("triggers")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ))
        .as_array_of_tables_mut()
        .ok_or("`triggers` is not an array of tables")?;
    let name = t.name.trim();
    if let Some(existing) = arr
        .iter_mut()
        .find(|x| x.get("name").and_then(|v| v.as_str()) == Some(name))
    {
        *existing = table;
    } else {
        arr.push(table);
    }
    write_checked(file, &doc.to_string())
}

/// Remove the trigger named `name` from `file`; `false` when it was not there.
///
/// # Errors
/// If the file cannot be read or written.
pub fn remove_trigger(file: &Path, name: &str) -> Result<bool, String> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Ok(false);
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let Some(arr) = doc
        .get_mut("triggers")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
    else {
        return Ok(false);
    };
    let before = arr.len();
    arr.retain(|x| x.get("name").and_then(|v| v.as_str()) != Some(name));
    if arr.len() == before {
        return Ok(false);
    }
    write_checked(file, &doc.to_string()).map(|()| true)
}

fn write_checked(file: &Path, text: &str) -> Result<(), String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    kor_config::Config::deserialize(toml::Value::Table(table)).map_err(|e| e.to_string())?;
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("kor-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

// ---------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Queued,
    Running,
    /// Paused on an approval.
    Waiting,
    Done,
    Failed,
}

/// One fire of a trigger.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: String,
    pub trigger: String,
    pub event: String,
    /// What fired it: `main 1a2b3c4 → 5d6e7f8`, `POST /hooks/deploy`, the log line.
    pub detail: String,
    pub root: PathBuf,
    pub status: Status,
    pub created: u64,
    pub started: Option<u64>,
    pub finished: Option<u64>,
    /// Push: branch and commit; the PR base.
    pub base: Option<String>,
    pub sha: Option<String>,
    pub worktree: Option<PathBuf>,
    pub transcript: Option<PathBuf>,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub cost_usd: f64,
    pub cost_estimated: bool,
    pub approvals: u32,
    pub summary: String,
    pub error: Option<String>,
}

fn jobs_dir(dir: &Path) -> PathBuf {
    dir.join("jobs")
}

/// Sortable id: time, then random.
pub fn new_job_id() -> Result<String, String> {
    Ok(format!("{:011x}-{}", now_ms(), notify::random_hex(3)?))
}

pub fn valid_job_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Write the job (atomic).
///
/// # Errors
/// If it cannot be written.
pub fn save_job(dir: &Path, job: &Job) -> Result<(), String> {
    if !valid_job_id(&job.id) {
        return Err("bad job id".into());
    }
    write_json(&jobs_dir(dir).join(format!("{}.json", job.id)), job)
}

pub fn load_job(dir: &Path, id: &str) -> Option<Job> {
    if !valid_job_id(id) {
        return None;
    }
    let t = std::fs::read_to_string(jobs_dir(dir).join(format!("{id}.json"))).ok()?;
    serde_json::from_str(&t).ok()
}

/// Newest first; `trigger` filters by name.
pub fn history(dir: &Path, trigger: Option<&str>, limit: usize) -> Vec<Job> {
    let Ok(rd) = std::fs::read_dir(jobs_dir(dir)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .filter(|id| valid_job_id(id))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.iter()
        .filter_map(|id| load_job(dir, id))
        .filter(|j| trigger.is_none_or(|t| j.trigger == t))
        .take(limit)
        .collect()
}

/// Keep the newest jobs only.
pub fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(jobs_dir(dir)) else {
        return;
    };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    if files.len() <= MAX_JOBS_KEPT {
        return;
    }
    files.sort();
    for f in &files[..files.len() - MAX_JOBS_KEPT] {
        let _ = std::fs::remove_file(f);
    }
}

/// Plain view of a value for `kor triggers list` / the desktop.
pub fn describe(d: &Defined) -> Value {
    let t = &d.trigger;
    let what = match t.event {
        TriggerEvent::GitPush => {
            if t.branch.is_empty() {
                "*".into()
            } else {
                t.branch.clone()
            }
        }
        TriggerEvent::Cron => t.schedule.clone(),
        TriggerEvent::Webhook => t.path.clone(),
        TriggerEvent::LogMatch => format!("{} ~ {}", t.source, t.pattern),
    };
    serde_json::json!({
        "name": d.name,
        "event": event_name(t.event),
        "on": what,
        "task": t.task,
        "root": d.root,
        "origin": d.origin,
        "enabled": t.enabled,
        "model": t.model,
        "budgetUsd": t.budget_usd,
        "approval": t.approval,
        "pr": t.pr,
        "secret": t.secret,
        "branch": t.branch,
        "schedule": t.schedule,
        "path": t.path,
        "source": t.source,
        "pattern": t.pattern,
        "debounceSecs": t.debounce_secs,
        "error": d.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_i18n::Lang;

    fn l() -> Localizer {
        Localizer::new(Lang::En)
    }

    #[test]
    fn collect_names_validates_and_edits_keep_comments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        let file = project_config(&root);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "# mine\n[[triggers]]\nevent = \"git_push\"\nbranch = \"main\"\ntask = \"test\"\n\n[[triggers]]\nevent = \"webhook\"\npath = \"/hooks/x\"\ntask = \"t\"\n",
        )
        .unwrap();
        let all = collect(&l(), None, std::slice::from_ref(&root));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "git_push-1");
        assert!(all[0].error.is_none());
        assert!(
            all[1].error.as_deref().unwrap().contains("secret"),
            "{:?}",
            all[1].error
        );
        assert_eq!(active(&all).count(), 1);

        let mut t = all[0].trigger.clone();
        t.name = "nightly".into();
        t.event = TriggerEvent::Cron;
        t.schedule = "0 3 * * *".into();
        t.budget_usd = 0.5;
        save_trigger(&l(), &file, &t).unwrap();
        t.task = "changed".into();
        save_trigger(&l(), &file, &t).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# mine"), "{text}");
        assert_eq!(text.matches("nightly").count(), 1, "{text}");
        let all = collect(&l(), None, std::slice::from_ref(&root));
        assert_eq!(all[2].trigger.task, "changed");
        assert!(remove_trigger(&file, "nightly").unwrap());
        assert_eq!(collect(&l(), None, &[root]).len(), 2);

        t.schedule = "nope".into();
        assert!(save_trigger(&l(), &file, &t).is_err());
    }

    #[test]
    fn jobs_and_projects() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Job {
            id: new_job_id().unwrap(),
            trigger: "a".into(),
            ..Job::default()
        };
        save_job(dir.path(), &j).unwrap();
        j.id = format!("{:011x}-ffffff", now_ms() + 5);
        j.trigger = "b".into();
        save_job(dir.path(), &j).unwrap();
        let h = history(dir.path(), None, 10);
        assert_eq!(h[0].trigger, "b", "newest first");
        assert_eq!(history(dir.path(), Some("a"), 10).len(), 1);
        assert!(load_job(dir.path(), "../x").is_none());

        assert!(register(dir.path(), dir.path()).unwrap());
        assert!(!register(dir.path(), dir.path()).unwrap());
        assert_eq!(projects(dir.path()).len(), 1);
        assert!(unregister(dir.path(), dir.path()).unwrap());
    }
}
