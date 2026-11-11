//! The TUI's "changes" panel: files this session changed, read from its undo journal
//! (`kor_tools::undo`, the same one `kor undo` and the desktop use). A diff against the recorded
//! original, and a per-file revert that runs only when the user presses the key.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use kor_tools::undo::Entry;

/// Larger files are not diffed (the line diff is quadratic).
const MAX_DIFF_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    /// The original bytes; `None` when the file did not exist before.
    pub blob: Option<PathBuf>,
    /// Too large or a directory: only `kor undo` reports it, nothing to revert here.
    pub skipped: bool,
}

/// The journal of the session whose transcript is `session_file`.
pub fn journal_dir(session_file: &Path) -> Option<PathBuf> {
    let id = session_file.file_stem()?.to_str()?;
    Some(kor_runtime::undo_dir()?.join(id))
}

/// Changed files recorded in the journal at `dir` (none after `kor undo`).
pub fn load(dir: &Path) -> Vec<Change> {
    fs::read_to_string(dir.join("manifest.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .map(|e| Change {
            blob: e.blob.map(|n| dir.join("blobs").join(n.to_string())),
            skipped: e.skipped,
            path: e.path,
        })
        .collect()
}

fn read_text(p: &Path) -> Result<String, &'static str> {
    let Ok(f) = fs::File::open(p) else {
        return Ok(String::new());
    };
    if f.metadata().map(|m| m.len()).unwrap_or(0) > MAX_DIFF_BYTES {
        return Err("large");
    }
    let mut bytes = Vec::new();
    f.take(MAX_DIFF_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| "unreadable")?;
    String::from_utf8(bytes).map_err(|_| "binary")
}

/// Diff lines, original → current; `Err` with a reason key (`large`, `binary`, `unreadable`).
pub fn diff(c: &Change) -> Result<Vec<String>, &'static str> {
    if c.skipped {
        return Err("large");
    }
    let old = match &c.blob {
        Some(b) => read_text(b)?,
        None => String::new(),
    };
    let new = read_text(&c.path)?;
    let name = kor_tools::display_path(&c.path);
    Ok(kor_agent::port::unified_diff(&name, &old, &new)
        .lines()
        .map(str::to_owned)
        .collect())
}

/// Put one file back to its recorded original (delete it if it was new).
///
/// # Errors
/// If the change was not restorable or the file system refuses.
pub fn revert(c: &Change) -> std::io::Result<()> {
    if c.skipped {
        return Err(std::io::Error::other("not recorded"));
    }
    match &c.blob {
        Some(b) => {
            if let Some(parent) = c.path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(b, &c.path).map(|_| ())
        }
        None => match fs::symlink_metadata(&c.path) {
            Ok(m) if m.is_dir() => Err(std::io::Error::other("directory")),
            Ok(_) => fs::remove_file(&c.path),
            Err(_) => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_and_revert_one_file_from_the_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "eski\n").unwrap();
        let base = tmp.path().join("undo");
        let mut j =
            kor_tools::undo::Journal::open(&base, "s1", std::slice::from_ref(&root)).unwrap();
        j.record(&root, "a.txt").unwrap();
        j.record(&root, "b.txt").unwrap();
        fs::write(root.join("a.txt"), "yeni\n").unwrap();
        fs::write(root.join("b.txt"), "b\n").unwrap();

        let changes = load(&base.join("s1"));
        assert_eq!(changes.len(), 2);
        let d = diff(&changes[0]).unwrap().join("\n");
        assert!(d.contains("- eski") && d.contains("+ yeni"), "{d}");

        revert(&changes[0]).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "eski\n");
        revert(&changes[1]).unwrap();
        assert!(!root.join("b.txt").exists());
        // Only the chosen files changed; the journal stays for `kor undo`.
        assert_eq!(load(&base.join("s1")).len(), 2);
        assert!(load(&tmp.path().join("none")).is_empty());
    }
}
