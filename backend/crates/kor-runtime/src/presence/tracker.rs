//! Desktop chat state for the tray, status window, taskbar and `kor status`, folded from the same
//! UI events the chat view receives (`kor:event` bodies: `turnStart`, `status`, `tool`,
//! `toolResult`, `confirm`, `done`, `error`, ...). Pure: the desktop applies the [`Effect`]s.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use super::desktop::{Approval, Running, Source};
use super::notifications::Category;

/// Something the desktop does after an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Kor waits for the user: notify (category approval or question).
    Waiting {
        session: String,
        approval: String,
        category: Category,
        tool: String,
        summary: String,
    },
    /// A turn ended (`error` set when it failed): notify, record its duration.
    Ended {
        session: String,
        title: String,
        secs: u64,
        root: String,
        cost_usd: f64,
        cost_estimated: bool,
        /// Files the turn changed.
        files: u32,
        error: Option<String>,
    },
}

#[derive(Debug, Default)]
pub struct Tracker {
    pub sessions: BTreeMap<String, Running>,
    pub approvals: Vec<Approval>,
    /// tool call id → tool name (results carry only the id).
    tools: HashMap<String, String>,
    /// Session → error of the current turn.
    errors: HashMap<String, String>,
    /// Session → files changed in the current turn.
    files: HashMap<String, BTreeSet<String>>,
}

/// `[done, total]` from a `TodoWrite` result (`now:` block of `- [status] …` lines).
pub fn todo_progress(result: &str) -> Option<[u32; 2]> {
    let now = result.rsplit_once("now:\n").map_or(result, |(_, n)| n);
    let mut done = 0u32;
    let mut total = 0u32;
    for line in now.lines() {
        let Some(rest) = line.strip_prefix("- [") else {
            continue;
        };
        total += 1;
        if rest.starts_with("completed]") {
            done += 1;
        }
    }
    (total > 0).then_some([done, total])
}

/// Verb key for the status window when the core sent no `status` yet.
fn tool_verb(tool: &str) -> &'static str {
    let t = tool.to_ascii_lowercase();
    if t.contains("read") || t.contains("grep") || t.contains("glob") || t.contains("search") {
        "reading"
    } else if t.contains("edit") || t.contains("write") || t.contains("patch") {
        "editing"
    } else if t.contains("bash") || t.contains("shell") || t.contains("powershell") {
        "running"
    } else {
        "working"
    }
}

impl Tracker {
    /// A turn of `session` started (`title` and `root` for the lists).
    pub fn start(&mut self, session: &str, title: &str, root: &str, now: u64) {
        self.errors.remove(session);
        self.files.remove(session);
        self.sessions.insert(
            session.to_owned(),
            Running {
                id: session.to_owned(),
                title: title.to_owned(),
                root: root.to_owned(),
                since: now,
                verb: "thinking".into(),
                ..Running::default()
            },
        );
    }

    /// The model call of `session` cost `usd`.
    pub fn add_cost(&mut self, session: &str, usd: f64, estimated: bool) {
        if let Some(r) = self.sessions.get_mut(session) {
            r.cost_usd += usd;
            r.cost_estimated |= estimated;
        }
    }

    /// An approval was answered in the app (or the turn was cancelled).
    pub fn answered(&mut self, approval: &str) {
        self.approvals.retain(|a| a.id != approval);
        self.refresh_waiting();
    }

    fn refresh_waiting(&mut self) {
        for r in self.sessions.values_mut() {
            r.waiting = self.approvals.iter().any(|a| a.owner == r.id);
        }
    }

    #[allow(clippy::too_many_lines)] // one flat match over the event types
    pub fn observe(&mut self, session: &str, ev: &Value, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let kind = ev["type"].as_str().unwrap_or_default();
        match kind {
            "status" => {
                if let Some(r) = self.sessions.get_mut(session)
                    && let Some(v) = ev["verb"].as_str()
                {
                    v.clone_into(&mut r.verb);
                }
            }
            "tool" => {
                let name = ev["name"].as_str().unwrap_or_default();
                if let Some(id) = ev["id"].as_str() {
                    self.tools.insert(id.to_owned(), name.to_owned());
                }
                if let Some(r) = self.sessions.get_mut(session) {
                    r.tool = Some(name.to_owned());
                    if r.verb == "thinking" || r.verb.is_empty() {
                        r.verb = tool_verb(name).into();
                    }
                }
            }
            "toolResult" => {
                let name = ev["id"]
                    .as_str()
                    .and_then(|id| self.tools.remove(id))
                    .unwrap_or_default();
                if let Some(r) = self.sessions.get_mut(session) {
                    r.tool = None;
                    if name == "TodoWrite"
                        && let Some(p) = ev["result"].as_str().and_then(todo_progress)
                    {
                        r.progress = Some(p);
                    }
                }
            }
            "confirm" => {
                let id = ev["id"].as_str().unwrap_or_default().to_owned();
                let tool = ev["name"].as_str().unwrap_or_default().to_owned();
                let summary = ev["summary"].as_str().unwrap_or_default().to_owned();
                let title = self
                    .sessions
                    .get(session)
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                self.approvals.retain(|a| a.id != id);
                self.approvals.push(Approval {
                    id: id.clone(),
                    source: Source::Desktop,
                    owner: session.to_owned(),
                    title,
                    tool: tool.clone(),
                    summary: summary.clone(),
                    destructive: ev["destructive"].as_str().map(str::to_owned),
                    files: ev["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f["path"].as_str().map(str::to_owned))
                        .collect(),
                    since: now,
                });
                self.refresh_waiting();
                let category = if tool == "AskUserQuestion" {
                    Category::Question
                } else {
                    Category::Approval
                };
                out.push(Effect::Waiting {
                    session: session.to_owned(),
                    approval: id,
                    category,
                    tool,
                    summary,
                });
            }
            "fileChanged" => {
                if let Some(p) = ev["path"].as_str() {
                    self.files
                        .entry(session.to_owned())
                        .or_default()
                        .insert(p.to_owned());
                }
            }
            "error" => {
                let msg = ev["message"].as_str().unwrap_or_default().to_owned();
                self.errors.insert(session.to_owned(), msg);
            }
            "done" => {
                self.approvals.retain(|a| a.owner != session);
                if let Some(r) = self.sessions.remove(session) {
                    out.push(Effect::Ended {
                        session: session.to_owned(),
                        title: r.title,
                        secs: now.saturating_sub(r.since) / 1000,
                        root: r.root,
                        cost_usd: r.cost_usd,
                        cost_estimated: r.cost_estimated,
                        files: self
                            .files
                            .remove(session)
                            .map_or(0, |f| u32::try_from(f.len()).unwrap_or(u32::MAX)),
                        error: self.errors.remove(session),
                    });
                }
                self.refresh_waiting();
            }
            _ => {}
        }
        out
    }

    /// Overall progress for the taskbar: `None` idle, `Some(None)` indeterminate, else 0–100.
    pub fn progress(&self) -> Option<Option<u8>> {
        if self.sessions.is_empty() {
            return None;
        }
        let (mut done, mut total) = (0u32, 0u32);
        for r in self.sessions.values() {
            match r.progress {
                Some([d, t]) => {
                    done += d;
                    total += t;
                }
                // One running chat without a task list: the bar cannot be honest.
                None => return Some(None),
            }
        }
        if total == 0 {
            return Some(None);
        }
        Some(Some(
            u8::try_from(u64::from(done) * 100 / u64::from(total)).unwrap_or(100),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn todo_progress_reads_the_new_list() {
        let r = "previous:\n- [pending] a (A)\n\nnow:\n- [completed] a (A)\n- [in_progress] b (B)\n- [pending] c (C)";
        assert_eq!(todo_progress(r), Some([1, 3]));
        assert_eq!(todo_progress("now:\n(empty)"), None);
    }

    #[test]
    fn a_turn_with_an_approval() {
        let mut t = Tracker::default();
        t.start("s1", "Fix build", "/p", 1_000);
        assert_eq!(t.progress(), Some(None));
        t.observe("s1", &json!({"type": "status", "verb": "Közlüyor"}), 1_100);
        assert_eq!(t.sessions["s1"].verb, "Közlüyor");
        t.observe(
            "s1",
            &json!({"type": "tool", "id": "c1", "name": "TodoWrite", "summary": "2 todos"}),
            1_200,
        );
        t.observe(
            "s1",
            &json!({"type": "toolResult", "id": "c1", "result": "now:\n- [completed] a (A)\n- [pending] b (B)"}),
            1_300,
        );
        assert_eq!(t.sessions["s1"].progress, Some([1, 2]));
        assert_eq!(t.progress(), Some(Some(50)));
        let fx = t.observe(
            "s1",
            &json!({"type": "confirm", "id": "c2", "name": "Bash", "summary": "rm -rf build", "destructive": "rm", "files": []}),
            1_400,
        );
        assert!(
            matches!(&fx[0], Effect::Waiting { category: Category::Approval, approval, .. } if approval == "c2")
        );
        assert!(t.sessions["s1"].waiting);
        assert_eq!(t.approvals[0].title, "Fix build");
        t.answered("c2");
        assert!(!t.sessions["s1"].waiting);
        t.add_cost("s1", 0.25, true);
        assert!((t.sessions["s1"].cost_usd - 0.25).abs() < 1e-9);
        t.observe("s1", &json!({"type": "fileChanged", "path": "a.rs"}), 1_450);
        t.observe("s1", &json!({"type": "fileChanged", "path": "a.rs"}), 1_460);
        t.observe("s1", &json!({"type": "error", "message": "boom"}), 1_500);
        let fx = t.observe("s1", &json!({"type": "done"}), 61_000);
        assert_eq!(
            fx,
            vec![Effect::Ended {
                session: "s1".into(),
                title: "Fix build".into(),
                secs: 60,
                root: "/p".into(),
                cost_usd: 0.25,
                cost_estimated: true,
                files: 1,
                error: Some("boom".into())
            }]
        );
        assert!(t.sessions.is_empty());
        assert_eq!(t.progress(), None);
    }

    #[test]
    fn questions_and_cleanup_on_done() {
        let mut t = Tracker::default();
        t.start("s1", "", "/p", 0);
        let fx = t.observe(
            "s1",
            &json!({"type": "confirm", "id": "q", "name": "AskUserQuestion", "summary": "Which?"}),
            1,
        );
        assert!(matches!(
            &fx[0],
            Effect::Waiting {
                category: Category::Question,
                ..
            }
        ));
        t.observe("s1", &json!({"type": "done"}), 2);
        assert!(
            t.approvals.is_empty(),
            "a finished turn leaves no approvals behind"
        );
    }
}
