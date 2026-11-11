//! Desktop shell presence shared by the CLI and the desktop app (§27.1, 27.2, 27.8, 27.9, 28.4,
//! 28.5; ADR 0051): the notification store, the desktop's status snapshot and the local answer
//! channel for its approvals, the approval queue over every source (desktop, `kor daemon`, remote
//! tasks), turn-duration ETA, the "call me" alert policy and per-monitor window layouts.
//!
//! Everything lives in `<kor home>/presence/` — the user's own data folder, never the network.

pub mod attention;
pub mod desktop;
pub mod eta;
pub mod layout;
pub mod notifications;
pub mod overview;
pub mod queue;
pub mod tracker;

use std::path::PathBuf;

/// `<kor home>/presence`.
pub fn dir() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("presence"))
}

pub(crate) fn now_ms() -> u64 {
    crate::cost::now_ms()
}

/// Read-modify-write of one JSON file under an OS lock (desktop and CLI write the same files).
pub(crate) fn edit_json<T, R>(
    path: &std::path::Path,
    f: impl FnOnce(&mut T) -> R,
) -> Result<R, String>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Default,
{
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let _lock = kor_platform::lock_file(&path.with_extension("lock")).map_err(|e| e.to_string())?;
    let mut value: T = read_json(path);
    let out = f(&mut value);
    crate::triggers::write_json(path, &value)?;
    Ok(out)
}

/// A missing or damaged file reads as the default (never fatal for a status view).
pub(crate) fn read_json<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
