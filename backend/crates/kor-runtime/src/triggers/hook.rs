//! Local git hooks for `git_push` triggers (`kor triggers hook install`, only on that explicit
//! command). A bare repository (a local "origin" or a mirror) gets `post-receive`; a working
//! repository gets `reference-transaction`, which sees the remote-tracking refs a `git push`
//! updates. The hook only drops a small event file into the daemon's inbox, so pushes made while
//! the daemon is stopped are handled when it starts; it never blocks or fails the push.

use std::path::{Path, PathBuf};

use super::webhook::{Push, branch_of};

pub const MARK: &str = "# kor-trigger-hook";
const EXT: &str = "push";

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    use kor_platform::NoWindow as _;
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .no_window()
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// The repository's git directory, canonical (`<repo>/.git`, or the bare repository itself).
pub fn git_dir(repo: &Path) -> Option<PathBuf> {
    let d = git(repo, &["rev-parse", "--absolute-git-dir"]).ok()?;
    super::canon(Path::new(&d))
}

/// The hook text for `bare` (post-receive) or a working repository (reference-transaction).
pub fn script(bare: bool, inbox: &Path) -> Result<String, String> {
    let inbox = inbox.display().to_string().replace('\\', "/");
    if inbox.contains('\'') {
        return Err("the inbox path contains a quote".into());
    }
    let (guard, filter) = if bare {
        ("", "")
    } else {
        (
            "[ \"$1\" = committed ] || exit 0\n",
            "  case \"$ref\" in refs/remotes/*) ;; *) continue ;; esac\n",
        )
    };
    Ok(format!(
        "#!/bin/sh\n{MARK} (kor triggers hook install)\n{guard}inbox='{inbox}'\n\
         dir=$(git rev-parse --absolute-git-dir 2>/dev/null) || exit 0\nmkdir -p \"$inbox\" 2>/dev/null\nn=0\n\
         while read old new ref; do\n{filter}  n=$((n+1))\n  f=\"$inbox/$(date +%s)-$$-$n.{EXT}\"\n  \
         printf '%s\\n%s\\n%s\\n%s\\n' \"$dir\" \"$ref\" \"$old\" \"$new\" > \"$f.tmp\" && mv \"$f.tmp\" \"$f\"\n\
         done\nexit 0\n"
    ))
}

fn hook_path(repo: &Path) -> Result<(PathBuf, bool), String> {
    let bare = git(repo, &["rev-parse", "--is-bare-repository"])? == "true";
    let name = if bare {
        "hooks/post-receive"
    } else {
        "hooks/reference-transaction"
    };
    let rel = git(repo, &["rev-parse", "--git-path", name])?;
    let p = PathBuf::from(&rel);
    Ok((if p.is_absolute() { p } else { repo.join(p) }, bare))
}

/// Install the hook in `repo`; a hook that is not Kor's is never overwritten.
///
/// # Errors
/// Not a git repository, a foreign hook in the way, or an I/O failure.
pub fn install(repo: &Path, inbox: &Path) -> Result<PathBuf, String> {
    let (path, bare) = hook_path(repo)?;
    if let Ok(old) = std::fs::read_to_string(&path)
        && !old.contains(MARK)
    {
        return Err(format!(
            "{} exists and is not Kor's; add Kor's lines to it by hand",
            path.display()
        ));
    }
    let io = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(io)?;
    }
    std::fs::create_dir_all(inbox).map_err(io)?;
    std::fs::write(&path, script(bare, inbox)?).map_err(io)?;
    kor_platform::make_executable(&path).map_err(io)?;
    Ok(path)
}

/// Remove Kor's hook from `repo`; `false` when there was none.
///
/// # Errors
/// Not a git repository or an I/O failure.
pub fn uninstall(repo: &Path) -> Result<bool, String> {
    let (path, _) = hook_path(repo)?;
    match std::fs::read_to_string(&path) {
        Ok(t) if t.contains(MARK) => std::fs::remove_file(&path)
            .map(|()| true)
            .map_err(|e| e.to_string()),
        _ => Ok(false),
    }
}

/// Whether `repo` has Kor's hook.
pub fn installed(repo: &Path) -> bool {
    hook_path(repo).is_ok_and(|(p, _)| std::fs::read_to_string(p).is_ok_and(|t| t.contains(MARK)))
}

/// A push from the hook's event file: git dir, ref, old, new.
pub fn parse_event(text: &str) -> Option<Push> {
    let mut l = text.lines();
    let (dir, r#ref, before, after) = (l.next()?, l.next()?, l.next()?, l.next()?);
    if after.bytes().all(|b| b == b'0') {
        return None; // deleted
    }
    Some(Push {
        repos: vec![dir.trim().to_owned()],
        branch: branch_of(r#ref.trim())?,
        before: before.trim().to_owned(),
        after: after.trim().to_owned(),
    })
}

/// Take every complete event file out of `inbox` (oldest first), deleting it.
pub fn drain(inbox: &Path) -> Vec<Push> {
    let Ok(rd) = std::fs::read_dir(inbox) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXT))
        .collect();
    files.sort();
    files
        .into_iter()
        .take(1000)
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok();
            let _ = std::fs::remove_file(&p);
            text.as_deref().and_then(parse_event)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_parse_and_skip_deletes() {
        let p = parse_event("/r/.git\nrefs/remotes/origin/main\naaa\nbbb\n").unwrap();
        assert_eq!((p.branch.as_str(), p.after.as_str()), ("main", "bbb"));
        assert!(
            parse_event("/r\nrefs/heads/x\naaa\n0000000000000000000000000000000000000000\n")
                .is_none()
        );
        assert!(parse_event("/r\nrefs/tags/v1\naaa\nbbb\n").is_none());
        let s = script(false, Path::new("C:\\k\\inbox")).unwrap();
        assert!(s.contains("inbox='C:/k/inbox'") && s.contains("committed"));
        assert!(script(true, Path::new("/it's")).is_err());
    }
}
