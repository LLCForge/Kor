//! Context Budget Manager (§5.3): what each request spends of the model's context window, by
//! category, with the same ≈ 4-bytes-per-token estimate the auto-compact uses. Every request emits
//! an [`AgentEvent::Context`] and, when a log path is set, appends one JSON line (local only).

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_core::ContentPart;
use kor_providers::{ChatRequest, Provider};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::AgentEvent;

/// Tools whose results are file contents ("active files" rather than "tool results").
const FILE_TOOLS: &[&str] = &["Read"];
/// The log is rotated to `<name>.1` past this size, so it never grows without bound.
const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// Estimated tokens for `bytes` of text (≈ 4 bytes per token, as in `compact::estimate_tokens`).
pub fn tokens(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(4)
}

/// Token use of one request. Every figure is an estimate (`estimated`), shown with "≈".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Breakdown {
    /// Unix seconds.
    pub ts: u64,
    pub model: String,
    pub window: u64,
    pub estimated: bool,
    /// Kor's own instructions and environment facts.
    pub system: u64,
    /// Always-on project/user rules (AGENTS.md, CLAUDE.md, ...).
    pub rules: u64,
    /// Repository signature map.
    pub repo_map: u64,
    /// Records recalled from the knowledge base for this task (§31.3).
    pub memory: u64,
    /// Tool schemas sent with the request.
    pub tools: u64,
    /// File contents returned by `Read`.
    pub active_files: u64,
    /// User and assistant turns, tool calls.
    pub history: u64,
    /// Other tool output.
    pub tool_results: u64,
    /// Category that passed the warning threshold on this request (first time only).
    pub warning: Option<String>,
}

impl Breakdown {
    /// `(category key, tokens)` in display order.
    pub fn parts(&self) -> [(&'static str, u64); 8] {
        [
            ("system", self.system),
            ("rules", self.rules),
            ("repo_map", self.repo_map),
            ("memory", self.memory),
            ("tools", self.tools),
            ("active_files", self.active_files),
            ("history", self.history),
            ("tool_results", self.tool_results),
        ]
    }

    pub fn total(&self) -> u64 {
        self.parts().iter().map(|(_, t)| t).sum()
    }

    /// Whole percent of the context window taken by `tokens`.
    pub fn percent(&self, tokens: u64) -> u64 {
        if self.window == 0 {
            0
        } else {
            tokens.saturating_mul(100) / self.window
        }
    }

    /// The largest category above `warn_percent` of the window; `0` never warns.
    pub fn dominant(&self, warn_percent: u8) -> Option<(&'static str, u64)> {
        if warn_percent == 0 || self.window == 0 {
            return None;
        }
        self.parts()
            .into_iter()
            .filter(|(_, t)| t.saturating_mul(100) > u64::from(warn_percent) * self.window)
            .max_by_key(|(_, t)| *t)
            .map(|(k, t)| (k, self.percent(t)))
    }
}

/// Context settings of a session (`[context]` in config) and its last breakdown.
#[derive(Default)]
pub struct Budget {
    /// Warn when one category passes this percent of the window; 0 never warns.
    pub warn_percent: u8,
    /// Summarize the conversation automatically past this percent of the window; 0 leaves only
    /// the built-in auto-compact (window minus `AUTOCOMPACT_BUFFER_TOKENS`).
    pub auto_percent: u8,
    /// Provider and model that write summaries; `None` uses the chat model.
    pub summarizer: Option<(Arc<dyn Provider>, String)>,
    /// Tokens of the system prompt that are rules and repo map (measured when it was built).
    pub rules_tokens: u64,
    pub repo_map_tokens: u64,
    /// Tokens of the system prompt that are recalled knowledge (set by [`crate::recall`]).
    pub memory_tokens: u64,
    /// Knowledge base recall and "what we learned" records (§31.3); `None` = off.
    pub recall: Option<crate::recall::Recall>,
    /// JSON-lines log of every breakdown; `None` keeps nothing on disk.
    pub log: Option<PathBuf>,
    pub last: Option<Breakdown>,
    /// A warning was shown since the last summary.
    pub warned: bool,
    /// §28.1 `[context] compact_confirm`: ask before an automatic summary.
    pub compact_confirm: bool,
    /// The user cancelled a summary: not asked again below this many ≈ tokens.
    pub compact_snooze: u64,
    /// §28.1 items the user pinned (conversation indices): kept verbatim by the next summary.
    pub pinned: Vec<usize>,
}

impl std::fmt::Debug for Budget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Budget")
            .field("warn_percent", &self.warn_percent)
            .field("auto_percent", &self.auto_percent)
            .field("summarizer", &self.summarizer.as_ref().map(|(_, m)| m))
            .field("log", &self.log)
            .finish_non_exhaustive()
    }
}

/// Break `req` down by category.
pub fn measure(req: &ChatRequest, window: u64, budget: &Budget) -> Breakdown {
    let names: HashMap<&str, &str> = req
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    let (mut files, mut results, mut history) = (0usize, 0usize, 0usize);
    // shortcut: text-protocol tool results arrive as plain user text and count as history.
    for p in req.messages.iter().flat_map(|m| &m.parts) {
        match p {
            ContentPart::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                if names
                    .get(tool_use_id.as_str())
                    .is_some_and(|n| FILE_TOOLS.contains(n))
                {
                    files += content.len();
                } else {
                    results += content.len();
                }
            }
            ContentPart::Text { text } | ContentPart::Thinking { text, .. } => {
                history += text.len();
            }
            ContentPart::ToolUse { name, input, .. } => {
                history += name.len() + input.to_string().len();
            }
            ContentPart::Image { .. } | ContentPart::File { .. } => history += 256,
        }
    }
    let tool_bytes: usize = req
        .tools
        .iter()
        .map(|t| serde_json::to_string(t).map_or(0, |s| s.len()))
        .sum();
    let system = tokens(req.system.as_deref().map_or(0, str::len));
    let rules = budget.rules_tokens.min(system);
    let repo_map = budget.repo_map_tokens.min(system - rules);
    let memory = budget.memory_tokens.min(system - rules - repo_map);
    Breakdown {
        ts: crate::memory::unix_now(),
        model: req.model.clone(),
        window,
        estimated: true,
        system: system - rules - repo_map - memory,
        rules,
        repo_map,
        memory,
        tools: tokens(tool_bytes),
        active_files: tokens(files),
        history: tokens(history),
        tool_results: tokens(results),
        warning: None,
    }
}

impl Budget {
    /// Mark a first-time warning, emit the breakdown, log it and keep it as `last`.
    pub(crate) fn observe(&mut self, mut b: Breakdown, events: &UnboundedSender<AgentEvent>) {
        if !self.warned
            && let Some((cat, _)) = b.dominant(self.warn_percent)
        {
            self.warned = true;
            b.warning = Some(cat.to_owned());
        }
        if let Some(path) = &self.log {
            // Logging is best effort; a full disk must not stop the agent.
            let _ = append_log(path, &b);
        }
        let _ = events.send(AgentEvent::Context(b.clone()));
        self.last = Some(b);
    }
}

/// Append one breakdown as a JSON line, rotating the file past `LOG_MAX_BYTES`.
pub fn append_log(path: &Path, b: &Breakdown) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        std::fs::rename(path, path.with_extension("jsonl.1"))?;
    }
    let mut line = serde_json::to_string(b).map_err(std::io::Error::other)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// The last `limit` breakdowns of the log, oldest first; a missing file is empty.
pub fn read_log(path: &Path, limit: usize) -> Vec<Breakdown> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<Breakdown> = text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(limit)
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_core::{Message, Role, ToolSpec};
    use serde_json::json;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            system: Some("s".repeat(400)),
            messages: vec![
                Message::text(Role::User, "u".repeat(40)),
                Message {
                    role: Role::Assistant,
                    parts: vec![
                        ContentPart::ToolUse {
                            id: "a".into(),
                            name: "Read".into(),
                            input: json!({}),
                        },
                        ContentPart::ToolUse {
                            id: "b".into(),
                            name: "Bash".into(),
                            input: json!({}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    parts: vec![
                        ContentPart::ToolResult {
                            tool_use_id: "a".into(),
                            content: "f".repeat(800),
                            is_error: false,
                        },
                        ContentPart::ToolResult {
                            tool_use_id: "b".into(),
                            content: "o".repeat(80),
                            is_error: false,
                        },
                    ],
                },
            ],
            tools: vec![ToolSpec {
                name: "Read".into(),
                description: String::new(),
                input_schema: json!({}),
            }],
            ..ChatRequest::default()
        }
    }

    #[test]
    fn categories_split_the_request() {
        let budget = Budget {
            rules_tokens: 30,
            repo_map_tokens: 20,
            ..Budget::default()
        };
        let b = measure(&req(), 1000, &budget);
        assert_eq!((b.system, b.rules, b.repo_map), (50, 30, 20));
        assert_eq!(b.active_files, 200);
        assert_eq!(b.tool_results, 20);
        // 40 bytes of text + "Read{}" + "Bash{}".
        assert_eq!(b.history, tokens(52));
        assert!(b.tools > 0 && b.estimated);
        assert_eq!(b.dominant(15), Some(("active_files", 20)));
        assert_eq!(b.dominant(25), None);
        assert_eq!(b.dominant(0), None);
    }

    #[test]
    fn warns_once_and_logs_locally() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("context.jsonl");
        let mut budget = Budget {
            warn_percent: 10,
            log: Some(log.clone()),
            ..Budget::default()
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for _ in 0..2 {
            let b = measure(&req(), 1000, &budget);
            budget.observe(b, &tx);
        }
        let first = rx.try_recv().unwrap();
        let second = rx.try_recv().unwrap();
        assert!(
            matches!(first, AgentEvent::Context(ref b) if b.warning.as_deref() == Some("active_files"))
        );
        assert!(matches!(second, AgentEvent::Context(ref b) if b.warning.is_none()));
        let logged = read_log(&log, 10);
        assert_eq!(logged.len(), 2);
        assert_eq!(read_log(&log, 1).len(), 1);
        assert_eq!(logged[0].active_files, 200);
    }
}
