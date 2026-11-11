//! Optional git-native memory (§6.1): `decisions.md` as a git note (`refs/notes/kor`) on the
//! repository's root commit, so it travels with the repo once the user pushes that ref.
//! Local git only; syncing the ref with a remote is the user's own `git push`/`git fetch`.

use std::path::Path;
use std::process::Command;

use crate::project::{Doc, ProjectMemory, parse_decisions};

pub const REF: &str = "kor";

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// The oldest root commit of `HEAD`: a stable object every clone has.
fn root_commit(repo: &Path) -> Result<String, String> {
    git(repo, &["rev-list", "--max-parents=0", "HEAD"])?
        .lines()
        .last()
        .map(str::to_owned)
        .filter(|c| c.chars().all(|ch| ch.is_ascii_hexdigit()) && !c.is_empty())
        .ok_or_else(|| "no commit in this repository".to_owned())
}

/// Store `decisions.md` as the `kor` note of the root commit. Returns the commit id.
///
/// # Errors
/// When there is no decisions file, no commit, or git fails.
pub fn push(repo: &Path, mem: &ProjectMemory) -> Result<String, String> {
    let file = mem.path(Doc::Decisions);
    if !file.is_file() {
        return Err(format!("{} does not exist", file.display()));
    }
    let commit = root_commit(repo)?;
    let file = file.to_string_lossy().into_owned();
    git(
        repo,
        &[
            "notes",
            &format!("--ref={REF}"),
            "add",
            "-f",
            "-F",
            &file,
            &commit,
        ],
    )?;
    Ok(commit)
}

/// Add the decisions of the `kor` note that `decisions.md` does not have yet (same date and
/// title). Returns how many were added.
///
/// # Errors
/// When there is no note or git fails.
pub fn pull(repo: &Path, mem: &ProjectMemory) -> Result<usize, String> {
    let commit = root_commit(repo)?;
    let note = git(repo, &["notes", &format!("--ref={REF}"), "show", &commit])?;
    let have = mem.decisions();
    let mut added = 0;
    for d in parse_decisions(&note) {
        if have.iter().any(|h| h.title == d.title && h.date == d.date) {
            continue;
        }
        mem.add_decision(&d).map_err(|e| e.to_string())?;
        added += 1;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Decision;

    #[test]
    fn push_then_pull_into_a_fresh_memory() {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        let ok = |args: &[&str]| git(r, args).is_ok();
        if !ok(&["init", "-q"]) {
            return; // no git on this machine
        }
        assert!(ok(&[
            "-c",
            "user.name=k",
            "-c",
            "user.email=k@k",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "root"
        ]));
        let home = tempfile::tempdir().unwrap();
        let a = ProjectMemory::new(home.path(), Path::new("/a"));
        a.ensure().unwrap();
        a.add_decision(&Decision {
            title: "Use WAL".into(),
            date: "2026-11-10".into(),
            refs: vec!["kb.rs".into()],
            ..Decision::default()
        })
        .unwrap();
        // `git notes add` needs an identity in some setups.
        let _ = git(r, &["config", "user.name", "k"]);
        let _ = git(r, &["config", "user.email", "k@k"]);
        push(r, &a).unwrap();
        let b = ProjectMemory::new(home.path(), Path::new("/b"));
        assert_eq!(pull(r, &b).unwrap(), 1);
        assert_eq!(pull(r, &b).unwrap(), 0);
        assert_eq!(b.decisions()[0].refs, ["kb.rs"]);
    }
}
