//! Windows 11 Widgets board card for Kor (§27.10): recent sessions, which of them were active in
//! the last minutes, and buttons that open Kor. Pure logic: the provider (`kor-widgets.exe`)
//! sends [`TEMPLATE`] once and [`data`] on every refresh; actions come back as [`Action`].
//!
//! The source is Kor's session store (`%LOCALAPPDATA%\kor\sessions\*.jsonl`, same place as
//! `kor_runtime::sessions_dir`). "Active" means written in the last [`ACTIVE_SECS`] seconds; the
//! card says exactly that rather than claiming a task is running.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::menu::Lang;

/// Widget definition id in the package manifest.
pub const DEFINITION_ID: &str = "kor_tasks";
/// A session written this recently counts as active.
pub const ACTIVE_SECS: u64 = 300;
const TITLE_CHARS: usize = 60;

/// Adaptive Card template (data bound with `${…}`).
pub const TEMPLATE: &str = r#"{
  "type": "AdaptiveCard",
  "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
  "version": "1.5",
  "body": [
    { "type": "TextBlock", "text": "${heading}", "weight": "Bolder", "size": "Medium" },
    { "type": "TextBlock", "text": "${activeLabel}", "isSubtle": true, "wrap": true, "spacing": "None" },
    {
      "type": "Container",
      "$when": "${count(sessions) > 0}",
      "items": [
        {
          "type": "ColumnSet",
          "$data": "${sessions}",
          "selectAction": { "type": "Action.Execute", "verb": "resume", "data": { "id": "${id}" } },
          "columns": [
            { "type": "Column", "width": "stretch", "items": [
              { "type": "TextBlock", "text": "${title}", "maxLines": 1, "weight": "${if(active, 'Bolder', 'Default')}" }
            ] },
            { "type": "Column", "width": "auto", "items": [
              { "type": "TextBlock", "text": "${when}", "isSubtle": true }
            ] }
          ]
        }
      ]
    },
    { "type": "TextBlock", "$when": "${count(sessions) == 0}", "text": "${emptyLabel}", "isSubtle": true, "wrap": true }
  ],
  "actions": [
    { "type": "Action.Execute", "title": "${openLabel}", "verb": "open" },
    { "type": "Action.Execute", "title": "${newLabel}", "verb": "new" }
  ]
}"#;

/// One session line on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub modified: SystemTime,
}

/// Kor's session folder.
pub fn sessions_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("kor").join("sessions"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Title of a session file: its first user message, shortened.
fn title_of(path: &Path) -> Option<String> {
    use std::io::BufRead as _;
    let f = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(f).lines().take(50) {
        let Ok(v) = serde_json::from_str::<Value>(&line.ok()?) else {
            continue;
        };
        if v.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let text: String = v
            .get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        return Some(if text.chars().count() > TITLE_CHARS {
            format!(
                "{}…",
                text.chars().take(TITLE_CHARS - 1).collect::<String>()
            )
        } else {
            text
        });
    }
    None
}

/// The newest `limit` sessions in `dir`.
pub fn recent(dir: &Path, limit: usize) -> Vec<Session> {
    let mut all: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    all.sort_by(|a, b| b.0.cmp(&a.0));
    all.into_iter()
        .filter_map(|(modified, path)| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            valid_id(&id).then_some(())?;
            let title = title_of(&path).unwrap_or_else(|| id.clone());
            Some(Session {
                id,
                title,
                modified,
            })
        })
        .take(limit)
        .collect()
}

/// How many sessions the card shows for a Widgets size (`small`, `medium`, `large`).
pub fn rows_for(size: &str) -> usize {
    match size {
        "small" => 1,
        "large" => 5,
        _ => 3,
    }
}

fn ago(lang: Lang, age: Duration) -> String {
    let m = age.as_secs() / 60;
    let (n, unit) = if m < 60 {
        (m.max(1), 0)
    } else if m < 24 * 60 {
        (m / 60, 1)
    } else {
        (m / (24 * 60), 2)
    };
    match lang {
        Lang::Tr => format!("{n} {} önce", ["dk", "sa", "g"][unit]),
        Lang::En => format!("{n} {} ago", ["min", "h", "d"][unit]),
        Lang::Fr => format!("il y a {n} {}", ["min", "h", "j"][unit]),
    }
}

/// Card data for `sessions` at time `now`.
pub fn data(lang: Lang, sessions: &[Session], now: SystemTime) -> Value {
    let age = |s: &Session| now.duration_since(s.modified).unwrap_or_default();
    let active = sessions
        .iter()
        .filter(|s| age(s).as_secs() < ACTIVE_SECS)
        .count();
    let (heading, active_label, empty, open, new) = match lang {
        Lang::Tr => (
            "Kor görevleri",
            if active == 0 {
                "Son 5 dakikada etkin oturum yok".to_owned()
            } else {
                format!("Son 5 dakikada {active} etkin oturum")
            },
            "Henüz oturum yok.",
            "Kor'u aç",
            "Yeni görev",
        ),
        Lang::En => (
            "Kor tasks",
            if active == 0 {
                "No session active in the last 5 minutes".to_owned()
            } else {
                format!("{active} session(s) active in the last 5 minutes")
            },
            "No sessions yet.",
            "Open Kor",
            "New task",
        ),
        Lang::Fr => (
            "Tâches Kor",
            if active == 0 {
                "Aucune session active ces 5 dernières minutes".to_owned()
            } else {
                format!("{active} session(s) active(s) ces 5 dernières minutes")
            },
            "Aucune session pour l'instant.",
            "Ouvrir Kor",
            "Nouvelle tâche",
        ),
    };
    let rows: Vec<Value> = sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "when": ago(lang, age(s)),
                "active": age(s).as_secs() < ACTIVE_SECS,
            })
        })
        .collect();
    json!({
        "heading": heading,
        "activeLabel": active_label,
        "emptyLabel": empty,
        "openLabel": open,
        "newLabel": new,
        "sessions": rows,
    })
}

/// A card button the user pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open,
    New,
    Resume(String),
}

impl Action {
    /// From `Action.Execute` verb and data (JSON text from the Widgets host: untrusted).
    pub fn parse(verb: &str, data: &str) -> Option<Self> {
        match verb {
            "open" => Some(Self::Open),
            "new" => Some(Self::New),
            "resume" => {
                let v: Value = serde_json::from_str(data).ok()?;
                let id = v.get("id")?.as_str()?;
                valid_id(id).then(|| Self::Resume(id.to_owned()))
            }
            _ => None,
        }
    }

    /// `kor-desktop` arguments (`kor_runtime::osint`).
    pub fn args(&self) -> Vec<OsString> {
        match self {
            Self::Open => Vec::new(),
            Self::New => vec!["kor://new".into()],
            Self::Resume(id) => vec![format!("kor://resume?id={id}").into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_valid_json_with_the_bound_fields() {
        let t: Value = serde_json::from_str(TEMPLATE).unwrap();
        assert_eq!(t["type"], "AdaptiveCard");
        let d = data(Lang::En, &[], SystemTime::now());
        for key in [
            "heading",
            "activeLabel",
            "emptyLabel",
            "openLabel",
            "newLabel",
        ] {
            assert!(TEMPLATE.contains(&format!("${{{key}}}")), "{key}");
            assert!(d[key].is_string(), "{key}");
        }
    }

    #[test]
    fn recent_sessions_are_read_newest_first_with_titles() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("20260101-000000-1.jsonl");
        std::fs::write(
            &old,
            "{\"role\":\"system\",\"parts\":[]}\n{\"role\":\"user\",\"parts\":[{\"type\":\"text\",\"text\":\"fix   the login\"}]}\n",
        )
        .unwrap();
        let t0 = SystemTime::now() - Duration::from_secs(7200);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(t0)
            .unwrap();
        let long = "x".repeat(100);
        std::fs::write(
            dir.path().join("20260102-000000-2.jsonl"),
            format!(
                "{{\"role\":\"user\",\"parts\":[{{\"type\":\"text\",\"text\":\"{long}\"}}]}}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.path().join("bad id!.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        let s = recent(dir.path(), 5);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].id, "20260102-000000-2");
        assert_eq!(s[0].title.chars().count(), TITLE_CHARS);
        assert_eq!(s[1].title, "fix the login");
        assert_eq!(recent(dir.path(), 1).len(), 1);

        let d = data(Lang::Tr, &s, SystemTime::now());
        assert_eq!(d["activeLabel"], "Son 5 dakikada 1 etkin oturum");
        assert_eq!(d["sessions"][0]["active"], true);
        assert_eq!(d["sessions"][1]["when"], "2 sa önce");
        assert_eq!(d["sessions"][1]["active"], false);
    }

    #[test]
    fn actions_are_validated() {
        assert_eq!(Action::parse("open", ""), Some(Action::Open));
        assert_eq!(
            Action::parse("resume", r#"{"id":"abc-1"}"#).unwrap().args(),
            ["kor://resume?id=abc-1"]
        );
        assert_eq!(Action::parse("resume", r#"{"id":"a&path=C:\\"}"#), None);
        assert_eq!(Action::parse("resume", "not json"), None);
        assert_eq!(Action::parse("rm", "{}"), None);
        assert_eq!(Action::New.args(), ["kor://new"]);
        assert_eq!(
            (rows_for("small"), rows_for("medium"), rows_for("large")),
            (1, 3, 5)
        );
        assert_eq!(ago(Lang::Fr, Duration::from_secs(3 * 86400)), "il y a 3 j");
        assert_eq!(ago(Lang::En, Duration::from_secs(5)), "1 min ago");
    }
}
