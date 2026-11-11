//! `Görevler/`: a dropped `.md` file is a task (§27.5).
//!
//! Front matter (all optional): `model: provider/model`, `project: <folder>`, `mode: plan|default`.
//! Safe by default: tasks started from a file run in `plan` mode (read-only) and nobody is there
//! to approve, so `default` also cannot write — any approval request is denied. Other modes are
//! refused. The answer is written to `<ad>.sonuç.md` next to it and the task file moves to
//! `Görevler/Tamamlandı/`. A file is claimed by renaming it first, so two watchers (daemon and
//! desktop) never run it twice.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use kor_core::PermissionMode;
use serde::Serialize;

use super::{Layout, TASKS, TASKS_DONE, move_unique, views::split_front};

const CLAIM: &str = ".kor-running";
/// Largest task file read.
const MAX_TASK: u64 = 64 * 1024;

/// A parsed task file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTask {
    /// The claimed file.
    pub path: PathBuf,
    pub name: String,
    pub prompt: String,
    pub model: Option<String>,
    /// Folder the agent works in (`project:`), else `Görevler/`.
    pub root: PathBuf,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TaskOutcome {
    Done { result: PathBuf, file: PathBuf },
    Failed { error: String, file: PathBuf },
}

/// Runs one task (the real one assembles an agent; tests script it).
#[async_trait]
pub trait TaskRunner: Send + Sync {
    async fn run(&self, task: &FileTask) -> Result<String, String>;
}

/// `.md` files waiting in `Görevler/` (results and claimed files excluded).
pub fn tasks_pending(l: &Layout) -> Vec<PathBuf> {
    if !l.enabled() {
        return Vec::new();
    }
    let result_suffix = format!(".{}.md", result_word());
    let mut v: Vec<PathBuf> = std::fs::read_dir(l.dir(TASKS))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            p.is_file()
                && n.to_lowercase().ends_with(".md")
                && !n.starts_with('.')
                && !n.ends_with(&result_suffix)
        })
        .collect();
    v.sort();
    v
}

fn result_word() -> &'static str {
    "sonuç"
}

/// Parse a task file's text. `default_root` is used without `project:`.
///
/// # Errors
/// Empty task, unknown or unsafe mode, missing project folder.
pub fn parse(path: &Path, name: &str, text: &str, default_root: &Path) -> Result<FileTask, String> {
    let (front, body) = split_front(text);
    let prompt = body.trim().to_owned();
    if prompt.is_empty() {
        return Err("empty task".into());
    }
    let get = |k: &str| {
        front
            .as_ref()
            .and_then(|f| f[k].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let mode = match get("mode").as_deref() {
        None | Some("plan") => PermissionMode::Plan,
        Some("default" | "ask") => PermissionMode::Default,
        Some(other) => {
            return Err(format!(
                "mode `{other}` is not allowed for file tasks (plan, default)"
            ));
        }
    };
    let root = match get("project") {
        Some(p) => {
            let p = PathBuf::from(p);
            if !p.is_absolute() || !p.is_dir() {
                return Err(format!("project folder not found: {}", p.display()));
            }
            p
        }
        None => default_root.to_path_buf(),
    };
    Ok(FileTask {
        path: path.to_path_buf(),
        name: name.to_owned(),
        prompt,
        model: get("model"),
        root,
        mode,
    })
}

/// Claim, run and file every waiting task, one after the other.
pub async fn run_pending(l: &Layout, runner: &dyn TaskRunner) -> Vec<TaskOutcome> {
    let mut out = Vec::new();
    for p in tasks_pending(l) {
        let Some(name) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let claimed = p.with_file_name(format!(".{name}.md{CLAIM}"));
        if std::fs::rename(&p, &claimed).is_err() {
            continue; // another watcher took it
        }
        let text = std::fs::metadata(&claimed)
            .ok()
            .filter(|m| m.len() <= MAX_TASK)
            .and_then(|_| std::fs::read_to_string(&claimed).ok());
        let result = match text {
            None => Err("task file is too large or not UTF-8".to_owned()),
            Some(t) => match parse(&claimed, &name, &t, &l.dir(TASKS)) {
                Ok(task) => runner.run(&task).await,
                Err(e) => Err(e),
            },
        };
        let done = l.dir(TASKS).join(TASKS_DONE);
        let back = p.clone();
        // Restore the original name before filing it.
        let file = std::fs::rename(&claimed, &back)
            .and_then(|()| move_unique(&back, &done))
            .unwrap_or(claimed);
        out.push(match result {
            Ok(answer) => {
                let res = crate::batch::unique_path(
                    &l.dir(TASKS).join(format!("{name}.{}.md", result_word())),
                    &[],
                );
                match std::fs::write(&res, answer) {
                    Ok(()) => TaskOutcome::Done { result: res, file },
                    Err(e) => TaskOutcome::Failed {
                        error: e.to_string(),
                        file,
                    },
                }
            }
            Err(error) => {
                let res = crate::batch::unique_path(
                    &l.dir(TASKS).join(format!("{name}.{}.md", result_word())),
                    &[],
                );
                let _ = std::fs::write(&res, format!("> Hata / Error: {error}\n"));
                TaskOutcome::Failed { error, file }
            }
        });
    }
    out
}
