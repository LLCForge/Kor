//! Local feature gates. Nothing is fetched over the network.
//!
//! Defaults match an external build: auto-compact on, background tasks off,
//! memory extraction off (`tengu_passport_quail` defaults false).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default = "yes")]
    pub auto_compact: bool,
    #[serde(default)]
    pub background_tasks: bool,
    /// `tengu_passport_quail`. Off unless the local file or `KOR_FLAG_MEMORY_EXTRACT` says so.
    #[serde(default)]
    pub memory_extract: bool,
}

fn yes() -> bool {
    true
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            auto_compact: true,
            background_tasks: false,
            memory_extract: false,
        }
    }
}

impl Flags {
    /// Read a JSON file, then apply `KOR_FLAG_AUTO_COMPACT`, `KOR_FLAG_BACKGROUND_TASKS`,
    /// and `KOR_FLAG_MEMORY_EXTRACT` (`1`/`0`).
    pub fn load(path: Option<&Path>) -> Self {
        let mut flags: Flags = path
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        apply_env("KOR_FLAG_AUTO_COMPACT", &mut flags.auto_compact);
        apply_env("KOR_FLAG_BACKGROUND_TASKS", &mut flags.background_tasks);
        apply_env("KOR_FLAG_MEMORY_EXTRACT", &mut flags.memory_extract);
        flags
    }
}

fn apply_env(name: &str, slot: &mut bool) {
    match std::env::var(name).ok().as_deref() {
        Some("1" | "true") => *slot = true,
        Some("0" | "false") => *slot = false,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_overrides_external_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flags.json");
        fs::write(
            &path,
            r#"{"auto_compact": false, "background_tasks": true, "memory_extract": true}"#,
        )
        .unwrap();
        let flags = Flags::load(Some(&path));
        assert!(!flags.auto_compact);
        assert!(flags.background_tasks);
        assert!(flags.memory_extract);
        assert!(Flags::load(None).auto_compact);
        assert!(!Flags::load(None).memory_extract);
    }
}
