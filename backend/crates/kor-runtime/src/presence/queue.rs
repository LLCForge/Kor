//! One approval queue over every source (§28.5): the open desktop app's chats, `kor daemon`
//! jobs (ADR 0039) and tasks run on this machine by `kor remote-serve` (ADR 0044). `kor approve`,
//! the tray, the status window and the desktop queue panel all read and answer through here.
//!
//! Answers go back the way each source already accepts them: the desktop through its local
//! answer folder ([`super::desktop`]), the daemon through its token-protected loopback control
//! API, a remote task through its inbox. Nothing is ever answered "always".

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::desktop::{self, Approval, Source};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    #[default]
    Low,
    Medium,
    High,
}

/// Risk badge: destructive steps are high; commands and writes medium; reads low.
pub fn risk(a: &Approval) -> Risk {
    if a.destructive.is_some() {
        return Risk::High;
    }
    let t = a.tool.to_ascii_lowercase();
    if [
        "bash",
        "shell",
        "powershell",
        "exec",
        "run",
        "computer",
        "browser",
        "budget",
    ]
    .iter()
    .any(|k| t.contains(k))
        || !a.files.is_empty()
        || ["write", "edit", "delete", "move", "patch"]
            .iter()
            .any(|k| t.contains(k))
    {
        Risk::Medium
    } else {
        Risk::Low
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    #[serde(flatten)]
    pub approval: Approval,
    pub risk: Risk,
    /// Items with the same group key can be answered together.
    pub group: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Group {
    pub key: String,
    pub tool: String,
    /// Summary of the first item.
    pub summary: String,
    pub ids: Vec<String>,
    pub risk: Risk,
}

/// Summary shape without the parts that differ between similar requests: paths become `<path>`,
/// numbers `#`, whitespace collapsed.
fn shape(summary: &str) -> String {
    summary
        .split_whitespace()
        .map(|w| {
            if w.contains('/')
                || w.contains('\\')
                || (w.contains('.') && w.len() > 3 && !w.ends_with('.'))
            {
                "<path>".to_owned()
            } else {
                w.chars()
                    .map(|c| {
                        if c.is_ascii_digit() {
                            '#'
                        } else {
                            c.to_ascii_lowercase()
                        }
                    })
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn group_key(a: &Approval) -> String {
    format!("{}|{}", a.tool.to_ascii_lowercase(), shape(&a.summary))
}

/// Oldest first, with risk and group key.
pub fn items(mut approvals: Vec<Approval>) -> Vec<Item> {
    approvals.sort_by(|a, b| a.since.cmp(&b.since).then(a.id.cmp(&b.id)));
    approvals
        .into_iter()
        .map(|a| Item {
            risk: risk(&a),
            group: group_key(&a),
            approval: a,
        })
        .collect()
}

/// Groups of two or more similar requests ("same change in 5 files — approve all").
pub fn groups(items: &[Item]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for it in items {
        match out.iter_mut().find(|g| g.key == it.group) {
            Some(g) => {
                g.ids.push(it.approval.id.clone());
                g.risk = g.risk.max(it.risk);
            }
            None => out.push(Group {
                key: it.group.clone(),
                tool: it.approval.tool.clone(),
                summary: it.approval.summary.clone(),
                ids: vec![it.approval.id.clone()],
                risk: it.risk,
            }),
        }
    }
    out.retain(|g| g.ids.len() > 1);
    out
}

/// Desktop approvals from a live snapshot.
pub fn from_desktop(presence: &Path, now: u64) -> Vec<Approval> {
    desktop::read_live(presence, now)
        .map(|s| s.approvals)
        .unwrap_or_default()
}

/// Daemon approvals from `kor daemon` status JSON (`waiting: [{id, job, summary}]`).
pub fn from_daemon(daemon_dir: &Path, status: &Value, now: u64) -> Vec<Approval> {
    status["waiting"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let id = w["id"].as_str()?.to_owned();
            let job = w["job"].as_str().unwrap_or_default().to_owned();
            let loaded = crate::triggers::load_job(daemon_dir, &job);
            Some(Approval {
                id,
                source: Source::Daemon,
                title: loaded
                    .as_ref()
                    .map(|j| j.trigger.clone())
                    .unwrap_or_default(),
                tool: "trigger".into(),
                summary: w["summary"].as_str().unwrap_or_default().to_owned(),
                since: loaded.and_then(|j| j.started).unwrap_or(now),
                owner: job,
                ..Approval::default()
            })
        })
        .collect()
}

/// Waiting `kor remote-serve` tasks on this machine; ids are `<task>:<confirm id>`.
pub fn from_remote(store: &crate::remote::TaskStore) -> Vec<Approval> {
    store
        .list()
        .into_iter()
        .filter(|t| t.state == "waiting")
        .flat_map(|t| {
            store
                .pending_confirms(&t.id)
                .into_iter()
                .filter_map(move |c| {
                    let cid = c["id"].as_str()?;
                    let id = format!("{}:{cid}", t.id);
                    desktop::valid_approval_id(&id).then(|| Approval {
                        id,
                        source: Source::Remote,
                        owner: t.id.clone(),
                        title: t.prompt.clone(),
                        tool: c["name"].as_str().unwrap_or_default().to_owned(),
                        summary: c["summary"].as_str().unwrap_or_default().to_owned(),
                        destructive: c["destructive"].as_str().map(str::to_owned),
                        files: c["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|f| f["path"].as_str().map(str::to_owned))
                            .collect(),
                        since: c["at"].as_u64().unwrap_or(t.updated),
                    })
                })
        })
        .collect()
}

/// Every pending approval on this machine (the daemon is asked over its control API).
pub async fn collect() -> Vec<Item> {
    let now = super::now_ms();
    let mut all = Vec::new();
    if let Some(p) = super::dir() {
        all.extend(from_desktop(&p, now));
    }
    if let Some(d) = crate::triggers::dir() {
        let status = crate::triggers::daemon::status(&d).await;
        all.extend(from_daemon(&d, &status, now));
    }
    if let Some(store) = crate::remote::TaskStore::standard() {
        all.extend(from_remote(&store));
    }
    items(all)
}

/// How long `answer` waits for the desktop app to pick up an answer.
const DESKTOP_WAIT: std::time::Duration = std::time::Duration::from_secs(4);

/// Answer `id` wherever it is pending; returns its source.
///
/// # Errors
/// When no source has it pending, or the source refused the answer.
pub async fn answer(id: &str, approve: bool) -> Result<Source, String> {
    if !desktop::valid_approval_id(id) {
        return Err("bad approval id".into());
    }
    let pending = collect().await;
    let item = pending
        .iter()
        .find(|i| i.approval.id == id)
        .ok_or_else(|| format!("no pending approval {id}"))?;
    match item.approval.source {
        Source::Desktop => {
            let dir = super::dir().ok_or("no Kor data folder")?;
            desktop::write_answer(&dir, id, approve)?;
            let deadline = std::time::Instant::now() + DESKTOP_WAIT;
            while desktop::answer_pending(&dir, id) {
                if std::time::Instant::now() > deadline {
                    desktop::cancel_answer(&dir, id);
                    return Err("the desktop app did not pick up the answer".into());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        Source::Daemon => {
            let d = crate::triggers::dir().ok_or("no Kor data folder")?;
            let v = crate::triggers::daemon::control(
                &d,
                "POST",
                "/control/answer",
                Some(serde_json::json!({ "id": id, "approve": approve })),
            )
            .await?;
            if v["ok"] != true {
                return Err(format!("no pending approval {id}"));
            }
        }
        Source::Remote => {
            let store = crate::remote::TaskStore::standard().ok_or("no Kor data folder")?;
            let (task, cid) = id.split_once(':').ok_or("bad approval id")?;
            store.push(
                task,
                &kor_agent::remote::wire::Command::Approve {
                    id: cid.to_owned(),
                    approval: if approve { "yes" } else { "no" }.into(),
                    files: None,
                },
            )?;
        }
    }
    if let Some(s) = super::notifications::Store::standard() {
        let _ = s.resolve_approval(id);
    }
    Ok(item.approval.source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(id: &str, tool: &str, summary: &str, since: u64) -> Approval {
        Approval {
            id: id.into(),
            tool: tool.into(),
            summary: summary.into(),
            since,
            ..Approval::default()
        }
    }

    #[test]
    fn risk_badges() {
        let mut a = ap("1", "Read", "src/a.rs", 0);
        assert_eq!(risk(&a), Risk::Low);
        a.tool = "Bash".into();
        assert_eq!(risk(&a), Risk::Medium);
        a.destructive = Some("rm".into());
        assert_eq!(risk(&a), Risk::High);
        let mut w = ap("2", "Edit", "x", 0);
        w.files = vec!["a".into()];
        assert_eq!(risk(&w), Risk::Medium);
    }

    #[test]
    fn similar_requests_group_and_order_is_oldest_first() {
        let list = items(vec![
            ap("c", "Edit", "Add import to src/c.rs", 3),
            ap("a", "Edit", "Add import to src/a.rs", 1),
            ap("b", "Edit", "add import to lib/b.ts", 2),
            ap("d", "Bash", "cargo test -p x", 4),
        ]);
        assert_eq!(
            list.iter()
                .map(|i| i.approval.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "d"]
        );
        let g = groups(&list);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].ids, ["a", "b", "c"]);
        assert_eq!(g[0].risk, Risk::Medium, "edits are medium risk");
    }

    #[test]
    fn daemon_waiting_becomes_items() {
        let d = tempfile::tempdir().unwrap();
        let status = serde_json::json!({ "waiting": [{ "id": "ab12", "job": "0000-ff", "summary": "push main" }] });
        let v = from_daemon(d.path(), &status, 5);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].source, Source::Daemon);
        assert_eq!(v[0].since, 5);
        assert!(from_daemon(d.path(), &serde_json::json!({ "running": false }), 5).is_empty());
    }

    #[test]
    fn remote_confirms_are_pending_until_confirmed() {
        let d = tempfile::tempdir().unwrap();
        let store = crate::remote::TaskStore {
            dir: d.path().to_owned(),
        };
        let id = store.create("/p", "fix the build").unwrap();
        let mut s = store.state(&id).unwrap();
        s.state = "waiting".into();
        store.save_state(&s).unwrap();
        let events = [
            r#"{"type":"confirm","id":"c1","name":"Bash","summary":"ls","files":[]}"#,
            r#"{"type":"confirmed","id":"c1","approval":"yes"}"#,
            r#"{"type":"confirm","id":"c2","name":"Edit","summary":"a.rs","files":[{"path":"a.rs"}],"at":9}"#,
        ];
        std::fs::write(store.task_dir(&id).join("events.jsonl"), events.join("\n")).unwrap();
        let v = from_remote(&store);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, format!("{id}:c2"));
        assert_eq!(v[0].files, ["a.rs"]);
        assert_eq!(v[0].since, 9);
        assert_eq!(v[0].source, Source::Remote);
    }

    #[test]
    fn desktop_snapshot_feeds_the_queue() {
        let d = tempfile::tempdir().unwrap();
        let snap = desktop::Snapshot {
            updated: 100,
            approvals: vec![ap("toolu_1", "Edit", "x", 1)],
            ..desktop::Snapshot::default()
        };
        desktop::write(d.path(), &snap).unwrap();
        assert_eq!(from_desktop(d.path(), 200).len(), 1);
        assert!(from_desktop(d.path(), 100 + desktop::STALE_MS + 1).is_empty());
    }
}
