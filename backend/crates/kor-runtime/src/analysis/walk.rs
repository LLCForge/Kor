//! Project text files for the analyzers: `.gitignore` respected, dotfiles (`.env`) included,
//! vendored/build directories and binaries skipped, sizes capped.

use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 20_000;
pub const MAX_BYTES: u64 = 1024 * 1024;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".kor",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
];

/// One readable text file.
#[derive(Debug, Clone)]
pub struct Source {
    /// Project-relative.
    pub rel: PathBuf,
    pub text: String,
}

impl Source {
    pub fn ext(&self) -> String {
        self.rel
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }

    pub fn name(&self) -> String {
        self.rel
            .file_name()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Text files under `root`; the flag is `true` when [`MAX_FILES`] cut the list short.
pub fn sources(root: &Path) -> (Vec<Source>, bool) {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .sort_by_file_path(Ord::cmp)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file())
            || entry.metadata().map_or(true, |m| m.len() > MAX_BYTES)
        {
            continue;
        }
        if out.len() >= MAX_FILES {
            return (out, true);
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if bytes.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        out.push(Source {
            rel: rel.to_path_buf(),
            text: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    (out, false)
}

/// 1-based line of byte offset `at`.
pub fn line_of(text: &str, at: usize) -> u32 {
    let at = at.min(text.len());
    // Pieces split at '\n' = newlines before `at` + 1.
    u32::try_from(text.as_bytes()[..at].split(|b| *b == b'\n').count()).unwrap_or(u32::MAX)
}
