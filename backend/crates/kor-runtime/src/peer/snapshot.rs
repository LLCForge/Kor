//! Project snapshots for delegated tasks (ADR 0057): the files of the project (what `.gitignore`,
//! `.korignore` and `.ignore` keep, never `.git/`), sent to the peer, unpacked into the task's own
//! workspace; afterwards the changed files and a unified diff come back and are applied locally
//! only where the local file still matches what was sent.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Largest single file in a snapshot.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;
/// Largest snapshot (the peer frame limit is 96 MiB; base64 adds a third).
pub const MAX_TOTAL: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    /// Project-relative, `/`-separated.
    pub path: String,
    /// Base64 of the bytes.
    pub data: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub files: Vec<File>,
    /// Files left out (too big, unreadable), so the user knows.
    #[serde(default)]
    pub skipped: Vec<String>,
}

/// One file the task changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    /// Base64; `None` = the task created the file.
    pub before: Option<String>,
    /// Base64; `None` = the task deleted the file.
    pub after: Option<String>,
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unb64(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

/// `a/b.txt` relative to `root`, refusing anything that could leave it.
///
/// # Errors
/// On an absolute path, `..`, a drive prefix or an empty path.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    if rel.is_empty() || rel.contains('\0') || rel.contains('\\') {
        return Err(format!("invalid path in snapshot: {rel}"));
    }
    let mut out = root.to_path_buf();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s),
            _ => return Err(format!("invalid path in snapshot: {rel}")),
        }
    }
    Ok(out)
}

fn rel_of(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    let parts: Vec<String> = r
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The project's files under `root`, honoring ignore files.
///
/// # Errors
/// When the project is larger than [`MAX_TOTAL`] or [`MAX_FILES`].
pub fn collect(root: &Path) -> Result<Bundle, String> {
    let mut b = Bundle::default();
    let mut total = 0u64;
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .add_custom_ignore_filename(".korignore")
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for e in walk.flatten() {
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel) = rel_of(root, e.path()) else {
            continue;
        };
        let size = e.metadata().map_or(u64::MAX, |m| m.len());
        if size > MAX_FILE {
            b.skipped.push(rel);
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else {
            b.skipped.push(rel);
            continue;
        };
        total += bytes.len() as u64;
        if total > MAX_TOTAL || b.files.len() >= MAX_FILES {
            return Err(format!(
                "the project is too large to send ({} MiB / {MAX_FILES} files at most); run the task in a directory on the peer instead",
                MAX_TOTAL / 1024 / 1024
            ));
        }
        b.files.push(File {
            path: rel,
            data: b64(&bytes),
        });
    }
    b.files.sort_by(|a, c| a.path.cmp(&c.path));
    Ok(b)
}

/// SHA-256 over paths and contents: "did the project change since the last standby push".
pub fn digest(b: &Bundle) -> String {
    let mut h = Sha256::new();
    for f in &b.files {
        h.update(f.path.as_bytes());
        h.update([0]);
        h.update(f.data.as_bytes());
        h.update([0]);
    }
    super::hex(&h.finalize())
}

/// Unpack into `dir` (created; must be empty or new).
///
/// # Errors
/// On an unsafe path, bad base64 or I/O errors.
pub fn extract(b: &Bundle, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for f in &b.files {
        let p = safe_join(dir, &f.path)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, unb64(&f.data)?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What the task changed in `dir` compared with the bundle it started from.
///
/// # Errors
/// When `dir` cannot be walked.
pub fn changes(b: &Bundle, dir: &Path) -> Result<Vec<Change>, String> {
    let now = collect(dir)?;
    let before: BTreeMap<&str, &str> = b
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.data.as_str()))
        .collect();
    let after: BTreeMap<&str, &str> = now
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.data.as_str()))
        .collect();
    let mut out = Vec::new();
    for (p, a) in &after {
        match before.get(p) {
            Some(x) if x == a => {}
            x => out.push(Change {
                path: (*p).to_owned(),
                before: x.map(|s| (*s).to_owned()),
                after: Some((*a).to_owned()),
            }),
        }
    }
    for (p, x) in &before {
        if !after.contains_key(p) {
            out.push(Change {
                path: (*p).to_owned(),
                before: Some((*x).to_owned()),
                after: None,
            });
        }
    }
    out.sort_by(|a, c| a.path.cmp(&c.path));
    Ok(out)
}

/// Unified diff of every change (binary files as one line).
pub fn diff(changes: &[Change]) -> String {
    let text = |s: &Option<String>| -> Result<Option<String>, ()> {
        match s {
            None => Ok(None),
            Some(d) => {
                let b = unb64(d).map_err(|_| ())?;
                String::from_utf8(b).map(Some).map_err(|_| ())
            }
        }
    };
    let mut out = String::new();
    for c in changes {
        if let (Ok(a), Ok(b)) = (text(&c.before), text(&c.after)) {
            out.push_str(&crate::textdiff::unified(
                &c.path,
                a.as_deref(),
                b.as_deref(),
            ));
        } else {
            out.push_str("Binary file ");
            out.push_str(&c.path);
            out.push_str(" changed\n");
        }
    }
    out
}

/// How applying one change went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub path: String,
    /// `written`, `deleted`, `conflict` (the local file changed since the snapshot), `failed`.
    pub status: String,
    pub detail: Option<String>,
}

/// Apply `changes` under `root` where the local file still equals the snapshot's version;
/// `only` limits it to those paths (per-file review).
pub fn apply(root: &Path, changes: &[Change], only: Option<&[String]>) -> Vec<Applied> {
    let mut out = Vec::new();
    for c in changes {
        if only.is_some_and(|o| !o.iter().any(|p| p == &c.path)) {
            continue;
        }
        let mut res = |status: &str, detail: Option<String>| {
            out.push(Applied {
                path: c.path.clone(),
                status: status.into(),
                detail,
            });
        };
        let p = match safe_join(root, &c.path) {
            Ok(p) => p,
            Err(e) => {
                res("failed", Some(e));
                continue;
            }
        };
        let local = std::fs::read(&p).ok();
        let expected = match c.before.as_deref().map(unb64).transpose() {
            Ok(x) => x,
            Err(e) => {
                res("failed", Some(e));
                continue;
            }
        };
        if local != expected {
            res("conflict", None);
            continue;
        }
        match &c.after {
            None => match std::fs::remove_file(&p) {
                Ok(()) => res("deleted", None),
                Err(e) => res("failed", Some(e.to_string())),
            },
            Some(d) => {
                let r = unb64(d).and_then(|bytes| {
                    if let Some(parent) = p.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&p, bytes).map_err(|e| e.to_string())
                });
                match r {
                    Ok(()) => res("written", None),
                    Err(e) => res("failed", Some(e)),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trip_changes_diff_and_safe_apply() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("src")).unwrap();
        std::fs::create_dir_all(src.path().join(".git")).unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "x").unwrap();
        std::fs::write(src.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(src.path().join("target")).unwrap();
        std::fs::write(src.path().join("target/big.bin"), "ignored").unwrap();
        std::fs::write(src.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(src.path().join("gone.txt"), "bye\n").unwrap();
        let b = collect(src.path()).unwrap();
        let paths: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, [".gitignore", "gone.txt", "src/a.rs"]);

        let work = tempfile::tempdir().unwrap();
        extract(&b, work.path()).unwrap();
        std::fs::write(work.path().join("src/a.rs"), "fn a() { 1; }\n").unwrap();
        std::fs::remove_file(work.path().join("gone.txt")).unwrap();
        std::fs::write(work.path().join("new.md"), "hi\n").unwrap();
        let ch = changes(&b, work.path()).unwrap();
        let names: Vec<&str> = ch.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(names, ["gone.txt", "new.md", "src/a.rs"]);
        let d = diff(&ch);
        assert!(d.contains("+fn a() { 1; }"), "{d}");

        // The local copy of gone.txt changed meanwhile: that one is a conflict, not a delete.
        std::fs::write(src.path().join("gone.txt"), "edited locally\n").unwrap();
        let res = apply(src.path(), &ch, None);
        let st: Vec<(&str, &str)> = res
            .iter()
            .map(|a| (a.path.as_str(), a.status.as_str()))
            .collect();
        assert_eq!(
            st,
            [
                ("gone.txt", "conflict"),
                ("new.md", "written"),
                ("src/a.rs", "written")
            ]
        );
        assert_eq!(
            std::fs::read_to_string(src.path().join("src/a.rs")).unwrap(),
            "fn a() { 1; }\n"
        );
        assert!(safe_join(src.path(), "../x").is_err());
        assert!(safe_join(src.path(), "/etc/passwd").is_err());
        assert!(safe_join(src.path(), "a\\..\\b").is_err());
        assert_eq!(digest(&b), digest(&b.clone()));
    }
}
