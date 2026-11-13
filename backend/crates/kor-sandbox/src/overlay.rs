//! Overlay file system (`[sandbox] overlay = true`, §6.4): a confined command sees the project as
//! it is, but what it writes there lands in a throwaway layer, so the real project never changes.
//! Each command starts from the project as it is now; its writes are gone when it ends.
//!
//! - Linux with bubblewrap 0.8 or later: overlayfs inside the sandbox (`--overlay-src <root>
//!   --tmp-overlay <root>`), the upper layer on a private tmpfs. Nothing is copied.
//! - Elsewhere (macOS, Windows, Docker, older bubblewrap): a copy of the project in a temp folder,
//!   made with the OS's copy-on-write cloning where the file system has it (`std::fs::copy` uses
//!   `clonefile` on APFS, `copy_file_range` reflinks on Btrfs/XFS, block cloning on `ReFS` / Dev
//!   Drive). On other file systems this is a full copy, so large projects take time and disk.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// bubblewrap 0.8 added `--overlay-src` / `--tmp-overlay`.
pub(crate) fn bwrap_has_overlay() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| {
        std::process::Command::new("bwrap")
            .arg("--version")
            .output()
            .ok()
            .is_some_and(|o| version_has_overlay(&String::from_utf8_lossy(&o.stdout)))
    })
}

/// `bubblewrap 0.8.0` → true.
fn version_has_overlay(text: &str) -> bool {
    let v = text.split_whitespace().last().unwrap_or_default();
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    major > 0 || minor >= 8
}

/// A temp copy of the project; removed when dropped.
#[derive(Debug)]
pub(crate) struct Scratch {
    pub(crate) root: PathBuf,
    dir: PathBuf,
}

impl Scratch {
    pub(crate) fn copy_of(root: &Path) -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "kor-overlay-{}-{}",
            std::process::id(),
            crate::docker::next_id()
        ));
        let name = root.file_name().unwrap_or_else(|| "project".as_ref());
        let root_copy = dir.join(name);
        std::fs::create_dir_all(&root_copy)?;
        // Built before copying so a failed copy is cleaned up by `Drop`.
        let mut s = Self {
            root: root_copy,
            dir,
        };
        copy_tree(root, &s.root)?;
        // Canonical like the caller's root (`\\?\` on Windows), so path checks keep working.
        if let Ok(c) = s.root.canonicalize() {
            s.root = c;
        }
        Ok(s)
    }

    /// `path` under the real root → the same place in the copy.
    pub(crate) fn map(&self, real_root: &Path, path: &Path) -> PathBuf {
        path.strip_prefix(real_root)
            .map_or_else(|_| self.root.clone(), |rel| self.root.join(rel))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let ty = entry.file_type()?;
        if ty.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_tree(&src, &dst)?;
        } else if ty.is_symlink() {
            kor_platform::copy_link(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bwrap_versions() {
        assert!(version_has_overlay("bubblewrap 0.8.0\n"));
        assert!(version_has_overlay("bubblewrap 0.11.0"));
        assert!(version_has_overlay("bubblewrap 1.0"));
        assert!(!version_has_overlay("bubblewrap 0.6.2"));
        assert!(!version_has_overlay(""));
    }

    #[test]
    fn copy_is_complete_separate_and_removed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("proj");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join("src/deep/b.txt"), "b").unwrap();
        let s = Scratch::copy_of(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(s.root.join("src/deep/b.txt")).unwrap(),
            "b"
        );
        std::fs::write(s.root.join("a.txt"), "changed").unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "a");
        assert_eq!(s.map(&root, &root.join("src")), s.root.join("src"));
        assert_eq!(s.map(&root, Path::new("/elsewhere")), s.root);
        let dir = s.root.parent().unwrap().to_path_buf();
        drop(s);
        assert!(!dir.exists());
    }
}
