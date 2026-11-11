//! Session undo journal (`kor undo`).
//!
//! Before a file tool changes a path for the first time in a session, its original state is
//! recorded on disk: the bytes (or "did not exist") and any parent directories the change will
//! create. `undo` restores every recorded path to that first state, so one command reverts all
//! file changes of the session, from the CLI or the desktop.
//!
//! Layout: `<base>/<session>/manifest.jsonl` (one entry per path) and `<base>/<session>/blobs/<n>`.
//! Plain files and JSON lines, readable without Kor.
//!
//! Shell commands (`Bash`, `PowerShell`) are covered too, after the fact: [`scan_before`] lists the
//! project (and, in git, snapshots tracked files with `git stash create`) before the command,
//! [`Journal::record_shell`] compares afterwards. New files and folders (`node_modules/`, build
//! output) are recorded as "did not exist", so undo deletes them; changed or deleted tracked files
//! get their bytes from the git snapshot. A changed untracked file has no earlier copy and is
//! reported as not restorable. This works on every OS without root (no overlayfs/btrfs snapshot).

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Files above this size are not copied; undo reports them as not restorable.
const MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    /// `None`: the path did not exist (undo deletes it). `Some(n)`: original bytes in `blobs/<n>`.
    pub blob: Option<u64>,
    /// The original was too large to copy.
    #[serde(default)]
    pub skipped: bool,
    /// Directories that did not exist before (removed on undo when empty), deepest first.
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
}

/// One session's journal. Cheap to clone the path; entries live on disk.
#[derive(Debug)]
pub struct Journal {
    dir: PathBuf,
    /// Paths may be recorded only inside these roots (the project and allowed extra roots).
    roots: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    next_blob: u64,
}

/// Folders whose contents are not listed when they already exist (only their creation is tracked).
const HEAVY_DIRS: &[&str] = &["node_modules", "target", ".venv", "venv", "__pycache__", ".next", "dist", "build", ".gradle", "vendor"];
/// Listing stops after this many entries or this long (the shell journal is then skipped).
const SCAN_MAX_ENTRIES: usize = 200_000;
const SCAN_MAX_TIME: std::time::Duration = std::time::Duration::from_millis(2_000);

/// The project as it was before a shell command ([`scan_before`]).
#[derive(Debug, Default)]
pub struct Before {
    root: PathBuf,
    files: std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>,
    dirs: HashSet<PathBuf>,
    /// `git stash create` commit (or `HEAD` when clean) holding tracked files' bytes.
    snapshot: Option<String>,
    repo: Option<PathBuf>,
    /// `false` when the listing hit its limits; then nothing is recorded afterwards.
    pub complete: bool,
}

/// What [`Journal::record_shell`] recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShellChanges {
    pub created: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
    /// Changed or deleted, but no earlier copy exists (untracked, or too large).
    pub unrestorable: Vec<PathBuf>,
}

fn listing(root: &Path) -> (std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>, HashSet<PathBuf>, bool) {
    let started = std::time::Instant::now();
    let (mut files, mut dirs) = (std::collections::HashMap::new(), HashSet::new());
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > SCAN_MAX_ENTRIES || started.elapsed() > SCAN_MAX_TIME {
                return (files, dirs, false);
            }
            let p = e.path();
            let name = e.file_name();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if dir == root && (name == ".git" || name == ".kor") {
                    continue;
                }
                dirs.insert(p.clone());
                if !HEAVY_DIRS.iter().any(|h| name == *h) {
                    stack.push(p);
                }
            } else if let Ok(m) = e.metadata() {
                files.insert(p, (m.len(), m.modified().ok()));
            }
        }
    }
    (files, dirs, true)
}

fn git(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    use kor_platform::NoWindow as _;
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .no_window()
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// List `root` (and snapshot tracked files in git) before a shell command runs.
pub fn scan_before(root: &Path) -> Before {
    let (files, dirs, complete) = listing(root);
    let repo = git(root, &["rev-parse", "--show-toplevel"])
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o).trim()))
        .filter(|p| !p.as_os_str().is_empty());
    let snapshot = repo.as_ref().and_then(|r| {
        let s = git(r, &["stash", "create", "kor undo"]).map(|o| String::from_utf8_lossy(&o).trim().to_owned()).filter(|s| !s.is_empty());
        s.or_else(|| git(r, &["rev-parse", "HEAD"]).map(|o| String::from_utf8_lossy(&o).trim().to_owned()))
    });
    Before { root: root.to_path_buf(), files, dirs, snapshot, repo, complete }
}

/// Lexical normalization (`a/./b/../c` → `a/c`) without touching the file system.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

impl Journal {
    /// Open (or continue) the journal of `session` under `base`.
    ///
    /// # Errors
    /// If the session id is unsafe or the directory cannot be created.
    pub fn open(base: &Path, session: &str, roots: &[PathBuf]) -> std::io::Result<Self> {
        let safe = !session.is_empty()
            && session.len() <= 128
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !safe {
            return Err(std::io::Error::other("invalid session id"));
        }
        let dir = base.join(session);
        // Created on the first record: sessions that change nothing leave no folder behind.
        let entries = read_entries(&dir);
        let next_blob = entries
            .iter()
            .filter_map(|e| e.blob)
            .max()
            .map_or(0, |n| n + 1);
        Ok(Self {
            seen: entries.into_iter().map(|e| e.path).collect(),
            dir,
            roots: roots.iter().map(|r| normalize(r)).collect(),
            next_blob,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Record what a shell command changed since `before` (see the module docs).
    ///
    /// # Errors
    /// If the journal cannot be written.
    pub fn record_shell(&mut self, before: &Before) -> std::io::Result<ShellChanges> {
        let mut out = ShellChanges::default();
        if !before.complete {
            return Ok(out);
        }
        let (files, dirs, complete) = listing(&before.root);
        if !complete {
            return Ok(out);
        }
        // New: the top-most path that did not exist (a new `node_modules/` is one entry).
        let new_dirs: Vec<&PathBuf> = dirs.iter().filter(|d| !before.dirs.contains(*d)).collect();
        let mut created: Vec<PathBuf> = new_dirs
            .iter()
            .filter(|d| d.parent().is_none_or(|p| p == before.root || before.dirs.contains(p)))
            .map(|d| (*d).clone())
            .collect();
        created.extend(
            files
                .keys()
                .filter(|f| !before.files.contains_key(*f) && f.parent().is_none_or(|p| p == before.root || before.dirs.contains(p)))
                .cloned(),
        );
        created.sort();
        for p in created {
            let p = normalize(&p);
            if self.seen.contains(&p) || !self.roots.iter().any(|r| p.starts_with(r)) {
                continue;
            }
            self.append(&Entry { path: p.clone(), blob: None, skipped: false, created_dirs: Vec::new() })?;
            self.seen.insert(p.clone());
            out.created.push(p);
        }
        // Changed or deleted files: their bytes come from the git snapshot.
        let mut touched: Vec<&PathBuf> = before
            .files
            .iter()
            .filter(|(p, old)| files.get(*p) != Some(*old))
            .map(|(p, _)| p)
            .collect();
        touched.sort();
        for p in touched {
            let np = normalize(p);
            if self.seen.contains(&np) || !self.roots.iter().any(|r| np.starts_with(r)) {
                continue;
            }
            // `<rev>:./path` resolves against `git -C <root>`; git's own top-level path may be
            // spelled differently (`C:/…` vs `\\?\C:\…`), the project root is not.
            let bytes = match (&before.repo, &before.snapshot) {
                (Some(_), Some(snap)) => np
                    .strip_prefix(normalize(&before.root))
                    .ok()
                    .and_then(|rel| git(&before.root, &["show", &format!("{snap}:./{}", rel.to_string_lossy().replace('\\', "/"))])),
                _ => None,
            };
            let mut entry = Entry { path: np.clone(), blob: None, skipped: false, created_dirs: Vec::new() };
            match bytes {
                Some(b) if (b.len() as u64) <= MAX_BLOB_BYTES => {
                    fs::create_dir_all(self.dir.join("blobs"))?;
                    let n = self.next_blob;
                    fs::write(self.dir.join("blobs").join(n.to_string()), &b)?;
                    self.next_blob += 1;
                    entry.blob = Some(n);
                    out.changed.push(np.clone());
                }
                _ => {
                    entry.skipped = true;
                    out.unrestorable.push(np.clone());
                }
            }
            self.append(&entry)?;
            self.seen.insert(np);
        }
        Ok(out)
    }

    fn append(&self, entry: &Entry) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(self.dir.join("manifest.jsonl"))?;
        writeln!(f, "{}", serde_json::to_string(entry).map_err(std::io::Error::other)?)
    }

    /// Record the current state of `raw` (relative to `cwd`) unless already recorded.
    ///
    /// # Errors
    /// If the original cannot be copied; the caller must then not run the change.
    pub fn record(&mut self, cwd: &Path, raw: &str) -> std::io::Result<()> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(());
        }
        let path = normalize(&cwd.join(raw));
        if !self.roots.iter().any(|r| path.starts_with(r)) || self.seen.contains(&path) {
            return Ok(());
        }
        fs::create_dir_all(self.dir.join("blobs"))?;
        let mut entry = Entry {
            path: path.clone(),
            blob: None,
            skipped: false,
            created_dirs: Vec::new(),
        };
        match fs::metadata(&path) {
            Ok(m) if m.is_file() => {
                if m.len() > MAX_BLOB_BYTES {
                    entry.skipped = true;
                } else {
                    let n = self.next_blob;
                    fs::copy(&path, self.dir.join("blobs").join(n.to_string()))?;
                    entry.blob = Some(n);
                    self.next_blob += 1;
                }
            }
            // A directory (Move/Delete of a folder) is not copied recursively; recorded as skipped.
            Ok(_) => entry.skipped = true,
            Err(_) => {
                let mut d = path.parent();
                while let Some(p) = d {
                    if p.exists() || !self.roots.iter().any(|r| p.starts_with(r) && p != r) {
                        break;
                    }
                    entry.created_dirs.push(p.to_owned());
                    d = p.parent();
                }
            }
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("manifest.jsonl"))?;
        writeln!(
            f,
            "{}",
            serde_json::to_string(&entry).map_err(std::io::Error::other)?
        )?;
        self.seen.insert(path);
        Ok(())
    }
}

/// The first state recorded for `path` in the journal at `dir`: `Some(None)` = it did not exist,
/// `Some(Some(bytes))` = its bytes; `None` = never recorded or too large to have been copied.
pub fn original(dir: &Path, path: &Path) -> Option<Option<Vec<u8>>> {
    let want = normalize(path);
    let e = read_entries(dir).into_iter().find(|e| normalize(&e.path) == want)?;
    match (e.blob, e.skipped) {
        (_, true) => None,
        (None, false) => Some(None),
        (Some(n), false) => fs::read(dir.join("blobs").join(n.to_string())).ok().map(Some),
    }
}

fn read_entries(dir: &Path) -> Vec<Entry> {
    fs::read_to_string(dir.join("manifest.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// What `undo` did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub restored: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
    /// Too large or a directory: left as is.
    pub skipped: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// Restore every path of the journal in `dir` to its recorded state, then archive the manifest so
/// a second undo does nothing (`manifest.undone-<n>.jsonl`).
///
/// # Errors
/// If the journal has no manifest.
pub fn undo(dir: &Path) -> std::io::Result<Report> {
    let manifest = dir.join("manifest.jsonl");
    if !manifest.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "nothing to undo",
        ));
    }
    let mut report = Report::default();
    for e in read_entries(dir) {
        if e.skipped {
            report.skipped.push(e.path);
            continue;
        }
        let result = match e.blob {
            Some(n) => e
                .path
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| {
                    fs::copy(dir.join("blobs").join(n.to_string()), &e.path).map(|_| ())
                }),
            None => match fs::symlink_metadata(&e.path) {
                Ok(m) if m.is_dir() => fs::remove_dir_all(&e.path),
                Ok(_) => fs::remove_file(&e.path),
                Err(_) => Ok(()),
            },
        };
        match result {
            Ok(()) => {
                if e.blob.is_some() {
                    report.restored.push(e.path);
                } else {
                    // Best effort: directories the change created, if now empty.
                    for d in &e.created_dirs {
                        let _ = fs::remove_dir(d);
                    }
                    report.deleted.push(e.path);
                }
            }
            Err(err) => report.failed.push((e.path, err.to_string())),
        }
    }
    let mut n = 0;
    while dir.join(format!("manifest.undone-{n}.jsonl")).exists() {
        n += 1;
    }
    fs::rename(&manifest, dir.join(format!("manifest.undone-{n}.jsonl")))?;
    Ok(report)
}

/// A session journal on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionJournal {
    pub session: String,
    pub dir: PathBuf,
    /// Paths recorded and not yet undone.
    pub paths: usize,
    pub modified: Option<std::time::SystemTime>,
}

/// Journals under `base` with pending changes, newest first.
pub fn list(base: &Path) -> Vec<SessionJournal> {
    let mut out: Vec<SessionJournal> = fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let manifest = dir.join("manifest.jsonl");
            let meta = fs::metadata(&manifest).ok()?;
            Some(SessionJournal {
                session: e.file_name().to_string_lossy().into_owned(),
                paths: read_entries(&dir).len(),
                dir,
                modified: meta.modified().ok(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_edits_removes_new_files_and_dirs_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "orijinal").unwrap();
        let base = tmp.path().join("undo");
        let mut j = Journal::open(&base, "s-1", std::slice::from_ref(&root)).unwrap();

        j.record(&root, "a.txt").unwrap();
        j.record(&root, "yeni/alt/b.txt").unwrap();
        j.record(&root, "../disarida.txt").unwrap(); // outside the project: ignored
        fs::write(root.join("a.txt"), "değişti").unwrap();
        j.record(&root, "./a.txt").unwrap(); // same path: first state kept
        fs::create_dir_all(root.join("yeni/alt")).unwrap();
        fs::write(root.join("yeni/alt/b.txt"), "x").unwrap();

        // A reopened journal continues (desktop and CLI share it).
        let j2 = Journal::open(&base, "s-1", std::slice::from_ref(&root)).unwrap();
        assert!(Journal::open(&base, "bos", std::slice::from_ref(&root)).is_ok());
        assert!(
            !base.join("bos").exists(),
            "no folder until something is recorded"
        );
        assert_eq!(j2.seen.len(), 2);

        assert_eq!(list(&base).len(), 1);
        let r = undo(j.dir()).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "orijinal");
        assert!(!root.join("yeni").exists(), "created dirs removed");
        assert_eq!(
            (r.restored.len(), r.deleted.len(), r.failed.len()),
            (1, 1, 0)
        );
        assert!(undo(j.dir()).is_err(), "second undo has nothing to do");
        assert!(list(&base).is_empty());
        assert!(Journal::open(&base, "../x", &[root]).is_err());
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn shell_changes_are_undone_too() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("src/app.js"), "v1").unwrap();
        fs::write(root.join("old.txt"), "keep me").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        fs::write(root.join("notes.txt"), "untracked").unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let base = tmp.path().join("undo");
        let mut j = Journal::open(&base, "s-2", std::slice::from_ref(&root)).unwrap();

        let before = scan_before(&root);
        assert!(before.complete);
        // What `npm install && sed -i … && rm old.txt` might do.
        fs::create_dir_all(root.join("node_modules/left-pad")).unwrap();
        fs::write(root.join("node_modules/left-pad/index.js"), "x").unwrap();
        fs::write(root.join("src/app.js"), "v2-changed").unwrap();
        fs::remove_file(root.join("old.txt")).unwrap();
        fs::write(root.join("src/new.js"), "new").unwrap();
        fs::write(root.join("notes.txt"), "untracked, changed").unwrap();
        let ch = j.record_shell(&before).unwrap();
        let rel = |v: &[PathBuf]| -> Vec<String> {
            let mut r: Vec<String> = v.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
            r.sort();
            r
        };
        assert_eq!(rel(&ch.created), ["node_modules", "src/new.js"]);
        assert_eq!(rel(&ch.changed), ["old.txt", "src/app.js"]);
        assert_eq!(rel(&ch.unrestorable), ["notes.txt"]);

        let r = undo(j.dir()).unwrap();
        assert!(!root.join("node_modules").exists() && !root.join("src/new.js").exists());
        assert_eq!(fs::read_to_string(root.join("src/app.js")).unwrap(), "v1");
        assert_eq!(fs::read_to_string(root.join("old.txt")).unwrap(), "keep me");
        assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), "untracked, changed");
        assert_eq!(r.skipped.len(), 1);
    }
}
