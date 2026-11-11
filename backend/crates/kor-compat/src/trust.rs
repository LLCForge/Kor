//! Workspace trust. A trusted directory (and everything below it) may grant permissions,
//! run hooks and set environment variables from its own config files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    trusted: Vec<PathBuf>,
}

fn load(store: &Path) -> Store {
    std::fs::read(store)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// `\\?\C:\x` and `C:\x` name the same folder: callers pass either (canonicalised roots are
/// verbatim on Windows), so the store compares without the verbatim prefix.
fn plain(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p.to_owned()
    }
}

pub fn is_trusted(store: &Path, root: &Path) -> bool {
    let root = plain(root);
    load(store).trusted.iter().any(|t| root.starts_with(plain(t)))
}

/// Trust or untrust `root` (an exact entry). Returns whether the store changed.
pub fn set(store: &Path, root: &Path, trusted: bool) -> std::io::Result<bool> {
    let mut s = load(store);
    let root = &plain(root);
    let had = s.trusted.iter().any(|t| plain(t) == *root);
    if had == trusted {
        return Ok(false);
    }
    if trusted {
        s.trusted.push(root.to_owned());
    } else {
        s.trusted.retain(|t| plain(t) != *root);
    }
    if let Some(dir) = store.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = store.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&s).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, store)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_covers_subdirectories_and_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("kor").join("trusted.json");
        let proj = dir.path().join("proj");
        assert!(!is_trusted(&store, &proj));
        assert!(set(&store, &proj, true).unwrap());
        assert!(!set(&store, &proj, true).unwrap());
        assert!(is_trusted(&store, &proj.join("sub")));
        // The verbatim form of the same folder (what `canonicalize` returns on Windows) matches too.
        let verbatim = PathBuf::from(format!(r"\\?\{}", proj.display()));
        assert!(is_trusted(&store, &verbatim));
        assert!(!set(&store, &verbatim, true).unwrap(), "same folder, already trusted");
        assert!(!is_trusted(&store, dir.path()));
        assert!(set(&store, &proj, false).unwrap());
        assert!(!is_trusted(&store, &proj));
    }
}
