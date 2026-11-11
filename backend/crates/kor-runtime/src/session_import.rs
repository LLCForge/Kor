//! Bring chats from other tools into Kor's session list (read only: their files are never
//! written). Claude Code keeps one JSON Lines file per chat under `~/.claude/projects/<folder>/`;
//! Cursor keeps its chats in the `cursorDiskKV` table of `globalStorage/state.vscdb`
//! (`bubbleId:<chat>:<message>` rows, `type` 1 = user, 2 = assistant). Only the conversation text
//! is imported; tool calls and their output stay behind. Importing twice adds nothing new.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kor_core::{Message, Role};
use serde_json::Value;

/// One message's text is cut here; a pasted log does not need to come along whole.
const MAX_TEXT: usize = 20_000;
/// Claude Code files above this size are skipped (they are mostly tool output).
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// A chat ready to save: a stable id (so a second import skips it) and its messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub id: String,
    pub messages: Vec<Message>,
}

fn clip(s: &str) -> String {
    match s.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// Claude Code's folder name for a project: every character that is not a letter or digit
/// becomes `-` (`C:\Users\x\proj` → `C--Users-x-proj`).
pub fn claude_code_folder(root: &Path) -> String {
    let s = root.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Chat files for `root` (or for every project with `all`), oldest first.
pub fn claude_code_files(home: &Path, root: &Path, all: bool) -> Vec<PathBuf> {
    let projects = home.join(".claude").join("projects");
    let want = claude_code_folder(root).to_lowercase();
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for dir in std::fs::read_dir(&projects).into_iter().flatten().flatten() {
        if !all && dir.file_name().to_string_lossy().to_lowercase() != want {
            continue;
        }
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = f.path();
            let Ok(meta) = f.metadata() else { continue };
            if p.extension().is_some_and(|e| e == "jsonl")
                && meta.is_file()
                && meta.len() <= MAX_FILE
            {
                out.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), p));
            }
        }
    }
    out.sort();
    out.into_iter().map(|(_, p)| p).collect()
}

/// The visible text of a Claude Code `message.content` (a string or text blocks).
fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The conversation in one Claude Code chat file: user and assistant text, without side
/// conversations of sub-agents, meta lines or tool results.
pub fn parse_claude_code(reader: impl std::io::BufRead) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let role = match v.get("type").and_then(Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true)
            || v.get("isMeta").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let text = v
            .get("message")
            .and_then(|m| m.get("content"))
            .map(content_text)
            .unwrap_or_default();
        let text = text.trim();
        // Slash-command wrappers and interrupt markers are not conversation.
        if text.is_empty() || text.starts_with("<command-") || text.starts_with("<local-command") {
            continue;
        }
        // An assistant answer arrives in several lines (one per block): join them.
        if let Some(last) = out.last_mut()
            && last.role == role
            && role == Role::Assistant
            && let Some(kor_core::ContentPart::Text { text: t }) = last.parts.first_mut()
        {
            t.push_str("\n\n");
            t.push_str(&clip(text));
            continue;
        }
        out.push(Message::text(role, clip(text)));
    }
    out
}

/// Every Claude Code chat for `root` (or all projects).
pub fn claude_code(home: &Path, root: &Path, all: bool) -> Vec<Chat> {
    claude_code_files(home, root, all)
        .into_iter()
        .filter_map(|p| {
            let f = std::fs::File::open(&p).ok()?;
            let messages = parse_claude_code(std::io::BufReader::new(f));
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            (!messages.is_empty()).then(|| Chat {
                id: format!("claude-{}", safe(&stem)),
                messages,
            })
        })
        .collect()
}

fn safe(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect()
}

/// Cursor's chat database (`<config>/Cursor/User/globalStorage/state.vscdb`).
pub fn cursor_db(app_config: &Path) -> PathBuf {
    app_config
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

/// Every Cursor chat in `db`, read only. A database Cursor holds open is read as it is.
///
/// # Errors
/// When the database cannot be opened or read.
pub fn cursor(db: &Path) -> Result<Vec<Chat>, String> {
    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%'")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| e.to_string())?;
    // chat id → (time, order seen, role, text)
    let mut chats: BTreeMap<String, Vec<(String, usize, Role, String)>> = BTreeMap::new();
    for (n, row) in rows.flatten().enumerate() {
        let (key, Some(value)) = row else { continue };
        let Some(chat) = key.split(':').nth(1) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&value) else {
            continue;
        };
        let role = match v.get("type").and_then(Value::as_i64) {
            Some(1) => Role::User,
            Some(2) => Role::Assistant,
            _ => continue,
        };
        let text = v.get("text").and_then(Value::as_str).unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        let at = match v.get("createdAt") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(x)) => format!("{x:0>20}"),
            _ => String::new(),
        };
        chats
            .entry(chat.to_owned())
            .or_default()
            .push((at, n, role, clip(text)));
    }
    Ok(chats
        .into_iter()
        .map(|(id, mut msgs)| {
            msgs.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
            Chat {
                id: format!("cursor-{}", safe(&id)),
                messages: msgs
                    .into_iter()
                    .map(|(_, _, r, t)| Message::text(r, t))
                    .collect(),
            }
        })
        .collect())
}

/// Save chats that are not in `sessions_dir` yet; returns how many were added.
///
/// # Errors
/// When a session file cannot be written.
pub fn save_new(sessions_dir: &Path, chats: &[Chat]) -> std::io::Result<usize> {
    std::fs::create_dir_all(sessions_dir)?;
    let mut n = 0;
    for c in chats {
        let path = sessions_dir.join(format!("{}.jsonl", c.id));
        if path.exists() || c.messages.is_empty() {
            continue;
        }
        kor_agent::session::save(&path, &c.messages)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_chat_keeps_only_the_conversation() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"explain the repo"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"It is a CLI."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"..."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"And a desktop app."}]}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"sub-agent prompt"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"summary","summary":"x"}"#,
            "not json",
        ]
        .join("\n");
        let m = parse_claude_code(std::io::Cursor::new(lines));
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], Message::text(Role::User, "explain the repo"));
        assert_eq!(
            m[1],
            Message::text(Role::Assistant, "It is a CLI.\n\nAnd a desktop app.")
        );
    }

    #[test]
    fn finds_the_project_folder_and_saves_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        let root = Path::new("/work/my proj");
        assert_eq!(claude_code_folder(root), "-work-my-proj");
        let p = home.join(".claude/projects/-work-my-proj");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("abc-1.jsonl"),
            r#"{"type":"user","message":{"content":"hi"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".claude/projects/other")).unwrap();
        std::fs::write(
            home.join(".claude/projects/other/z.jsonl"),
            r#"{"type":"user","message":{"content":"x"}}"#,
        )
        .unwrap();
        let chats = claude_code(&home, root, false);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].id, "claude-abc-1");
        assert_eq!(claude_code(&home, root, true).len(), 2);
        let sd = dir.path().join("sessions");
        assert_eq!(save_new(&sd, &chats).unwrap(), 1);
        assert_eq!(
            save_new(&sd, &chats).unwrap(),
            0,
            "a second import adds nothing"
        );
    }

    #[test]
    fn cursor_bubbles_become_ordered_chats() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
        )
        .unwrap();
        for (k, v) in [
            (
                "bubbleId:c1:b2",
                r#"{"type":2,"text":"answer","createdAt":"2026-01-01T00:00:02Z"}"#,
            ),
            (
                "bubbleId:c1:b1",
                r#"{"type":1,"text":"question","createdAt":"2026-01-01T00:00:01Z"}"#,
            ),
            (
                "bubbleId:c1:b3",
                r#"{"type":2,"text":"","createdAt":"2026-01-01T00:00:03Z"}"#,
            ),
            ("composerData:c1", r#"{"composerId":"c1"}"#),
        ] {
            c.execute("INSERT INTO cursorDiskKV VALUES (?1, ?2)", [k, v])
                .unwrap();
        }
        drop(c);
        let chats = cursor(&db).unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].id, "cursor-c1");
        assert_eq!(
            chats[0].messages,
            [
                Message::text(Role::User, "question"),
                Message::text(Role::Assistant, "answer")
            ]
        );
    }
}
