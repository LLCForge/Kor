//! Executables on `PATH` (§26.2 tool catalog, ADR 0056): Windows by `PATHEXT`, Unix by the
//! execute bit. The first directory that has a name wins, like a shell.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Lowercase extensions that make a file runnable here (`.exe`, `.cmd`, …); empty on Unix.
fn exts() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| e.starts_with('.') && e.len() > 1)
            .map(str::to_ascii_lowercase)
            // Scripts that need an interpreter's file association are not commands for the agent.
            .filter(|e| {
                !matches!(
                    e.as_str(),
                    ".vbs" | ".vbe" | ".js" | ".jse" | ".wsf" | ".wsh" | ".msc" | ".cpl"
                )
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(unix)]
fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(p: &Path) -> bool {
    p.is_file()
}

/// Command name of `file` in a PATH directory (`git.exe` → `git`), or `None` when not runnable.
fn command_name(file: &Path, exts: &[String]) -> Option<String> {
    let name = file.file_name()?.to_str()?;
    if exts.is_empty() {
        return runnable(file).then(|| name.to_owned());
    }
    let ext = file
        .extension()
        .and_then(OsStr::to_str)
        .map(|e| format!(".{}", e.to_ascii_lowercase()))?;
    if !exts.contains(&ext) || !file.is_file() {
        return None;
    }
    Some(name[..name.len() - ext.len()].to_ascii_lowercase())
}

/// Every command on `path` (the `PATH` value), first occurrence per name, sorted by name.
pub fn executables_on(path: &OsStr) -> BTreeMap<String, PathBuf> {
    let exts = exts();
    let mut out = BTreeMap::new();
    for dir in std::env::split_paths(path) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if let Some(n) = command_name(&p, &exts) {
                out.entry(n).or_insert(p);
            }
        }
    }
    out
}

/// Every command on this process's `PATH`.
pub fn executables() -> BTreeMap<String, PathBuf> {
    executables_on(&std::env::var_os("PATH").unwrap_or_default())
}

/// `name` on `PATH` (with `PATHEXT` on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let exts = exts();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if exts.is_empty() {
            let p = dir.join(name);
            if runnable(&p) {
                return Some(p);
            }
        }
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Text that changes when a PATH directory gains or loses a program (a new install): each
/// directory with its modification time.
pub fn path_fingerprint() -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|d| {
            let t = std::fs::metadata(&d)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{}={t}", d.display())
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_in_order_of_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let exts = exts();
        let file = |d: &Path, n: &str| {
            let name = if exts.is_empty() {
                n.to_owned()
            } else {
                format!("{n}.exe")
            };
            let p = d.join(name);
            std::fs::write(&p, "x").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        };
        let first = file(a.path(), "tool");
        file(b.path(), "tool");
        file(b.path(), "other");
        std::fs::write(a.path().join("notes.txt"), "x").unwrap();
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        let found = executables_on(&path);
        assert_eq!(found.get("tool"), Some(&first));
        assert!(found.contains_key("other"));
        assert!(!found.contains_key("notes"));
        assert!(!path_fingerprint().is_empty() || std::env::var_os("PATH").is_none());
    }
}
