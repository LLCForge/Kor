//! What the open desktop app is doing, for `kor status`, and the local channel through which
//! `kor approve|deny` answers its approval cards.
//!
//! The app rewrites `<presence>/desktop.json` on every change and at least every
//! [`HEARTBEAT_MS`]; a snapshot older than [`STALE_MS`] means the app is gone. An answer is a file
//! `<presence>/answers/<id>.json` the app consumes within a second; it applies only to an approval
//! that is pending in the app at that moment, and is only ever "yes" or "no" (never "always").
//! The folder is the user's own data folder: the same trust as the user's shell.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::eta::Eta;

pub const HEARTBEAT_MS: u64 = 5_000;
pub const STALE_MS: u64 = 15_000;
/// Answers older than this are dropped unread (the card they meant is long gone).
const ANSWER_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Desktop,
    Daemon,
    Remote,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Daemon => "daemon",
            Self::Remote => "remote",
        }
    }
}

/// One running desktop chat.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Running {
    pub id: String,
    pub title: String,
    pub root: String,
    /// Turn start, Unix ms.
    pub since: u64,
    /// `KorStatus` verb key (`thinking`, `reading`, `editing`, `running`, `waiting`, ...),
    /// derived from the last agent event.
    pub verb: String,
    /// Tool in progress.
    pub tool: Option<String>,
    pub cost_usd: f64,
    pub cost_estimated: bool,
    /// Todo progress `[done, total]` when the agent keeps a task list.
    pub progress: Option<[u32; 2]>,
    pub eta: Option<Eta>,
    pub waiting: bool,
}

/// One pending approval (any source).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Approval {
    /// Id to answer with (`kor approve <id>`).
    pub id: String,
    pub source: Source,
    /// Desktop chat, daemon job or remote task id.
    pub owner: String,
    /// Chat title, trigger name or task prompt.
    pub title: String,
    pub tool: String,
    pub summary: String,
    pub destructive: Option<String>,
    pub files: Vec<String>,
    pub since: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Snapshot {
    pub pid: u32,
    pub updated: u64,
    pub sessions: Vec<Running>,
    pub approvals: Vec<Approval>,
    /// Notifications paused from the tray.
    pub paused: bool,
}

pub fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join("desktop.json")
}

fn answers_dir(dir: &Path) -> PathBuf {
    dir.join("answers")
}

/// # Errors
/// When the file cannot be written.
pub fn write(dir: &Path, s: &Snapshot) -> Result<(), String> {
    crate::triggers::write_json(&snapshot_path(dir), s)
}

/// The desktop app is closing.
pub fn remove(dir: &Path) {
    let _ = std::fs::remove_file(snapshot_path(dir));
}

/// The snapshot when the app is running (written within [`STALE_MS`]).
pub fn read_live(dir: &Path, now: u64) -> Option<Snapshot> {
    let s: Snapshot = serde_json::from_slice(&std::fs::read(snapshot_path(dir)).ok()?).ok()?;
    (now.saturating_sub(s.updated) <= STALE_MS).then_some(s)
}

/// Approval ids: tool-call ids (`toolu_01…`, `call_…`), daemon hex ids, `task:id` for remote.
pub fn valid_approval_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Answer {
    approve: bool,
    at: u64,
}

fn answer_file(dir: &Path, id: &str) -> PathBuf {
    // `:` is not allowed in Windows file names.
    answers_dir(dir).join(format!("{}.json", id.replace(':', "~")))
}

/// Queue an answer for the desktop app.
///
/// # Errors
/// A bad id or an unwritable folder.
pub fn write_answer(dir: &Path, id: &str, approve: bool) -> Result<(), String> {
    if !valid_approval_id(id) {
        return Err("bad approval id".into());
    }
    crate::triggers::write_json(
        &answer_file(dir, id),
        &Answer {
            approve,
            at: super::now_ms(),
        },
    )
}

/// Whether the app has consumed (or dropped) the answer for `id`.
pub fn answer_pending(dir: &Path, id: &str) -> bool {
    answer_file(dir, id).is_file()
}

/// Withdraw an answer the app did not pick up.
pub fn cancel_answer(dir: &Path, id: &str) {
    if valid_approval_id(id) {
        let _ = std::fs::remove_file(answer_file(dir, id));
    }
}

/// Answers written since the last call (each file is removed): `(id, approve)`.
pub fn take_answers(dir: &Path) -> Vec<(String, bool)> {
    let now = super::now_ms();
    let Ok(rd) = std::fs::read_dir(answers_dir(dir)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
            .map(|n| n.replace('~', ":"))
        else {
            continue;
        };
        let answer: Option<Answer> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let _ = std::fs::remove_file(&path);
        if let Some(a) = answer
            && valid_approval_id(&id)
            && now.saturating_sub(a.at) <= ANSWER_TTL_MS
        {
            out.push((id, a.approve));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_live_only_while_fresh() {
        let d = tempfile::tempdir().unwrap();
        let s = Snapshot {
            pid: 7,
            updated: 1_000_000,
            ..Snapshot::default()
        };
        write(d.path(), &s).unwrap();
        assert_eq!(read_live(d.path(), 1_000_000 + STALE_MS).unwrap().pid, 7);
        assert!(read_live(d.path(), 1_000_000 + STALE_MS + 1).is_none());
        remove(d.path());
        assert!(read_live(d.path(), 1_000_000).is_none());
    }

    #[test]
    fn answers_are_taken_once_and_ids_are_checked() {
        let d = tempfile::tempdir().unwrap();
        write_answer(d.path(), "toolu_01AB", true).unwrap();
        write_answer(d.path(), "r-1-ab:call_2", false).unwrap();
        assert!(answer_pending(d.path(), "toolu_01AB"));
        assert!(write_answer(d.path(), "../x", true).is_err());
        assert!(write_answer(d.path(), ".hidden", true).is_err());
        assert!(write_answer(d.path(), "a/b", true).is_err());
        assert!(write_answer(d.path(), "", true).is_err());
        let got = take_answers(d.path());
        assert_eq!(
            got,
            vec![("r-1-ab:call_2".into(), false), ("toolu_01AB".into(), true)]
        );
        assert!(take_answers(d.path()).is_empty());
        assert!(!answer_pending(d.path(), "toolu_01AB"));
    }

    #[test]
    fn stale_answers_are_dropped() {
        let d = tempfile::tempdir().unwrap();
        crate::triggers::write_json(
            &answer_file(d.path(), "old"),
            &Answer {
                approve: true,
                at: 1,
            },
        )
        .unwrap();
        assert!(take_answers(d.path()).is_empty());
        assert!(!answer_pending(d.path(), "old"));
    }
}
