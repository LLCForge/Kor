//! Conversation forking (Master Build Prompt §27.3, ADR 0063).
//!
//! A fork is a new session whose transcript is the parent's first `at` messages; the parent never
//! changes. In a git project the fork also gets its own working copy: a worktree at
//! `<repo>/.kor/worktrees/fork-<id>` on branch `kor/fork/<id>`, started from the parent's current
//! files (`git stash create` for tracked changes, untracked files copied), so each branch writes
//! in its own directory. Fork records live in `<kor home>/forks.json` (plain JSON, lock-serialized);
//! titles, folders and tags stay in the session tree (`session_tree`).
//!
//! The worktree starts from the files *as they were at the fork point*: every file the parent
//! changed after message `at` is put back to its state then, rebuilt from the parent's undo
//! journal (the file before the session touched it) plus the `Write`/`Edit` calls of the first
//! `at` messages. Files changed by other means (`apply_patch`, notebooks) before the fork point
//! cannot be rebuilt and stay as they are now (`unrestored`); `Bash` side effects are not known.
//!
//! Merging takes a branch's conclusion back: its last answer becomes a note in the parent's
//! transcript, and its file changes are offered file by file (ADR 0030) before anything is
//! written; accepted writes go through the parent's undo journal, so `kor undo <parent>` reverts
//! them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kor_core::{Message, Role};
use serde::{Deserialize, Serialize};

use crate::task::git;

/// Characters of a branch's conclusion kept in the merge note.
const MAX_CONCLUSION: usize = 4_000;
/// Files larger than this are not offered for merge (listed as skipped).
const MAX_FILE: u64 = 2 * 1024 * 1024;
/// Untracked files copied into a new fork worktree at most.
const MAX_UNTRACKED: usize = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum ForkError {
    #[error("unknown session `{0}`")]
    UnknownSession(String),
    #[error("`{0}` is not a fork")]
    NotAFork(String),
    #[error("message {at} is out of range (the session has {len} messages)")]
    OutOfRange { at: usize, len: usize },
    #[error("invalid path in a merge: `{0}`")]
    BadPath(String),
    #[error("git: {0}")]
    Git(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{path} is corrupt, fix or delete it: {error}")]
    Corrupt { path: String, error: String },
}

/// One fork.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fork {
    pub id: String,
    pub parent: String,
    /// Messages copied from the parent (the fork point, 1-based message number).
    pub at: usize,
    /// Unix seconds.
    pub created: u64,
    /// Model to continue with ("try this with another model"); `None` keeps the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Project the fork came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    /// The fork's own working copy (git worktree); `None` outside git.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Commit the worktree started from (the parent's files at fork time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Unix seconds of the last merge back into the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<u64>,
    /// Files put back to their state at the fork point (project-relative, `/`-separated).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewound: Vec<String>,
    /// Files changed after the fork point that could not be rebuilt (left as they are now).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unrestored: Vec<String>,
    /// The parent ran shell commands after the fork point; their effects stay in the worktree.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shell_after: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    forks: Vec<Fork>,
}

/// `<kor home>/forks.json`.
pub fn store_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("forks.json"))
}

fn read(path: &Path) -> Result<Store, ForkError> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| ForkError::Corrupt {
            path: path.display().to_string(),
            error: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e.into()),
    }
}

fn update<T>(path: &Path, f: impl FnOnce(&mut Store) -> T) -> Result<T, ForkError> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let _lock = kor_platform::lock_file(&path.with_extension("lock"))?;
    let mut s = read(path)?;
    let out = f(&mut s);
    s.version = 1;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&s).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)?;
    Ok(out)
}

/// Every fork record.
///
/// # Errors
/// When the store is corrupt.
pub fn list(store: &Path) -> Result<Vec<Fork>, ForkError> {
    Ok(read(store)?.forks)
}

/// The fork record of `id`, if `id` is a fork.
///
/// # Errors
/// When the store is corrupt.
pub fn get(store: &Path, id: &str) -> Result<Option<Fork>, ForkError> {
    Ok(read(store)?.forks.into_iter().find(|f| f.id == id))
}

fn transcript(sessions: &Path, id: &str) -> Result<PathBuf, ForkError> {
    let p = sessions.join(format!("{id}.jsonl"));
    if crate::session_tree::valid_id(id) && p.is_file() {
        Ok(p)
    } else {
        Err(ForkError::UnknownSession(id.to_owned()))
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write `messages` as a new session; the id gets a suffix when one exists this second.
fn new_session(sessions: &Path, messages: &[Message]) -> Result<String, ForkError> {
    let base = kor_agent::session::new_id();
    for n in 0..100 {
        let id = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let p = sessions.join(format!("{id}.jsonl"));
        // Reserve the name first so two forks in the same second never overwrite each other.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            Ok(_) => {
                kor_agent::session::save(&p, messages)?;
                return Ok(id);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(ForkError::Io(std::io::Error::other("no free session id")))
}

/// Options of [`fork`].
#[derive(Debug, Clone, Default)]
pub struct Options<'a> {
    pub model: Option<String>,
    /// Project root: in a git repository the fork gets its own worktree.
    pub root: Option<&'a Path>,
    /// Share the parent's directory instead of a worktree.
    pub no_worktree: bool,
    /// Undo journals (`<kor home>/undo`): with it, the worktree starts from the files at the fork
    /// point instead of the parent's current files.
    pub undo_base: Option<&'a Path>,
}

/// One file-changing tool call of a transcript.
enum FileCall {
    Write {
        path: String,
        content: String,
    },
    Edit {
        path: String,
        old: String,
        new: String,
        all: bool,
    },
    /// A tool that changes files in a way that cannot be replayed (`apply_patch`, notebooks).
    Other {
        paths: Vec<String>,
    },
    Shell,
}

fn file_calls(m: &Message) -> Vec<FileCall> {
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    m.parts
        .iter()
        .filter_map(|p| match p {
            kor_core::ContentPart::ToolUse { name, input, .. } => match name.as_str() {
                "Write" => Some(FileCall::Write {
                    path: s(input, "file_path").or_else(|| s(input, "path"))?,
                    content: s(input, "content")?,
                }),
                "Edit" => Some(FileCall::Edit {
                    path: s(input, "file_path").or_else(|| s(input, "path"))?,
                    old: s(input, "old_string")?,
                    new: s(input, "new_string")?,
                    all: input
                        .get("replace_all")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                }),
                "NotebookEdit" | "EditSymbol" | "MultiEdit" | "Delete" | "Move" => {
                    Some(FileCall::Other {
                        paths: [
                            "notebook_path",
                            "file_path",
                            "path",
                            "from",
                            "to",
                            "source",
                            "destination",
                        ]
                        .iter()
                        .filter_map(|k| s(input, k))
                        .collect(),
                    })
                }
                "apply_patch" => Some(FileCall::Other {
                    paths: s(input, "patch")
                        .or_else(|| s(input, "input"))
                        .unwrap_or_default()
                        .lines()
                        .filter_map(|l| {
                            [
                                "*** Update File: ",
                                "*** Add File: ",
                                "*** Delete File: ",
                                "+++ b/",
                                "--- a/",
                            ]
                            .iter()
                            .find_map(|pre| l.strip_prefix(pre))
                            .map(|p| p.trim().to_owned())
                        })
                        .collect(),
                }),
                "Bash" | "PowerShell" | "shell" => Some(FileCall::Shell),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn path_of(call: &FileCall) -> Vec<&str> {
    match call {
        FileCall::Write { path, .. } | FileCall::Edit { path, .. } => vec![path.as_str()],
        FileCall::Other { paths } => paths.iter().map(String::as_str).collect(),
        FileCall::Shell => Vec::new(),
    }
}

/// Lexical `a/./b/../c` → `a/c`, with `rel` resolved against `root`.
fn absolute(root: &Path, rel: &str) -> PathBuf {
    let joined = root.join(rel);
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Put every file the parent changed after message `at` back to its state at that point, inside
/// `worktree` (which mirrors `root`). Returns (rewound, unrestored, shell ran after the point).
fn rewind_to(
    messages: &[Message],
    at: usize,
    journal: &Path,
    root: &Path,
    worktree: &Path,
) -> (Vec<String>, Vec<String>, bool) {
    let before: Vec<FileCall> = messages[..at].iter().flat_map(file_calls).collect();
    let after: Vec<FileCall> = messages[at..].iter().flat_map(file_calls).collect();
    let shell_after = after.iter().any(|c| matches!(c, FileCall::Shell));
    let mut touched: Vec<PathBuf> = Vec::new();
    for c in &after {
        for p in path_of(c) {
            let abs = absolute(root, p);
            if !touched.contains(&abs) {
                touched.push(abs);
            }
        }
    }
    let (mut rewound, mut unrestored) = (Vec::new(), Vec::new());
    for abs in touched {
        let Ok(rel) = abs.strip_prefix(root) else {
            continue;
        };
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let target = worktree.join(rel);
        // The file before the session first touched it, then the replayable calls up to `at`.
        let mut state: Option<Option<String>> = kor_tools::undo::original(journal, &abs)
            .map(|o| o.map(|b| String::from_utf8_lossy(&b).into_owned()));
        for c in before
            .iter()
            .filter(|c| path_of(c).iter().any(|p| absolute(root, p) == abs))
        {
            state = match (state, c) {
                (_, FileCall::Write { content, .. }) => Some(Some(content.clone())),
                (Some(Some(text)), FileCall::Edit { old, new, all, .. })
                    if !old.is_empty() && text.contains(old.as_str()) =>
                {
                    Some(Some(if *all {
                        text.replace(old.as_str(), new)
                    } else {
                        text.replacen(old.as_str(), new, 1)
                    }))
                }
                _ => None,
            };
        }
        let done = match state {
            Some(Some(text)) => target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&target, text))
                .is_ok(),
            Some(None) => !target.exists() || std::fs::remove_file(&target).is_ok(),
            None => false,
        };
        if done {
            rewound.push(rel_s);
        } else {
            unrestored.push(rel_s);
        }
    }
    (rewound, unrestored, shell_after)
}

/// Fork `parent` after message `at` (1-based; `at = len` keeps everything).
///
/// # Errors
/// Unknown session, `at` out of range, I/O or git errors (a failed worktree removes the new
/// session again).
pub fn fork(
    sessions: &Path,
    store: &Path,
    parent: &str,
    at: usize,
    opts: &Options<'_>,
) -> Result<Fork, ForkError> {
    let messages = kor_agent::session::load(&transcript(sessions, parent)?)?;
    if at == 0 || at > messages.len() {
        return Err(ForkError::OutOfRange {
            at,
            len: messages.len(),
        });
    }
    let id = new_session(sessions, &messages[..at])?;
    let mut f = Fork {
        id: id.clone(),
        parent: parent.to_owned(),
        at,
        created: now(),
        model: opts.model.clone().filter(|m| !m.trim().is_empty()),
        root: opts.root.map(Path::to_path_buf),
        worktree: None,
        branch: None,
        base: None,
        merged: None,
        rewound: Vec::new(),
        unrestored: Vec::new(),
        shell_after: false,
    };
    if let Some(root) = opts.root.filter(|_| !opts.no_worktree)
        && let Ok(repo) = git::repo_root(root)
    {
        match working_copy(&repo, &id) {
            Ok((wt, branch, base)) => {
                // Same place inside the worktree as `root` inside the repository.
                let sub = root
                    .strip_prefix(&repo)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                f.worktree = Some(wt.join(sub));
                f.branch = Some(branch);
                f.base = Some(base);
                if at < messages.len()
                    && let (Some(undo), Some(wt)) = (opts.undo_base, &f.worktree)
                {
                    (f.rewound, f.unrestored, f.shell_after) =
                        rewind_to(&messages, at, &undo.join(parent), root, wt);
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(sessions.join(format!("{id}.jsonl")));
                return Err(e);
            }
        }
    }
    let rec = f.clone();
    update(store, |s| s.forks.push(rec))?;
    Ok(f)
}

/// Worktree for fork `id` holding the repository's current files.
fn working_copy(repo: &Path, id: &str) -> Result<(PathBuf, String, String), ForkError> {
    let g = |args: &[&str]| git::git(repo, args).map_err(ForkError::Git);
    // `git stash create` commits tracked changes without touching the working tree; empty when clean.
    let stash = g(&["stash", "create", "kor fork"])?.trim().to_owned();
    let base = if stash.is_empty() {
        git::head(repo).map_err(ForkError::Git)?
    } else {
        stash
    };
    git::exclude_kor_dirs(repo).map_err(ForkError::Git)?;
    let path = repo
        .join(".kor")
        .join("worktrees")
        .join(format!("fork-{id}"));
    let branch = format!("kor/fork/{id}");
    git::fresh_worktree(repo, &path, &branch, &base).map_err(ForkError::Git)?;
    let untracked = g(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    for rel in untracked
        .split('\0')
        .filter(|r| !r.is_empty() && !r.starts_with(".kor/"))
        .take(MAX_UNTRACKED)
    {
        let (src, dst) = (repo.join(rel), path.join(rel));
        if std::fs::metadata(&src).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE) {
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok((path, branch, base))
}

/// A node of the fork tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub title: String,
    /// Fork point in the parent; `None` for a root.
    pub at: Option<usize>,
    pub model: Option<String>,
    pub merged: Option<u64>,
    pub worktree: Option<PathBuf>,
    /// The transcript is gone (deleted session).
    pub missing: bool,
    pub children: Vec<Node>,
}

fn title(sessions: &Path, id: &str, titles: &BTreeMap<String, String>) -> (String, bool) {
    let p = sessions.join(format!("{id}.jsonl"));
    if !p.is_file() {
        return (String::new(), true);
    }
    let t = titles.get(id).cloned().unwrap_or_else(|| {
        kor_agent::session::load(&p)
            .ok()
            .and_then(|m| {
                m.iter()
                    .find(|m| m.role == Role::User)
                    .map(Message::plain_text)
            })
            .map(|t| {
                t.lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect()
            })
            .unwrap_or_default()
    });
    (t, false)
}

/// Fork trees: every session that has forks (or is one) with its branches below it. `only`
/// keeps the tree that contains that session.
///
/// # Errors
/// When a store is corrupt.
pub fn tree(
    sessions: &Path,
    store: &Path,
    meta: Option<&Path>,
    only: Option<&str>,
) -> Result<Vec<Node>, ForkError> {
    let forks = list(store)?;
    let titles: BTreeMap<String, String> = meta
        .and_then(|m| {
            crate::session_tree::list(
                sessions,
                m,
                &crate::session_tree::Filter {
                    archived: true,
                    ..Default::default()
                },
            )
            .ok()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, e.title))
        .collect();
    let by_id: BTreeMap<&str, &Fork> = forks.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut children: BTreeMap<&str, Vec<&Fork>> = BTreeMap::new();
    for f in &forks {
        children.entry(f.parent.as_str()).or_default().push(f);
    }
    #[allow(clippy::items_after_statements)] // helper kept next to its only use
    fn build(
        id: &str,
        fork: Option<&Fork>,
        sessions: &Path,
        titles: &BTreeMap<String, String>,
        children: &BTreeMap<&str, Vec<&Fork>>,
        depth: usize,
    ) -> Node {
        let (title, missing) = title(sessions, id, titles);
        let mut kids: Vec<Node> = if depth > 64 {
            Vec::new()
        } else {
            children.get(id).map_or_else(Vec::new, |v| {
                v.iter()
                    .map(|c| build(&c.id, Some(c), sessions, titles, children, depth + 1))
                    .collect()
            })
        };
        kids.sort_by(|a, b| a.at.cmp(&b.at).then(a.id.cmp(&b.id)));
        Node {
            id: id.to_owned(),
            title,
            at: fork.map(|f| f.at),
            model: fork.and_then(|f| f.model.clone()),
            merged: fork.and_then(|f| f.merged),
            worktree: fork.and_then(|f| f.worktree.clone()),
            missing,
            children: kids,
        }
    }
    // Roots: parents that are not forks themselves (a fork whose parent record is gone is still
    // under that parent id, shown as missing).
    let roots: BTreeSet<&str> = forks
        .iter()
        .map(|f| {
            let mut id = f.parent.as_str();
            let mut guard = 0;
            while let Some(p) = by_id.get(id) {
                id = p.parent.as_str();
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            id
        })
        .collect();
    let mut out: Vec<Node> = roots
        .into_iter()
        .map(|r| build(r, None, sessions, &titles, &children, 0))
        .collect();
    if let Some(id) = only {
        out.retain(|n| contains(n, id));
    }
    Ok(out)
}

fn contains(n: &Node, id: &str) -> bool {
    n.id == id || n.children.iter().any(|c| contains(c, id))
}

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub id: String,
    /// Messages after the common history.
    pub after: usize,
    /// Last answer of the side.
    pub conclusion: String,
    /// Directory the side writes in.
    pub dir: Option<PathBuf>,
    /// Files the side changed since the fork point.
    pub changed: Vec<String>,
}

/// Two branches side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// Messages both share from the start.
    pub common: usize,
    pub a: Side,
    pub b: Side,
    /// Unified diff of the two sides' files, per path changed on either side.
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
    pub added: usize,
    pub removed: usize,
}

fn conclusion(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.plain_text().trim().is_empty())
        .map(|m| clip(m.plain_text().trim(), MAX_CONCLUSION))
        .unwrap_or_default()
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// Files changed in `dir` since `base` (tracked changes and untracked files), repo-relative.
fn changed_since(dir: &Path, base: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    if let Ok(t) = git::git(dir, &["diff", "--name-only", "-z", base, "--"]) {
        out.extend(t.split('\0').filter(|s| !s.is_empty()).map(str::to_owned));
    }
    if let Ok(t) = git::git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        out.extend(
            t.split('\0')
                .filter(|s| !s.is_empty() && !s.starts_with(".kor/"))
                .map(str::to_owned),
        );
    }
    out.into_iter().collect()
}

fn read_small(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    if !m.is_file() || m.len() > MAX_FILE {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

/// Directory and git base of a session: a fork's worktree, else the project of its fork record
/// (a root session writes in the project itself).
fn side_dir(f: Option<&Fork>, root_of_tree: Option<&Fork>) -> (Option<PathBuf>, Option<String>) {
    match f {
        Some(f) => (
            f.worktree.clone().or_else(|| f.root.clone()),
            f.base.clone(),
        ),
        None => (
            root_of_tree.and_then(|r| r.root.clone()),
            root_of_tree.and_then(|r| r.base.clone()),
        ),
    }
}

/// Compare two sessions of the same tree: shared history, each side's conclusion and the
/// difference between their files.
///
/// # Errors
/// Unknown session or corrupt store.
pub fn compare(sessions: &Path, store: &Path, a: &str, b: &str) -> Result<Comparison, ForkError> {
    let ma = kor_agent::session::load(&transcript(sessions, a)?)?;
    let mb = kor_agent::session::load(&transcript(sessions, b)?)?;
    let common = ma.iter().zip(&mb).take_while(|(x, y)| x == y).count();
    let forks = list(store)?;
    let rec = |id: &str| forks.iter().find(|f| f.id == id);
    let child_of = |id: &str| forks.iter().find(|f| f.parent == id);
    let (fa, fb) = (rec(a), rec(b));
    let (da, ba) = side_dir(fa, fb.or_else(|| child_of(a)));
    let (db, bb) = side_dir(fb, fa.or_else(|| child_of(b)));
    let changed = |d: &Option<PathBuf>, base: &Option<String>| match (d, base) {
        (Some(d), Some(base)) => changed_since(d, base),
        _ => Vec::new(),
    };
    let side = |id: &str, m: &[Message], dir: Option<PathBuf>, ch: Vec<String>| Side {
        id: id.to_owned(),
        after: m.len().saturating_sub(common),
        conclusion: conclusion(&m[common.min(m.len())..]),
        dir,
        changed: ch,
    };
    let ca = changed(&da, &ba);
    let cb = changed(&db, &bb);
    let paths: BTreeSet<&String> = ca.iter().chain(&cb).collect();
    let mut files = Vec::new();
    if let (Some(x), Some(y)) = (&da, &db) {
        for p in paths {
            let (l, r) = (read_small(&x.join(p)), read_small(&y.join(p)));
            let diff = crate::textdiff::unified(p, l.as_deref(), r.as_deref());
            if !diff.is_empty() {
                let (added, removed) = crate::textdiff::stats(&diff);
                files.push(FileDiff {
                    path: p.clone(),
                    diff,
                    added,
                    removed,
                });
            }
        }
    }
    Ok(Comparison {
        common,
        a: side(a, &ma, da, ca),
        b: side(b, &mb, db, cb),
        files,
    })
}

/// What merging a fork back would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePlan {
    pub fork: String,
    pub parent: String,
    /// The note appended to the parent's transcript.
    pub summary: String,
    /// Directory the files are written in (the parent's working copy).
    pub target: Option<PathBuf>,
    /// One proposal per file the fork changed: `before` in the parent, `after` in the fork.
    pub files: Vec<kor_tools::Proposal>,
    /// Changed files not offered (too large or not text).
    pub skipped: Vec<String>,
}

/// The parent's working copy: its own worktree when the parent is a fork, else the project.
fn target_dir(forks: &[Fork], f: &Fork) -> Option<PathBuf> {
    forks
        .iter()
        .find(|p| p.id == f.parent)
        .and_then(|p| p.worktree.clone())
        .or_else(|| f.root.clone())
}

/// Plan a merge of `fork` into its parent.
///
/// # Errors
/// Not a fork, unknown session or corrupt store.
pub fn merge_plan(sessions: &Path, store: &Path, fork: &str) -> Result<MergePlan, ForkError> {
    let forks = list(store)?;
    let f = forks
        .iter()
        .find(|f| f.id == fork)
        .ok_or_else(|| ForkError::NotAFork(fork.to_owned()))?;
    let msgs = kor_agent::session::load(&transcript(sessions, fork)?)?;
    transcript(sessions, &f.parent)?;
    let target = target_dir(&forks, f);
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    if let (Some(wt), Some(base), Some(target)) = (&f.worktree, &f.base, &target) {
        for p in changed_since(wt, base) {
            let (src, dst) = (wt.join(&p), target.join(&p));
            let after = if src.exists() { read_small(&src) } else { None };
            if src.exists() && after.is_none() {
                skipped.push(p);
                continue;
            }
            let before = if dst.exists() {
                if let Some(b) = read_small(&dst) {
                    Some(b)
                } else {
                    skipped.push(p);
                    continue;
                }
            } else {
                None
            };
            if before != after {
                files.push(kor_tools::Proposal {
                    path: p,
                    before,
                    after,
                });
            }
        }
    }
    let mut summary = format!(
        "[Kor] Merged branch {fork} (forked at message {}{}).",
        f.at,
        f.model
            .as_deref()
            .map(|m| format!(", model {m}"))
            .unwrap_or_default()
    );
    let c = conclusion(&msgs[f.at.min(msgs.len())..]);
    if !c.is_empty() {
        summary.push_str("\nConclusion of the branch:\n");
        summary.push_str(&c);
    }
    if !files.is_empty() {
        summary.push_str("\nFiles changed on the branch: ");
        summary.push_str(
            &files
                .iter()
                .map(|x| x.path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(MergePlan {
        fork: fork.to_owned(),
        parent: f.parent.clone(),
        summary,
        target,
        files,
        skipped,
    })
}

/// Result of [`merge_apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Merged {
    pub written: Vec<String>,
    pub note: bool,
}

/// `path` relative and inside the target (no `..`, no absolute paths).
fn safe_rel(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    (!path.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_))))
    .then(|| p.to_path_buf())
}

/// Apply a merge: write the accepted files (`Some` content, `None` deletes) into the parent's
/// working copy through the parent's undo journal, then append the summary note to the parent's
/// transcript (`note = false` skips it).
///
/// # Errors
/// Not a fork, a path outside the target, I/O errors.
pub fn merge_apply(
    sessions: &Path,
    store: &Path,
    undo_base: Option<&Path>,
    fork: &str,
    accepted: &[(String, Option<String>)],
    note: Option<&str>,
) -> Result<Merged, ForkError> {
    let forks = list(store)?;
    let f = forks
        .iter()
        .find(|f| f.id == fork)
        .ok_or_else(|| ForkError::NotAFork(fork.to_owned()))?;
    let parent_file = transcript(sessions, &f.parent)?;
    let mut written = Vec::new();
    if !accepted.is_empty() {
        let target =
            target_dir(&forks, f).ok_or_else(|| ForkError::BadPath("<no working copy>".into()))?;
        let rels: Vec<PathBuf> = accepted
            .iter()
            .map(|(p, _)| safe_rel(p).ok_or_else(|| ForkError::BadPath(p.clone())))
            .collect::<Result<_, _>>()?;
        let mut journal = match undo_base {
            Some(b) => Some(kor_tools::undo::Journal::open(
                b,
                &f.parent,
                std::slice::from_ref(&target),
            )?),
            None => None,
        };
        for ((path, content), rel) in accepted.iter().zip(&rels) {
            if let Some(j) = journal.as_mut() {
                j.record(&target, path)?;
            }
            let dst = target.join(rel);
            match content {
                Some(c) => {
                    if let Some(d) = dst.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    let tmp = dst.with_extension("kor-merge.tmp");
                    std::fs::write(&tmp, c)?;
                    std::fs::rename(&tmp, &dst)?;
                }
                None => match std::fs::remove_file(&dst) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                },
            }
            written.push(path.clone());
        }
    }
    let mut noted = false;
    if let Some(text) = note.filter(|t| !t.trim().is_empty()) {
        let mut msgs = kor_agent::session::load(&parent_file)?;
        msgs.push(Message::text(Role::User, text));
        kor_agent::session::save(&parent_file, &msgs)?;
        noted = true;
    }
    let id = fork.to_owned();
    let t = now();
    update(store, |s| {
        if let Some(r) = s.forks.iter_mut().find(|r| r.id == id) {
            r.merged = Some(t);
        }
    })?;
    Ok(Merged {
        written,
        note: noted,
    })
}

/// Forget a fork: its record, and with `worktree` its working copy and branch. The transcript
/// stays (delete it like any session).
///
/// # Errors
/// Not a fork or the store cannot be written.
pub fn remove(store: &Path, fork: &str, worktree: bool) -> Result<(), ForkError> {
    let f = get(store, fork)?.ok_or_else(|| ForkError::NotAFork(fork.to_owned()))?;
    if worktree
        && let (Some(wt), Some(root)) = (&f.worktree, &f.root)
        && let Ok(repo) = git::repo_root(root)
    {
        let dir = repo
            .join(".kor")
            .join("worktrees")
            .join(format!("fork-{fork}"));
        if wt.starts_with(&dir) {
            git::remove_worktree(&repo, &dir, f.branch.as_deref());
        }
    }
    let id = fork.to_owned();
    update(store, |s| s.forks.retain(|r| r.id != id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| {
                Message::text(
                    if i % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    format!("m{i}"),
                )
            })
            .collect()
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        git::git(dir, args).unwrap();
    }

    #[test]
    fn fork_tree_and_out_of_range() {
        let d = tempfile::tempdir().unwrap();
        let (sessions, store) = (d.path().join("s"), d.path().join("forks.json"));
        std::fs::create_dir_all(&sessions).unwrap();
        kor_agent::session::save(&sessions.join("20260101-000000-1.jsonl"), &msgs(4)).unwrap();
        let f = fork(
            &sessions,
            &store,
            "20260101-000000-1",
            2,
            &Options {
                model: Some("anthropic/claude".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let copied = kor_agent::session::load(&sessions.join(format!("{}.jsonl", f.id))).unwrap();
        assert_eq!(copied, msgs(2));
        assert!(f.worktree.is_none());
        let sub = fork(&sessions, &store, &f.id, 1, &Options::default()).unwrap();
        assert_ne!(sub.id, f.id);
        assert!(matches!(
            fork(
                &sessions,
                &store,
                "20260101-000000-1",
                9,
                &Options::default()
            ),
            Err(ForkError::OutOfRange { at: 9, len: 4 })
        ));
        assert!(matches!(
            fork(&sessions, &store, "../x", 1, &Options::default()),
            Err(ForkError::UnknownSession(_))
        ));
        let t = tree(&sessions, &store, None, None).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "20260101-000000-1");
        assert_eq!(t[0].title, "m0");
        assert_eq!(t[0].children[0].id, f.id);
        assert_eq!(t[0].children[0].model.as_deref(), Some("anthropic/claude"));
        assert_eq!(t[0].children[0].children[0].id, sub.id);
        assert_eq!(
            tree(&sessions, &store, None, Some(&sub.id)).unwrap().len(),
            1
        );
        assert!(
            tree(&sessions, &store, None, Some("other"))
                .unwrap()
                .is_empty()
        );
        remove(&store, &sub.id, false).unwrap();
        assert!(get(&store, &sub.id).unwrap().is_none());
    }

    fn tool(name: &str, input: serde_json::Value) -> Message {
        Message {
            role: Role::Assistant,
            parts: vec![kor_core::ContentPart::ToolUse {
                id: "t".into(),
                name: name.into(),
                input,
            }],
        }
    }

    fn text(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
    }

    #[test]
    #[allow(clippy::many_single_char_names)] // short names of the math or the protocol
    fn worktree_starts_from_the_files_at_the_fork_point() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-q"]);
        std::fs::write(repo.join("keep.txt"), "base\n").unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        );
        let repo = git::repo_root(&repo).unwrap();
        // The parent: message 2 writes a.txt; message 4 edits it, writes b.txt and runs a shell command.
        let a = repo.join("a.txt").to_string_lossy().into_owned();
        let parent = vec![
            Message::text(Role::User, "write a"),
            tool(
                "Write",
                serde_json::json!({ "file_path": a, "content": "v1\n" }),
            ),
            Message::text(Role::User, "now change it"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    kor_core::ContentPart::ToolUse {
                        id: "e".into(),
                        name: "Edit".into(),
                        input: serde_json::json!({ "file_path": "a.txt", "old_string": "v1", "new_string": "v2" }),
                    },
                    kor_core::ContentPart::ToolUse {
                        id: "w".into(),
                        name: "Write".into(),
                        input: serde_json::json!({ "file_path": "b.txt", "content": "new\n" }),
                    },
                    kor_core::ContentPart::ToolUse {
                        id: "s".into(),
                        name: "Bash".into(),
                        input: serde_json::json!({ "command": "npm i" }),
                    },
                ],
            },
        ];
        // The session's undo journal, recorded before the first change (both files were new).
        let undo = d.path().join("undo");
        let mut j =
            kor_tools::undo::Journal::open(&undo, "p-2", std::slice::from_ref(&repo)).unwrap();
        j.record(&repo, "a.txt").unwrap();
        j.record(&repo, "b.txt").unwrap();
        // The files as the parent left them.
        std::fs::write(repo.join("a.txt"), "v2\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        let (sessions, store) = (d.path().join("s"), d.path().join("forks.json"));
        std::fs::create_dir_all(&sessions).unwrap();
        kor_agent::session::save(&sessions.join("p-2.jsonl"), &parent).unwrap();
        let opts = Options {
            root: Some(&repo),
            undo_base: Some(&undo),
            ..Default::default()
        };
        let f = fork(&sessions, &store, "p-2", 2, &opts).unwrap();
        let wt = f.worktree.clone().unwrap();
        assert_eq!(text(&wt.join("a.txt")), "v1\n");
        assert!(
            !wt.join("b.txt").exists(),
            "b.txt came after the fork point"
        );
        assert_eq!(text(&wt.join("keep.txt")), "base\n");
        let mut r = f.rewound.clone();
        r.sort();
        assert_eq!(r, ["a.txt", "b.txt"]);
        assert!(f.unrestored.is_empty() && f.shell_after);
        // The parent's own files are untouched.
        assert_eq!(text(&repo.join("a.txt")), "v2\n");
        // Forking at the end keeps today's files.
        let g = fork(&sessions, &store, "p-2", 4, &opts).unwrap();
        assert!(g.rewound.is_empty());
        assert_eq!(text(&g.worktree.unwrap().join("a.txt")), "v2\n");
    }

    #[test]
    #[allow(clippy::many_single_char_names, clippy::too_many_lines)] // short names of the math or the protocol; one linear sequence; splitting it would scatter the steps
    fn worktree_compare_and_merge_with_undo() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        );
        // Uncommitted and untracked work is carried into the fork.
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("new.txt"), "fresh\n").unwrap();
        let repo = git::repo_root(&repo).unwrap();
        let (sessions, store) = (d.path().join("s"), d.path().join("forks.json"));
        std::fs::create_dir_all(&sessions).unwrap();
        kor_agent::session::save(&sessions.join("p-1.jsonl"), &msgs(2)).unwrap();
        let f = fork(
            &sessions,
            &store,
            "p-1",
            2,
            &Options {
                root: Some(&repo),
                ..Default::default()
            },
        )
        .unwrap();
        let wt = f.worktree.clone().unwrap();
        assert_eq!(
            std::fs::read_to_string(wt.join("a.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "one\ntwo\n"
        );
        assert_eq!(
            std::fs::read_to_string(wt.join("new.txt")).unwrap(),
            "fresh\n"
        );
        // The branch works on its own copy; the parent's files do not move.
        std::fs::write(wt.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(wt.join("b.txt"), "bee\n").unwrap();
        let mut m = msgs(2);
        m.push(Message::text(Role::User, "go"));
        m.push(Message::text(Role::Assistant, "done: added three"));
        kor_agent::session::save(&sessions.join(format!("{}.jsonl", f.id)), &m).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "one\ntwo\n"
        );

        let c = compare(&sessions, &store, "p-1", &f.id).unwrap();
        assert_eq!(c.common, 2);
        assert_eq!(c.b.after, 2);
        assert_eq!(c.b.conclusion, "done: added three");
        assert!(c.b.changed.contains(&"b.txt".to_owned()));
        let a_diff = c.files.iter().find(|x| x.path == "a.txt").unwrap();
        assert!(a_diff.diff.contains("+three"));

        let plan = merge_plan(&sessions, &store, &f.id).unwrap();
        assert_eq!(plan.parent, "p-1");
        assert!(plan.summary.contains("done: added three"));
        let paths: Vec<&str> = plan.files.iter().map(|x| x.path.as_str()).collect();
        assert!(paths.contains(&"a.txt") && paths.contains(&"b.txt"));
        // Accept only a.txt.
        let a = plan.files.iter().find(|x| x.path == "a.txt").unwrap();
        let undo = d.path().join("undo");
        let r = merge_apply(
            &sessions,
            &store,
            Some(&undo),
            &f.id,
            &[(a.path.clone(), a.after.clone())],
            Some(&plan.summary),
        )
        .unwrap();
        assert_eq!(r.written, vec!["a.txt".to_owned()]);
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );
        assert!(!repo.join("b.txt").exists());
        let parent = kor_agent::session::load(&sessions.join("p-1.jsonl")).unwrap();
        assert!(
            parent
                .last()
                .unwrap()
                .plain_text()
                .contains("Merged branch")
        );
        assert!(get(&store, &f.id).unwrap().unwrap().merged.is_some());
        // The parent's undo journal restores the file.
        kor_tools::undo::undo(&undo.join("p-1")).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "one\ntwo\n"
        );
        assert!(matches!(
            merge_apply(
                &sessions,
                &store,
                None,
                &f.id,
                &[("../x".into(), Some(String::new()))],
                None
            ),
            Err(ForkError::BadPath(_))
        ));
        remove(&store, &f.id, true).unwrap();
        assert!(!wt.exists());
    }
}
