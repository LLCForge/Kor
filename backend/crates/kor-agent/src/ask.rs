//! Questions the agent puts to the user (§28.13) and the compaction preview (§28.1), ADR 0050.
//!
//! Both go through the [`crate::Approver`] of the surface (CLI line, TUI, desktop); an approver
//! that cannot ask (print mode, triggers, tests) keeps the defaults: the recommended option, and a
//! summary that keeps nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_core::{ContentPart, Message, Role};
use serde::Serialize;

/// A multiple-choice question (`AskUserQuestion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    /// 0-based recommended option; `Enter` takes it.
    pub default: usize,
}

/// The user's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// 0-based option.
    pub index: usize,
    /// "Always do it this way": save the answer as a project rule in `.kor/rules/`.
    pub remember: bool,
}

/// One part of the conversation an automatic summary would replace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompactItem {
    /// Index in the conversation.
    pub index: usize,
    /// `user`, `assistant` or `tool`.
    pub kind: &'static str,
    /// First line (tool: `Name target`).
    pub label: String,
    /// ≈ tokens.
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactChoice {
    /// Summarize; the listed item indices stay verbatim after the summary.
    Proceed { keep: Vec<usize> },
    /// Do not summarize now.
    Cancel,
}

impl Default for CompactChoice {
    fn default() -> Self {
        Self::Proceed { keep: Vec::new() }
    }
}

const LABEL_CHARS: usize = 80;

fn first_line(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(LABEL_CHARS).collect();
    if line.chars().count() > LABEL_CHARS {
        out.push('…');
    }
    out
}

/// What a summary of `messages` would replace, one item per message (tool results labelled with
/// the call that produced them), largest-first order kept as in the conversation.
pub fn compact_items(messages: &[Message]) -> Vec<CompactItem> {
    let mut calls: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for (index, m) in messages.iter().enumerate() {
        let tokens = crate::compact::estimate_tokens(std::slice::from_ref(m));
        let mut tool = None;
        let mut text = String::new();
        for p in &m.parts {
            match p {
                ContentPart::ToolUse { id, name, input } => {
                    let target = ["file_path", "path", "command", "pattern", "url"]
                        .iter()
                        .find_map(|k| input[*k].as_str())
                        .unwrap_or("");
                    calls.insert(id.as_str(), format!("{name} {target}").trim().to_owned());
                }
                ContentPart::ToolResult { tool_use_id, .. } if tool.is_none() => {
                    tool = Some(
                        calls
                            .get(tool_use_id.as_str())
                            .cloned()
                            .unwrap_or_else(|| "tool output".into()),
                    );
                }
                ContentPart::Text { text: t } if text.is_empty() => t.clone_into(&mut text),
                _ => {}
            }
        }
        let (kind, label) = match (tool, m.role) {
            (Some(t), _) => ("tool", first_line(&t)),
            (None, Role::User) => ("user", first_line(&text)),
            (None, _) => (
                "assistant",
                if text.is_empty() {
                    calls.values().last().map(|c| first_line(c)).unwrap_or_default()
                } else {
                    first_line(&text)
                },
            ),
        };
        out.push(CompactItem {
            index,
            kind,
            label,
            tokens,
        });
    }
    out
}

/// The kept items as one user message placed after the summary (tool results lose their call
/// pairing, so they are kept as text).
pub fn kept_message(messages: &[Message], keep: &[usize]) -> Option<Message> {
    let items = compact_items(messages);
    let mut body = String::new();
    for &i in keep {
        let (Some(m), Some(item)) = (messages.get(i), items.get(i)) else {
            continue;
        };
        let text: Vec<String> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.clone()),
                ContentPart::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        if text.is_empty() {
            continue;
        }
        let _ = write!(body, "\n--- {} #{i}: {} ---\n{}\n", item.kind, item.label, text.join("\n"));
    }
    (!body.is_empty()).then(|| {
        Message::text(
            Role::User,
            format!("[kept verbatim by the user when the conversation was summarized]{body}"),
        )
    })
}

/// §28.1 "remove from context": the item's text and tool output are replaced by a short marker
/// (calls keep their results, so the conversation stays valid). Returns `false` for a bad index.
pub fn drop_item(messages: &mut [Message], index: usize) -> bool {
    let Some(label) = compact_items(messages).get(index).map(|i| i.label.clone()) else {
        return false;
    };
    let marker = format!("[removed from context by the user: {label}]");
    let Some(m) = messages.get_mut(index) else { return false };
    for p in &mut m.parts {
        match p {
            ContentPart::Text { text } => text.clone_from(&marker),
            ContentPart::ToolResult { content, .. } => content.clone_from(&marker),
            _ => {}
        }
    }
    true
}

/// "Always do it this way": the question and the chosen answer as `.kor/rules/<slug>.md`.
///
/// # Errors
/// If the rules folder cannot be written.
pub fn save_rule(root: &Path, question: &str, answer: &str) -> std::io::Result<PathBuf> {
    let dir = root.join(".kor").join("rules");
    std::fs::create_dir_all(&dir)?;
    let base = kor_tools::plans::slug(question);
    let mut path = dir.join(format!("{base}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.md"));
        n += 1;
    }
    let one = |s: &str| s.replace(['\r', '\n'], " ");
    std::fs::write(
        &path,
        format!(
            "---\ndescription: {}\n---\n\nWhen this comes up — \"{}\" — always: {}.\n",
            one(question),
            one(question).trim(),
            one(answer).trim()
        ),
    )?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msgs() -> Vec<Message> {
        vec![
            Message::text(Role::User, "fix the login bug\nmore"),
            Message {
                role: Role::Assistant,
                parts: vec![ContentPart::ToolUse {
                    id: "t1".into(),
                    name: "Read".into(),
                    input: json!({"file_path": "src/login.rs"}),
                }],
            },
            Message {
                role: Role::User,
                parts: vec![ContentPart::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "fn login() {}".into(),
                    is_error: false,
                }],
            },
        ]
    }

    #[test]
    fn items_label_tool_outputs_and_kept_text_survives() {
        let m = msgs();
        let items = compact_items(&m);
        assert_eq!(items.len(), 3);
        assert_eq!((items[0].kind, items[0].label.as_str()), ("user", "fix the login bug"));
        assert_eq!((items[2].kind, items[2].label.as_str()), ("tool", "Read src/login.rs"));
        let kept = kept_message(&m, &[2]).unwrap().plain_text();
        assert!(kept.contains("Read src/login.rs") && kept.contains("fn login() {}"), "{kept}");
        assert!(kept_message(&m, &[1]).is_none(), "a bare tool call has no text to keep");
    }

    #[test]
    fn dropping_an_item_keeps_the_pairing() {
        let mut m = msgs();
        let before = crate::compact::estimate_tokens(&m);
        assert!(drop_item(&mut m, 2));
        assert!(!drop_item(&mut m, 9));
        let ContentPart::ToolResult { content, tool_use_id, .. } = &m[2].parts[0] else { panic!() };
        assert_eq!(tool_use_id, "t1");
        assert!(content.starts_with("[removed from context by the user: Read src/login.rs"));
        assert!(crate::compact::estimate_tokens(&m) <= before + 20);
    }

    #[test]
    fn answer_becomes_a_rule() {
        let dir = tempfile::tempdir().unwrap();
        let p = save_rule(dir.path(), "Which test runner?", "vitest").unwrap();
        assert!(p.starts_with(dir.path().join(".kor/rules")));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("always: vitest"), "{text}");
        let q = save_rule(dir.path(), "Which test runner?", "jest").unwrap();
        assert_ne!(p, q);
    }
}
