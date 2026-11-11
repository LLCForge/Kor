//! Git plumbing for `kor task`: worktrees, branches, commits and all-or-nothing merges.

use std::path::{Path, PathBuf};

use kor_agent::gitflow;

/// Characters of a diff or conflict kept in the checkpoint and sent to a model.
pub const MAX_DIFF: usize = 48_000;
/// Bytes of one side of a conflicting file kept for the per-file conflict view.
const MAX_SIDE: usize = 200_000;

/// Kor's own state never goes into a sub-agent's commit.
const NOT_KOR: [&str; 4] = [
    ".",
    ":(exclude).kor/graph",
    ":(exclude).kor/worktrees",
    ":(exclude).kor/tasks",
];

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    gitflow::git(dir, args).map_err(|e| e.to_string())
}

pub fn repo_root(dir: &Path) -> Result<PathBuf, String> {
    gitflow::repo_root(dir).map_err(|e| e.to_string())
}

pub fn head(dir: &Path) -> Result<String, String> {
    git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_owned())
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn branch_exists(root: &Path, branch: &str) -> bool {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

/// Keep `.kor/worktrees/` and `.kor/tasks/` out of `git status` (local `info/exclude`, never
/// a tracked `.gitignore`).
pub fn exclude_kor_dirs(root: &Path) -> Result<(), String> {
    let common = git(root, &["rev-parse", "--git-common-dir"])?;
    let common = PathBuf::from(common.trim());
    let common = if common.is_absolute() {
        common
    } else {
        root.join(common)
    };
    let file = common.join("info").join("exclude");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let missing: Vec<&str> = ["/.kor/worktrees/", "/.kor/tasks/"]
        .into_iter()
        .filter(|l| !text.lines().any(|x| x.trim() == *l))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for l in missing {
        out.push_str(l);
        out.push('\n');
    }
    std::fs::create_dir_all(common.join("info")).map_err(|e| e.to_string())?;
    std::fs::write(&file, out).map_err(|e| e.to_string())
}

/// `git worktree add/remove/prune` and branch deletion touch shared files in `.git`
/// (`worktrees/`, `config`, `packed-refs`); parallel sub-agents take turns.
static WORKTREES: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn worktree_lock() -> std::sync::MutexGuard<'static, ()> {
    WORKTREES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Remove a worktree (and its branch when `branch` is given); missing ones are fine.
pub fn remove_worktree(root: &Path, path: &Path, branch: Option<&str>) {
    let _guard = worktree_lock();
    remove_locked(root, path, branch);
}

fn remove_locked(root: &Path, path: &Path, branch: Option<&str>) {
    if path.exists() {
        let _ = git(
            root,
            &["worktree", "remove", "--force", "--force", &path_arg(path)],
        );
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = git(root, &["worktree", "prune"]);
    if let Some(b) = branch {
        let _ = git(root, &["branch", "-D", b]);
    }
}

/// A fresh worktree on a new `branch` starting at `base` (an old one is replaced).
pub fn fresh_worktree(root: &Path, path: &Path, branch: &str, base: &str) -> Result<(), String> {
    let _guard = worktree_lock();
    remove_locked(root, path, Some(branch));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    git(
        root,
        &["worktree", "add", "-b", branch, &path_arg(path), base],
    )
    .map(|_| ())
}

/// The integration worktree: on `branch`, created from `base` the first time.
pub fn integration_worktree(
    root: &Path,
    path: &Path,
    branch: &str,
    base: &str,
) -> Result<(), String> {
    if path.join(".git").exists() {
        return Ok(());
    }
    let _guard = worktree_lock();
    let _ = git(root, &["worktree", "prune"]);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if branch_exists(root, branch) {
        git(root, &["worktree", "add", &path_arg(path), branch]).map(|_| ())
    } else {
        git(
            root,
            &["worktree", "add", "-b", branch, &path_arg(path), base],
        )
        .map(|_| ())
    }
}

/// `-c user.name/-c user.email` when the repository has no identity (fresh CI machines).
fn identity(dir: &Path) -> Vec<&'static str> {
    if git(dir, &["config", "user.email"]).is_ok_and(|s| !s.trim().is_empty()) {
        Vec::new()
    } else {
        vec!["-c", "user.name=Kor", "-c", "user.email=kor@localhost"]
    }
}

/// Commit everything the sub-agent changed; `None` when nothing changed.
pub fn commit_all(dir: &Path, message: &str) -> Result<Option<String>, String> {
    let mut add = vec!["add", "-A", "--"];
    add.extend(NOT_KOR);
    git(dir, &add)?;
    if git(dir, &["diff", "--cached", "--quiet"]).is_ok() {
        return Ok(None);
    }
    let mut args = identity(dir);
    args.extend(["commit", "--no-verify", "-q", "-m", message]);
    git(dir, &args)?;
    head(dir).map(Some)
}

fn cut(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n[…]\n");
    }
    s
}

/// Changed files and the diff between `base` and `head` (cut at [`MAX_DIFF`]).
pub fn diff(dir: &Path, base: &str, head: &str) -> Result<(Vec<String>, String), String> {
    let range = format!("{base}..{head}");
    let files = git(dir, &["diff", "--name-only", &range])?
        .lines()
        .map(str::to_owned)
        .collect();
    let text = git(dir, &["diff", &range])?;
    Ok((files, cut(text, MAX_DIFF)))
}

/// One conflicting file: our side (the task branch) and theirs (the sub-agent's).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub ours: String,
    pub theirs: String,
}

/// Result of [`merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merge {
    Merged(String),
    /// Nothing was merged: the attempt was aborted and the worktree is as before.
    Conflict {
        files: Vec<ConflictFile>,
        diff: String,
    },
}

fn abort_pending(dir: &Path) {
    if git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok() {
        let _ = git(dir, &["merge", "--abort"]);
    }
}

/// Merge `branch` into the worktree at `dir`. On a conflict the merge is aborted, so the task
/// branch is never left half-merged; the conflict comes back as per-file sides plus the diff
/// git printed with conflict markers.
pub fn merge(dir: &Path, branch: &str, message: &str) -> Result<Merge, String> {
    abort_pending(dir);
    let mut args = identity(dir);
    args.extend(["merge", "--no-ff", "--no-edit", "-m", message, branch]);
    if git(dir, &args).is_ok() {
        return head(dir).map(Merge::Merged);
    }
    let names: Vec<String> = git(dir, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let text = git(dir, &["diff"]).unwrap_or_default();
    let side = |rev: &str, p: &str| {
        cut(
            git(dir, &["show", &format!("{rev}:{p}")]).unwrap_or_default(),
            MAX_SIDE,
        )
    };
    let files = names
        .iter()
        .map(|p| ConflictFile {
            path: p.clone(),
            ours: side("HEAD", p),
            theirs: side(branch, p),
        })
        .collect();
    let _ = git(dir, &["merge", "--abort"]);
    // Not a content conflict (unrelated failure): report git's own message instead.
    if names.is_empty() {
        return Err(format!("git merge {branch} failed"));
    }
    Ok(Merge::Conflict {
        files,
        diff: cut(text, MAX_DIFF),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_keeps_char_boundaries() {
        let s = cut("ççççç".into(), 3);
        assert!(s.starts_with('ç') && s.contains("[…]"));
        assert_eq!(cut("abc".into(), 10), "abc");
    }
}
