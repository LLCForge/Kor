//! On-disk model catalog cache (one JSON file, per-provider timestamps).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ModelInfo;

pub const TTL_SECS: u64 = 24 * 60 * 60;
const MAX_CACHE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub fetched_at: u64,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub providers: BTreeMap<String, Entry>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Catalog {
    /// A missing or corrupt cache is treated as empty; it is only a cache.
    pub fn load(path: &Path) -> Self {
        let mut text = String::new();
        let ok = fs::File::open(path)
            .and_then(|f| f.take(MAX_CACHE_BYTES).read_to_string(&mut text))
            .is_ok();
        if !ok {
            return Self::default();
        }
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Atomic write: temp file in the same directory, then rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        fs::rename(&tmp, path)
    }

    pub fn fresh(&self, provider: &str, now: u64) -> Option<&Entry> {
        self.providers
            .get(provider)
            .filter(|e| now.saturating_sub(e.fetched_at) < TTL_SECS)
    }

    pub fn put(&mut self, provider: &str, models: Vec<ModelInfo>, now: u64) {
        self.providers.insert(
            provider.to_owned(),
            Entry {
                fetched_at: now,
                models,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("models.json");
        assert_eq!(Catalog::load(&path), Catalog::default());

        let mut c = Catalog::default();
        let m = ModelInfo {
            id: "m".into(),
            display_name: None,
            context_window: Some(8),
            max_output: None,
        };
        c.put("p", vec![m], 1000);
        c.save(&path).unwrap();
        let back = Catalog::load(&path);
        assert_eq!(back, c);
        assert!(back.fresh("p", 1000 + TTL_SECS - 1).is_some());
        assert!(back.fresh("p", 1000 + TTL_SECS).is_none());

        fs::write(&path, "not json").unwrap();
        assert_eq!(Catalog::load(&path), Catalog::default());
    }
}
