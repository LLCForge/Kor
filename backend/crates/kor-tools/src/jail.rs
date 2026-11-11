//! Path confinement: every tool path must stay inside the project root, including
//! after following symlinks.

use std::path::{Component, Path, PathBuf};

/// Drop the Windows `\\?\` prefix so prefix checks and tool paths stay comparable.
pub fn without_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if let Some(unc) = rest.strip_prefix(r"UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        return PathBuf::from(rest);
    }
    p.to_path_buf()
}

/// Resolve `input` (relative to `root`, or absolute) to a path inside `root`.
///
/// `root` must be canonical. The returned path may not exist yet (for writes).
///
/// # Errors
/// If the path escapes `root` lexically (`..`) or through a symlink.
pub fn resolve(root: &Path, input: &str) -> Result<PathBuf, String> {
    let root = without_verbatim(root);
    if input.is_empty() || input.contains('\0') {
        return Err("empty or invalid path".into());
    }
    let joined = root.join(input);
    // Lexical normalization: drop `.`, fold `..`.
    let mut lexical = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if !lexical.pop() {
                    return Err(format!("`{input}` is outside the project"));
                }
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    // Canonicalize the deepest existing ancestor so symlinks cannot escape.
    let mut existing = lexical.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        rest.push(existing.file_name().ok_or("invalid path")?.to_owned());
        existing = existing.parent().ok_or("invalid path")?;
    }
    let mut real = without_verbatim(
        &existing
            .canonicalize()
            .map_err(|e| format!("`{input}`: {e}"))?,
    );
    for part in rest.iter().rev() {
        real.push(part);
    }
    if real.starts_with(root) {
        Ok(real)
    } else {
        Err(format!("`{input}` is outside the project"))
    }
}

/// `root`-relative display form with forward slashes.
pub(crate) fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let expect = |p: PathBuf| without_verbatim(&p);

        assert_eq!(
            resolve(&root, "src/a.rs").unwrap(),
            expect(root.join("src").join("a.rs"))
        );
        assert_eq!(
            resolve(&root, "./src/../b.rs").unwrap(),
            expect(root.join("b.rs"))
        );
        assert_eq!(
            resolve(&root, "new/dir/c.rs").unwrap(),
            expect(root.join("new").join("dir").join("c.rs"))
        );
        assert!(resolve(&root, root.join("src").to_str().unwrap()).is_ok());

        for bad in ["../x", "src/../../x", ""] {
            assert!(resolve(&root, bad).is_err(), "{bad}");
        }
        let outside = dir.path().parent().unwrap().join("elsewhere");
        assert!(resolve(&root, outside.to_str().unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
        assert!(resolve(&root, "link/secret").is_err());
    }
}
