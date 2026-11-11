//! Local calendar and reminders: one JSON file in Kor's home (`assistant.json`), written
//! atomically. Times are stored as local wall-clock `YYYY-MM-DDTHH:MM:SS`, so ranges compare as
//! strings.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

pub const MAX_EVENTS: usize = 5000;
pub const MAX_REMINDERS: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub event_id: String,
    pub calendar_id: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_day: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Human-readable repetition ("every Monday"); not expanded into occurrences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct List {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub list_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Data {
    pub version: u32,
    pub next_id: u64,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub reminder_lists: Vec<List>,
    pub reminders: Vec<Reminder>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 2,
            calendars: vec![Calendar {
                id: "1".into(),
                name: "Personal".into(),
                is_primary: true,
            }],
            events: Vec::new(),
            reminder_lists: vec![List {
                id: "1".into(),
                name: "Reminders".into(),
            }],
            reminders: Vec::new(),
        }
    }
}

impl Data {
    pub fn take_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        id.to_string()
    }

    /// A calendar by id, or by name (models often pass the name).
    pub fn calendar_id(&self, key: &str) -> Option<String> {
        let k = key.trim();
        self.calendars
            .iter()
            .find(|c| c.id == k)
            .or_else(|| {
                self.calendars
                    .iter()
                    .find(|c| c.name.to_lowercase() == k.to_lowercase())
            })
            .map(|c| c.id.clone())
    }

    /// A reminder list by id or name.
    pub fn list_id(&self, key: &str) -> Option<String> {
        let k = key.trim();
        self.reminder_lists
            .iter()
            .find(|l| l.id == k)
            .or_else(|| {
                self.reminder_lists
                    .iter()
                    .find(|l| l.name.to_lowercase() == k.to_lowercase())
            })
            .map(|l| l.id.clone())
    }

    pub fn primary_calendar(&self) -> Option<&Calendar> {
        self.calendars
            .iter()
            .find(|c| c.is_primary)
            .or_else(|| self.calendars.first())
    }
}

/// Local wall-clock time from ISO 8601: `2026-11-10`, `2026-11-10T14:30[:00]`, or with an offset
/// (`Z`, `+03:00`), which is converted to local time.
pub fn normalize(raw: &str) -> Result<NaiveDateTime, String> {
    let s = raw.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Local).naive_local());
    }
    for f in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, f) {
            return Ok(dt);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .ok_or_else(|| format!("`{raw}` is not an ISO 8601 date or time such as 2026-11-10T14:30"))
}

pub fn key(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    fn load(path: &Path) -> Result<Data, String> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| format!("{} is damaged: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Data::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(path: &Path, data: &Data) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Read only.
    pub async fn read<T>(&self, f: impl FnOnce(&Data) -> T) -> Result<T, String> {
        let _g = self.lock.lock().await;
        Ok(f(&Self::load(&self.path)?))
    }

    /// Read, change, write back when `f` succeeds.
    pub async fn update<T>(
        &self,
        f: impl FnOnce(&mut Data) -> Result<T, String>,
    ) -> Result<T, String> {
        let _g = self.lock.lock().await;
        let mut data = Self::load(&self.path)?;
        let out = f(&mut data)?;
        Self::save(&self.path, &data)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_normalize_to_local_wall_clock() {
        assert_eq!(key(normalize("2026-11-10").unwrap()), "2026-11-10T00:00:00");
        assert_eq!(
            key(normalize("2026-11-10T14:30").unwrap()),
            "2026-11-10T14:30:00"
        );
        assert_eq!(
            key(normalize("2026-11-10 09:05:07").unwrap()),
            "2026-11-10T09:05:07"
        );
        let utc = normalize("2026-11-10T12:00:00Z").unwrap();
        let expected = DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Local)
            .naive_local();
        assert_eq!(utc, expected);
        assert!(normalize("next tuesday").is_err());
    }

    #[tokio::test]
    async fn store_round_trips_and_keeps_ids_unique() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("assistant.json"));
        let a = s.update(|d| Ok(d.take_id())).await.unwrap();
        let b = s.update(|d| Ok(d.take_id())).await.unwrap();
        assert_ne!(a, b);
        let primary = s
            .read(|d| d.primary_calendar().map(|c| c.id.clone()))
            .await
            .unwrap();
        assert_eq!(primary.as_deref(), Some("1"));
        let failed: Result<(), String> = s
            .update(|d| {
                d.reminders.clear();
                d.next_id = 99;
                Err("nope".into())
            })
            .await;
        assert!(failed.is_err());
        assert_eq!(
            s.read(|d| d.next_id).await.unwrap(),
            4,
            "a failed change is not written"
        );
    }
}
