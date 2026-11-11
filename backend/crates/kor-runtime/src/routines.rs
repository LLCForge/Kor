//! Scheduled tasks ("routines"): a prompt that runs every N minutes in a folder.
//!
//! One store for `kor routines` and the desktop (`<kor home>/triggers.json`, plain JSON). The
//! desktop app runs due routines while it is open; the CLI manages them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

pub const MIN_INTERVAL: u64 = 1;
pub const MAX_INTERVAL: u64 = 7 * 24 * 60;
pub const MAX_ROUTINES: usize = 100;

/// Serializes read-modify-write cycles inside one process.
static LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Deserialize, serde::Serialize, Default)]
struct File {
    #[serde(default)]
    triggers: Vec<Value>,
}

/// `<kor home>/triggers.json`.
pub fn file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("triggers.json"))
}

fn read(path: &Path) -> File {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write(path: &Path, f: &File) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("kor-tmp");
    let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn with<T>(
    path: &Path,
    f: impl FnOnce(&mut File) -> Result<(T, bool), String>,
) -> Result<T, String> {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut file = read(path);
    let (out, changed) = f(&mut file)?;
    if changed {
        write(path, &file)?;
    }
    Ok(out)
}

pub fn list(path: &Path) -> Vec<Value> {
    read(path).triggers
}

/// Add a routine from `{name?, prompt, intervalMinutes, root}`; returns the stored entry.
///
/// # Errors
/// On an invalid prompt, folder or interval, or when the limit is reached.
pub fn add(path: &Path, input: &Map<String, Value>, now: u64) -> Result<Value, String> {
    let prompt = input.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() || prompt.chars().count() > 10_000 {
        return Err("İstem 1-10000 karakter olmalı".into());
    }
    let root = input.get("root").and_then(Value::as_str).unwrap_or("");
    if root.len() > 4096 {
        return Err("Klasör yolu çok uzun".into());
    }
    let minutes = input
        .get("intervalMinutes")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or("Aralık sayı olmalı")?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped right after
    let interval = (minutes.round().max(0.0) as u64).clamp(MIN_INTERVAL, MAX_INTERVAL);
    let name: String = input
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(prompt)
        .chars()
        .take(120)
        .collect();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = json!({
        "id": format!("{now:x}-{seq:x}"),
        "name": name,
        "prompt": prompt,
        "intervalMinutes": interval,
        "root": root,
        "enabled": true,
        "createdAt": now,
        "lastRun": null,
        // `firstRun` (unix ms, §30 "her pazartesi"): a later first run aligned to a weekday/time.
        "nextRun": input
            .get("firstRun")
            .and_then(Value::as_u64)
            .filter(|t| *t > now)
            .unwrap_or(now + interval * 60_000),
    });
    with(path, |f| {
        if f.triggers.len() >= MAX_ROUTINES {
            return Err(format!("En fazla {MAX_ROUTINES} görev eklenebilir"));
        }
        f.triggers.push(t.clone());
        Ok((t, true))
    })
}

/// `true` when a routine was removed.
///
/// # Errors
/// If the file cannot be written.
pub fn remove(path: &Path, id: &str) -> Result<bool, String> {
    with(path, |f| {
        let before = f.triggers.len();
        f.triggers.retain(|t| t["id"] != id);
        let changed = before != f.triggers.len();
        Ok((changed, changed))
    })
}

/// Pause or resume; resuming schedules the next run one interval from `now`.
///
/// # Errors
/// If the file cannot be written.
pub fn set_enabled(path: &Path, id: &str, enabled: bool, now: u64) -> Result<bool, String> {
    with(path, |f| {
        let Some(t) = f.triggers.iter_mut().find(|t| t["id"] == id) else {
            return Ok((false, false));
        };
        t["enabled"] = json!(enabled);
        if enabled {
            let interval = t["intervalMinutes"].as_u64().unwrap_or(MIN_INTERVAL);
            t["nextRun"] = json!(now + interval * 60_000);
        }
        Ok((true, true))
    })
}

/// Mark due routines as run and return them (the next run counts from `now`, so runs missed
/// while nothing was open happen once).
pub fn take_due(path: &Path, now: u64) -> Vec<Value> {
    take_due_where(path, now, |_| true)
}

/// [`take_due`] for the routines `pick` accepts (`kor daemon` runs those with a folder, the
/// desktop app the rest while the daemon runs).
pub fn take_due_where(path: &Path, now: u64, pick: impl Fn(&Value) -> bool) -> Vec<Value> {
    with(path, |f| {
        let mut due = Vec::new();
        for t in f.triggers.iter_mut().filter(|t| pick(t)) {
            due.extend(mark_due(std::slice::from_mut(t), now));
        }
        let changed = !due.is_empty();
        Ok((due, changed))
    })
    .unwrap_or_default()
}

/// The run after `now`: by the cron `schedule` when the entry has one (`cron` triggers of
/// `kor daemon`), else `intervalMinutes` later.
pub fn next_run(t: &Value, now: u64) -> Option<u64> {
    match t["schedule"].as_str() {
        Some(s) => crate::triggers::cron::Schedule::parse(s).ok()?.next_ms(now),
        None => Some(now + t["intervalMinutes"].as_u64().unwrap_or(MIN_INTERVAL) * 60_000),
    }
}

pub fn mark_due(triggers: &mut [Value], now: u64) -> Vec<Value> {
    let mut due = Vec::new();
    for t in triggers {
        if t["enabled"] == true && t["nextRun"].as_u64().is_some_and(|n| n <= now) {
            t["lastRun"] = json!(now);
            t["nextRun"] = json!(next_run(t, now));
            due.push(t.clone());
        }
    }
    due
}

/// §27.15 (ADR 0063): the weekly knowledge digest (`<kor home>/knowledge/digest/<week>.md`), written
/// once per ISO week by whichever scheduler runs (`kor daemon` or the desktop app). Local only.
pub fn knowledge_digest_tick() -> Option<PathBuf> {
    let home = crate::kor_home()?;
    let now = chrono::Utc::now().timestamp();
    kor_memory::digest::due(&home, now)?;
    let k = kor_memory::open_knowledge(&home).ok()?;
    kor_memory::digest::write_due(&k, &home, now).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_pause_due_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("triggers.json");
        let input = json!({"prompt": "testleri çalıştır", "intervalMinutes": 5, "root": ""});
        let t = add(&p, input.as_object().unwrap(), 1_000).unwrap();
        let id = t["id"].as_str().unwrap().to_owned();
        assert_eq!(t["name"], "testleri çalıştır");
        assert!(
            add(
                &p,
                json!({"prompt": " ", "intervalMinutes": 5})
                    .as_object()
                    .unwrap(),
                0
            )
            .is_err()
        );
        assert!(take_due(&p, 1_000).is_empty(), "not due yet");
        let due = take_due(&p, 1_000 + 5 * 60_000);
        assert_eq!(due.len(), 1);
        assert!(take_due(&p, 1_000 + 5 * 60_000).is_empty(), "not twice");
        assert!(set_enabled(&p, &id, false, 0).unwrap());
        assert!(take_due(&p, u64::MAX / 2).is_empty(), "paused");
        assert!(remove(&p, &id).unwrap());
        assert!(list(&p).is_empty());
    }
}
