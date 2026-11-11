//! Notification center store (§27.2): one list for the desktop bell, OS toasts and
//! `kor notifications`. `<presence>/notifications.json`, newest last on disk, 30 days, 500 items.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const KEEP_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const MAX_ITEMS: usize = 500;
const MAX_TEXT: usize = 2000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A task waits for an approval.
    Approval,
    /// A task asks the user a question.
    Question,
    /// A task finished.
    #[default]
    Done,
    Error,
    /// Budget alert / limit reached.
    Budget,
    /// A new Kor version.
    Update,
    /// Health warnings (§19.3).
    Health,
    /// Component installs (§24).
    Component,
    /// Provider quota warnings.
    Quota,
}

impl Category {
    pub const ALL: [Self; 9] = [
        Self::Approval,
        Self::Question,
        Self::Done,
        Self::Error,
        Self::Budget,
        Self::Update,
        Self::Health,
        Self::Component,
        Self::Quota,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::Question => "question",
            Self::Done => "done",
            Self::Error => "error",
            Self::Budget => "budget",
            Self::Update => "update",
            Self::Health => "health",
            Self::Component => "component",
            Self::Quota => "quota",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == s)
    }

    /// Kor waits for the user (flash / bounce / BEL apply).
    pub fn waits_for_user(self) -> bool {
        matches!(self, Self::Approval | Self::Question)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    /// Unix ms.
    pub at: u64,
    pub category: Category,
    pub title: String,
    pub body: String,
    /// Desktop chat it belongs to (opens it on click).
    pub session: Option<String>,
    /// `kor daemon` job.
    pub job: Option<String>,
    /// Approval id (answerable from the bell / `kor approve`).
    pub approval: Option<String>,
    pub cost_usd: Option<f64>,
    /// The cost is a catalog estimate (`≈`).
    pub cost_estimated: bool,
    /// Files changed by the task.
    pub files: Option<u32>,
    pub read: bool,
    /// Dedup key: a notification with the same key is not added twice (budget alert per day).
    pub key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    items: Vec<Notification>,
    next: u64,
}

/// The store at one path.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// `<kor home>/presence/notifications.json`.
    pub fn standard() -> Option<Self> {
        super::dir().map(|d| Self::at(&d.join("notifications.json")))
    }

    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    /// Newest first, older than 30 days dropped.
    pub fn list(&self) -> Vec<Notification> {
        let f: File = super::read_json(&self.path);
        let cutoff = super::now_ms().saturating_sub(KEEP_MS);
        let mut v: Vec<Notification> = f.items.into_iter().filter(|n| n.at >= cutoff).collect();
        v.reverse();
        v
    }

    pub fn unread(&self) -> usize {
        self.list().iter().filter(|n| !n.read).count()
    }

    /// Adds `n` (id and time assigned, text clipped). `Ok(None)` when its `key` is already there.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn push(&self, mut n: Notification) -> Result<Option<Notification>, String> {
        super::edit_json(&self.path, |f: &mut File| {
            let now = super::now_ms();
            f.items.retain(|x| x.at >= now.saturating_sub(KEEP_MS));
            if n.key.is_some() && f.items.iter().any(|x| x.key == n.key) {
                return None;
            }
            f.next += 1;
            n.id = format!("n{}", f.next);
            n.at = now;
            n.read = false;
            n.title = clip(&n.title);
            n.body = clip(&n.body);
            f.items.push(n.clone());
            let over = f.items.len().saturating_sub(MAX_ITEMS);
            f.items.drain(..over);
            Some(n)
        })
    }

    /// Marks `ids` (all when `None`) read; returns how many changed.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn mark_read(&self, ids: Option<&[String]>) -> Result<usize, String> {
        super::edit_json(&self.path, |f: &mut File| {
            let mut n = 0;
            for x in &mut f.items {
                if !x.read && ids.is_none_or(|ids| ids.contains(&x.id)) {
                    x.read = true;
                    n += 1;
                }
            }
            n
        })
    }

    /// Removes `ids` (all when `None`); returns how many were removed.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn clear(&self, ids: Option<&[String]>) -> Result<usize, String> {
        super::edit_json(&self.path, |f: &mut File| {
            let before = f.items.len();
            f.items
                .retain(|x| ids.is_some_and(|ids| !ids.contains(&x.id)));
            before - f.items.len()
        })
    }

    /// Notifications for an approval that has been answered are marked read.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn resolve_approval(&self, approval: &str) -> Result<usize, String> {
        super::edit_json(&self.path, |f: &mut File| {
            let mut n = 0;
            for x in &mut f.items {
                if x.approval.as_deref() == Some(approval) && !x.read {
                    x.read = true;
                    n += 1;
                }
            }
            n
        })
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(MAX_TEXT - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(cat: Category, title: &str) -> Notification {
        Notification {
            category: cat,
            title: title.into(),
            ..Notification::default()
        }
    }

    #[test]
    fn push_list_read_clear_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(&d.path().join("n.json"));
        assert!(s.list().is_empty());
        let first = s.push(note(Category::Done, "a")).unwrap().unwrap();
        let second = s.push(note(Category::Approval, "b")).unwrap().unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(s.list()[0].title, "b", "newest first");
        assert_eq!(s.unread(), 2);
        assert_eq!(
            s.mark_read(Some(std::slice::from_ref(&first.id))).unwrap(),
            1
        );
        assert_eq!(s.unread(), 1);
        assert_eq!(s.mark_read(None).unwrap(), 1);
        assert_eq!(s.clear(Some(std::slice::from_ref(&second.id))).unwrap(), 1);
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.clear(None).unwrap(), 1);
        assert!(s.list().is_empty());
    }

    #[test]
    fn keys_dedupe_and_text_is_clipped() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(&d.path().join("n.json"));
        let mut n = note(Category::Budget, &"x".repeat(5000));
        n.key = Some("budget:daily:2026-11-10".into());
        let first = s.push(n.clone()).unwrap().unwrap();
        assert_eq!(first.title.chars().count(), MAX_TEXT);
        assert!(s.push(n).unwrap().is_none());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn old_items_expire_and_the_list_is_capped() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("n.json");
        let old = File {
            items: vec![Notification {
                id: "n1".into(),
                at: 1,
                ..Notification::default()
            }],
            next: 1,
        };
        std::fs::write(&path, serde_json::to_string(&old).unwrap()).unwrap();
        let s = Store::at(&path);
        assert!(s.list().is_empty(), "older than 30 days");
        for i in 0..(MAX_ITEMS + 5) {
            s.push(note(Category::Done, &i.to_string())).unwrap();
        }
        assert_eq!(s.list().len(), MAX_ITEMS);
    }

    #[test]
    fn answered_approvals_are_marked_read() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(&d.path().join("n.json"));
        let mut n = note(Category::Approval, "ok?");
        n.approval = Some("call_1".into());
        s.push(n).unwrap();
        assert_eq!(s.resolve_approval("call_1").unwrap(), 1);
        assert_eq!(s.unread(), 0);
    }

    #[test]
    fn categories_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.key()), Some(c));
        }
        assert!(Category::Approval.waits_for_user());
        assert!(!Category::Done.waits_for_user());
    }
}
