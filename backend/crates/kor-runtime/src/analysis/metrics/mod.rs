//! Code metrics (spec §9 `kor explain|quality|debt|style|stats|impact`, §15.14 `kor dead-code`,
//! ADR 0047). Everything here is deterministic: tree-sitter statistics ([`code`]), git history
//! ([`git`]), coverage reports the user already produced ([`coverage`]) and the project graph
//! (`kor_graph`). A model is only used by `kor explain` to put the facts into words.
//!
//! §9.1: no number without its source. Every result struct carries the source of each figure
//! (`source` fields) and every per-line result is a [`Finding`](super::Finding) with a rule id.
//! Estimates (hours) are marked with `≈` wherever they are printed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod code;
pub mod coverage;
pub mod deadcode;
pub mod debt;
pub mod explain;
pub mod git;
pub mod impact;
pub mod quality;
pub mod stats;
pub mod style;

/// A–F grade of a 0–100 score: A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60, F below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Grade {
    A,
    B,
    C,
    D,
    F,
}

impl Grade {
    pub fn from_score(score: f64) -> Self {
        match score {
            s if s >= 90.0 => Self::A,
            s if s >= 80.0 => Self::B,
            s if s >= 70.0 => Self::C,
            s if s >= 60.0 => Self::D,
            _ => Self::F,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::F => "F",
        }
    }
}

/// What most metrics need, read once: code statistics, duplication, the newest coverage report
/// and the last year of history.
#[derive(Debug, Clone)]
pub struct Project {
    pub now: i64,
    pub is_repo: bool,
    pub head: Option<String>,
    pub files: Vec<code::FileStats>,
    pub dup: code::Duplication,
    pub coverage: Option<coverage::Coverage>,
    /// Non-merge commits of the last 365 days, newest first.
    pub commits: Vec<git::Commit>,
    /// Commits per file in the last [`debt::CHURN_DAYS`] days.
    pub churn: HashMap<String, u32>,
}

impl Project {
    pub fn gather(root: &Path) -> Self {
        let now = now_secs();
        let is_repo = git::is_repo(root);
        let commits = if is_repo {
            git::log(
                root,
                &git::LogOpts {
                    since: Some(now - 365 * 86_400),
                    ..git::LogOpts::default()
                },
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let files = code::scan(root);
        let dup = code::duplication(&files);
        Self {
            now,
            is_repo,
            head: is_repo.then(|| git::head(root)).flatten(),
            churn: git::churn(&commits, now - debt::CHURN_DAYS * 86_400),
            dup,
            files,
            coverage: coverage::load(root),
            commits,
        }
    }
}

/// The project graph, opened and brought up to date (incremental: only changed files reparse).
///
/// # Errors
/// When the graph store cannot be opened or written.
pub fn graph(root: &Path) -> Result<kor_graph::Graph, String> {
    let mut g = kor_graph::Graph::open(root).map_err(|e| e.to_string())?;
    g.update().map_err(|e| e.to_string())?;
    Ok(g)
}

/// `<root>/.kor/<name>`.
pub fn kor_dir(root: &Path, name: &str) -> PathBuf {
    root.join(".kor").join(name)
}

/// Project-relative, `/`-separated.
pub fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Round to one decimal for display and stored snapshots.
pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Write `value` as pretty JSON through a temp file and a rename.
///
/// # Errors
/// I/O errors.
pub fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Git repositories built in temp dirs with fixed authors and dates.
    use std::path::Path;
    use std::process::Command;

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", dir.join(".no-global"))
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn init(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.name", "Ayşe Dev"]);
        git(dir, &["config", "user.email", "ayse@example.com"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    pub fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// Commit everything as `name <email>` at `days_ago` days before now.
    pub fn commit(dir: &Path, msg: &str, name: &str, email: &str, days_ago: i64) {
        let when = chrono::Utc::now().timestamp() - days_ago * 86_400;
        let date = format!("@{when} +0000");
        git(dir, &["add", "-A"]);
        let out = Command::new("git")
            .args(["commit", "-q", "--no-verify", "-m", msg])
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", dir.join(".no-global"))
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .expect("git commit runs");
        assert!(
            out.status.success(),
            "commit: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Grade;

    #[test]
    fn grades_follow_the_bands() {
        assert_eq!(Grade::from_score(95.0), Grade::A);
        assert_eq!(Grade::from_score(80.0), Grade::B);
        assert_eq!(Grade::from_score(70.1), Grade::C);
        assert_eq!(Grade::from_score(60.0), Grade::D);
        assert_eq!(Grade::from_score(12.0), Grade::F);
    }
}
