//! Knowledge base in the agent loop (§31.3). At task start the records most relevant to the
//! prompt (FTS fused with local vector similarity when the embedding model is installed, same
//! project ranked higher) go into the system prompt as their own context
//! category, within `[memory] max_tokens`. At task end a short "what we learned" record is
//! written from the turn's own events (files, commands, errors, final answer): deterministic,
//! no model call, nothing leaves the machine.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use kor_core::{ContentPart, Message, Role};
use kor_memory::{Hit, Knowledge, ProjectMemory, Record, SessionSummary};

use crate::agent::{Agent, StopReason};

const HEADING: &str = "\n# Recalled knowledge\n";
/// Records considered for injection.
const TOP: usize = 5;
const LINE_MAX: usize = 400;
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Delete"];
const MAX_ITEMS: usize = 10;
const SUMMARY_LINES: usize = 6;

/// Recall settings of a session and the knowledge base it uses.
pub struct Recall {
    pub kb: Arc<Knowledge>,
    /// Project memory folder for session summaries (`sessions/`); `None` skips them.
    pub memory: Option<ProjectMemory>,
    /// [`kor_memory::project_key`] of the session's project.
    pub project: String,
    pub inject: bool,
    pub max_tokens: u64,
    pub learn: bool,
    injected: Option<String>,
}

impl std::fmt::Debug for Recall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recall")
            .field("project", &self.project)
            .field("inject", &self.inject)
            .field("max_tokens", &self.max_tokens)
            .field("learn", &self.learn)
            .finish_non_exhaustive()
    }
}

impl Recall {
    pub fn new(
        kb: Arc<Knowledge>,
        memory: Option<ProjectMemory>,
        root: &Path,
        inject: bool,
        max_tokens: u64,
        learn: bool,
    ) -> Self {
        Self {
            kb,
            memory,
            project: kor_memory::project_key(root),
            inject,
            max_tokens,
            learn,
            injected: None,
        }
    }

    /// `<home>/knowledge.sqlite` and the project memory of `root`; `None` when both features are
    /// off or the database cannot be opened (the agent then runs without it).
    pub fn open(
        home: Option<&Path>,
        root: &Path,
        inject: bool,
        max_tokens: u64,
        learn: bool,
    ) -> Option<Self> {
        if !inject && !learn {
            return None;
        }
        let home = home?;
        // With the local embedding model when installed: hybrid recall, background embedding.
        let kb = kor_memory::open_knowledge(home).ok()?;
        Some(Self::new(
            Arc::new(kb),
            Some(ProjectMemory::new(home, root)),
            root,
            inject,
            max_tokens,
            learn,
        ))
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn project_name(key: &str) -> &str {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key)
}

/// The prompt section for `hits`, at most `max_tokens` (≈ 4 bytes each); empty when none fit.
pub fn section(hits: &[Hit], max_tokens: u64) -> String {
    let budget = usize::try_from(max_tokens.saturating_mul(4)).unwrap_or(usize::MAX);
    let mut out = format!(
        "{HEADING}Earlier Kor work on this computer that may help (local knowledge base; verify before relying on it):\n"
    );
    let head = out.len();
    for h in hits {
        let r = &h.record;
        let date = kor_memory::kb::date(r.ts);
        let mut line = format!(
            "- #{} ({}, {date}) {}: {}",
            r.id,
            project_name(&r.project),
            r.task,
            r.summary.lines().next().unwrap_or_default()
        );
        if !r.errors.is_empty() {
            let _ = write!(line, " | errors: {}", r.errors.join("; "));
        }
        if !r.files.is_empty() {
            let _ = write!(line, " | files: {}", r.files.join(", "));
        }
        let line = format!("{}\n", clip(&line, LINE_MAX));
        if out.len() + line.len() > budget {
            break;
        }
        out.push_str(&line);
    }
    if out.len() == head {
        String::new()
    } else {
        out
    }
}

/// Replace the previous task's recalled section with records relevant to `prompt`.
pub(crate) fn before_task(agent: &mut Agent, prompt: &str) {
    let Some(r) = agent.context.recall.as_mut() else {
        return;
    };
    if let Some(prev) = r.injected.take()
        && let Some(i) = agent.system.find(&prev)
    {
        agent.system.replace_range(i..i + prev.len(), "");
    }
    agent.context.memory_tokens = 0;
    if !r.inject || r.max_tokens == 0 {
        return;
    }
    // shortcut: synchronous on the async task: FTS is a few ms (ADR 0021), the local query
    // embedding ~20-40 ms more when the model is installed (ADR 0023).
    let Ok(hits) = r.kb.recall(prompt, None, Some(&r.project), TOP) else {
        return;
    };
    // §27.15: count what was shown to the model (the weekly "most referenced notes").
    if let Some(home) = r.kb.path().parent() {
        let ids: Vec<i64> = hits.iter().map(|h| h.record.id).collect();
        let _ = kor_memory::digest::record_uses(home, &ids);
    }
    let text = section(&hits, r.max_tokens);
    if text.is_empty() {
        return;
    }
    agent.context.memory_tokens = crate::context::tokens(text.len());
    agent.system.push_str(&text);
    r.injected = Some(text);
}

/// `Some(true)` succeeded, `Some(false)` failed; `None` = nothing to learn from (cancelled,
/// blocked, stopped by the budget).
pub(crate) fn outcome(
    result: &Result<StopReason, kor_core::ProviderError>,
    verify_failed: bool,
) -> Option<bool> {
    match result {
        Ok(StopReason::Done(_)) => Some(!verify_failed),
        Ok(StopReason::MaxTurns) | Err(_) => Some(false),
        Ok(_) => None,
    }
}

/// The "what we learned" record of `messages` (the turn), or `None` for a turn without tool
/// calls (plain conversation is not knowledge).
pub fn learned(messages: &[Message], prompt: &str, project: &str, ok: bool) -> Option<Record> {
    let mut r = Record {
        project: project.to_owned(),
        kind: "task".into(),
        task: clip(
            prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or(""),
            200,
        ),
        ..Record::default()
    };
    let mut calls = 0;
    let mut last_text = "";
    for m in messages {
        for p in &m.parts {
            match p {
                ContentPart::ToolUse { name, input, .. } => {
                    calls += 1;
                    if WRITE_TOOLS.contains(&name.as_str()) {
                        let path = ["file_path", "notebook_path", "path"]
                            .iter()
                            .find_map(|k| input[*k].as_str());
                        if let Some(path) = path
                            && !r.files.iter().any(|f| f == path)
                            && r.files.len() < MAX_ITEMS
                        {
                            r.files.push(path.to_owned());
                        }
                    } else if name == "Bash"
                        && let Some(cmd) = input["command"].as_str()
                        && r.commands.len() < MAX_ITEMS
                    {
                        r.commands.push(clip(cmd, 200));
                    }
                }
                ContentPart::ToolResult {
                    content, is_error, ..
                } if *is_error && r.errors.len() < MAX_ITEMS => {
                    let first = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                    r.errors.push(clip(first, 200));
                }
                ContentPart::Text { text }
                    if m.role == Role::Assistant && !text.trim().is_empty() =>
                {
                    last_text = text;
                }
                _ => {}
            }
        }
    }
    if calls == 0 {
        return None;
    }
    let lines: Vec<String> = last_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(SUMMARY_LINES)
        .map(|l| clip(l, 200))
        .collect();
    r.summary = format!(
        "{}{}",
        if ok { "" } else { "(not finished) " },
        lines.join("\n")
    );
    Some(r)
}

/// Write the turn's record and a session summary line (best effort: a full disk or a locked
/// database never stops the agent).
pub(crate) fn after_task(agent: &Agent, prompt: &str, start: usize, ok: Option<bool>) {
    let (Some(r), Some(ok)) = (agent.context.recall.as_ref(), ok) else {
        return;
    };
    if !r.learn {
        return;
    }
    let turn = agent.messages.get(start..).unwrap_or_default();
    let Some(mut rec) = learned(turn, prompt, &r.project, ok) else {
        return;
    };
    // Keys this session knows are removed too (the database also masks known key shapes).
    let red = |s: &str| agent.redactor.redact(s).into_owned();
    rec.task = red(&rec.task);
    rec.summary = red(&rec.summary);
    for list in [&mut rec.commands, &mut rec.errors] {
        for item in list.iter_mut() {
            *item = red(item);
        }
    }
    if let Some(mem) = &r.memory {
        let result = format!(
            "{} — {} file(s) changed. {}",
            if ok { "ok" } else { "failed" },
            rec.files.len(),
            rec.summary.lines().next().unwrap_or_default()
        );
        let _ = mem.add_session(&SessionSummary {
            date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            task: rec.task.clone(),
            result,
        });
    }
    let _ = r.kb.record(rec);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn() -> Vec<Message> {
        vec![
            Message::text(Role::User, "Fix the login bug"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    ContentPart::ToolUse {
                        id: "1".into(),
                        name: "Edit".into(),
                        input: json!({"file_path": "src/auth.rs"}),
                    },
                    ContentPart::ToolUse {
                        id: "2".into(),
                        name: "Bash".into(),
                        input: json!({"command": "cargo test -p auth"}),
                    },
                ],
            },
            Message {
                role: Role::User,
                parts: vec![ContentPart::ToolResult {
                    tool_use_id: "2".into(),
                    content: "\nerror[E0502]: cannot borrow\nmore".into(),
                    is_error: true,
                }],
            },
            Message::text(
                Role::Assistant,
                "Fixed: the token is cloned before refresh.\n\nTests pass.",
            ),
        ]
    }

    #[test]
    fn learned_record_comes_from_the_turn() {
        let r = learned(&turn(), "Fix the login bug\nmore detail", "/p", true).unwrap();
        assert_eq!(r.task, "Fix the login bug");
        assert_eq!(r.files, ["src/auth.rs"]);
        assert_eq!(r.commands, ["cargo test -p auth"]);
        assert_eq!(r.errors, ["error[E0502]: cannot borrow"]);
        assert_eq!(
            r.summary,
            "Fixed: the token is cloned before refresh.\nTests pass."
        );
        assert!(learned(&[Message::text(Role::Assistant, "hi")], "hi", "/p", true).is_none());
    }

    #[test]
    fn section_respects_the_budget() {
        let hit = |id: i64| Hit {
            score: 1.0,
            record: Record {
                id,
                ts: 1_700_000_000,
                project: "C:/work/shop".into(),
                task: "iyzico webhook".into(),
                summary: "Verify the signature first.\nsecond line".into(),
                files: vec!["pay.rs".into()],
                ..Record::default()
            },
        };
        let s = section(&[hit(1), hit(2)], 1000);
        assert!(s.starts_with(HEADING));
        assert!(s.contains("- #1 (shop, 2023-11-14) iyzico webhook: Verify the signature first. | files: pay.rs"), "{s}");
        assert!(s.contains("- #2 "));
        assert!(!s.contains("second line"));
        let small = section(&[hit(1), hit(2)], 60);
        assert!(small.contains("#1") && !small.contains("#2"), "{small}");
        assert!(section(&[hit(1)], 10).is_empty());
    }
}
