//! Memory tools: `remember` and `forget` keep the auto-memory directory (topic files plus the
//! `MEMORY.md` index) without the two-step Write/Edit dance, and `conversation_search` finds
//! earlier Kor conversations. Everything stays on this computer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use kor_core::{ContentPart, Message, Role, ToolSpec};
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::memory::{self, Layout};

const KINDS: &[&str] = &["user", "feedback", "project", "reference"];
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 4000;
/// Newest conversations scanned by one search, and the largest transcript read.
const MAX_SESSIONS: usize = 300;
const MAX_SESSION_BYTES: u64 = 16 * 1024 * 1024;
const SNIPPET_CHARS: usize = 240;

fn parse<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolOutput> {
    serde_json::from_value(input).map_err(|e| ToolOutput::err(format!("invalid input: {e}")))
}

/// Topic file name from a title: ASCII letters and digits, the rest folded to `-`.
fn file_name(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' => 'i',
            'ö' => 'o',
            'ş' => 's',
            'ü' => 'u',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 60 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("note-{}.md", memory::unix_now())
    } else {
        format!("{slug}.md")
    }
}

/// The three tools; `remember`/`forget` only when auto memory is on (`layout` is set).
pub fn tools(
    layout: Option<Layout>,
    sessions: Option<PathBuf>,
    current: Option<PathBuf>,
) -> Vec<Arc<dyn Tool>> {
    let mut out: Vec<Arc<dyn Tool>> = Vec::new();
    if let Some(lay) = layout {
        let lay = Arc::new(lay);
        out.push(Arc::new(Remember(lay.clone())));
        out.push(Arc::new(Forget(lay)));
    }
    if let Some(dir) = sessions {
        out.push(Arc::new(ConversationSearch { dir, current }));
    }
    out
}

#[derive(Debug)]
pub struct Remember(Arc<Layout>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberIn {
    title: String,
    body: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[async_trait]
impl Tool for Remember {
    fn name(&self) -> &'static str {
        "remember"
    }
    fn rule_name(&self) -> &'static str {
        "remember"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Save a durable memory for future conversations in this project: a user preference, feedback on how to work, a project fact or decision, or a reference. It is written as a topic file and listed in the Auto memory section of later prompts. Saving the same title again updates it. Never store passwords, API keys or other secrets, and nothing that Grep or git can recover.".into(),
            input_schema: json!({"type": "object", "properties": {
                "title": {"type": "string", "maxLength": MAX_TITLE, "description": "Short title; also names the file"},
                "body": {"type": "string", "maxLength": MAX_BODY, "description": "The memory; the first line is shown in the index"},
                "type": {"type": "string", "enum": KINDS}
            }, "required": ["title", "body"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["title"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let a: RememberIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let title = a.title.split_whitespace().collect::<Vec<_>>().join(" ");
        let body = a.body.trim();
        if title.is_empty() || body.is_empty() {
            return ToolOutput::err("title and body must not be empty");
        }
        if title.chars().count() > MAX_TITLE || body.chars().count() > MAX_BODY {
            return ToolOutput::err(format!(
                "title is limited to {MAX_TITLE} and body to {MAX_BODY} characters"
            ));
        }
        let kind = kind_owned(a.kind.as_deref().unwrap_or("user"));
        if a.kind.as_deref().is_some_and(|k| !KINDS.contains(&k)) {
            return ToolOutput::err(format!("type must be one of {}", KINDS.join(", ")));
        }
        let hits = kor_tools::scan_secrets(&format!("{title}\n{body}"));
        if !hits.is_empty() {
            return ToolOutput::err(format!(
                "Refusing to store what looks like a secret ({}). Memories go into every later prompt.",
                hits.join(", ")
            ));
        }
        let name = file_name(&title);
        let lay = self.0.clone();
        let (owned_title, owned_body) = (title.clone(), body.to_owned());
        let file = name.clone();
        let res = tokio::task::spawn_blocking(move || {
            memory::save_memory_as(&lay, &file, &owned_title, kind, &owned_body)
        })
        .await;
        match res {
            Ok(Ok(())) => ToolOutput::ok(format!(
                "remembered `{title}` ({name}); it appears in the Auto memory section from the next turn"
            )),
            Ok(Err(e)) => ToolOutput::err(e.to_string()),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

/// `kind` validated against [`KINDS`], as a `'static` for the blocking task.
fn kind_owned(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|k| **k == kind)
        .copied()
        .unwrap_or("user")
}

#[derive(Debug)]
pub struct Forget(Arc<Layout>);

#[async_trait]
impl Tool for Forget {
    fn name(&self) -> &'static str {
        "forget"
    }
    fn rule_name(&self) -> &'static str {
        "forget"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Delete a saved memory: its topic file and its line in the MEMORY.md index. Use it when the user asks to forget something or a memory turned out wrong. `file` is the topic file name shown in the Auto memory index, such as `prefers-tabs.md`.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file": {"type": "string"}
            }, "required": ["file"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["file"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(file) = input["file"].as_str().map(str::to_owned) else {
            return ToolOutput::err("file is required");
        };
        let lay = self.0.clone();
        let f = file.clone();
        match tokio::task::spawn_blocking(move || memory::forget_memory(&lay, &f)).await {
            Ok(Ok(true)) => ToolOutput::ok(format!(
                "forgot {file}; it leaves the Auto memory section from the next turn"
            )),
            Ok(Ok(false)) => ToolOutput::err(format!("no memory named {file}")),
            Ok(Err(e)) => ToolOutput::err(e.to_string()),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct ConversationSearch {
    dir: PathBuf,
    current: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchIn {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

fn text_of(m: &Message) -> String {
    m.parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Around the first hit of any term, on one line.
fn snippet(text: &str, lower: &str, terms: &[String]) -> String {
    let at = terms
        .iter()
        .filter_map(|t| lower.find(t.as_str()))
        .min()
        .unwrap_or(0);
    // `lower` may differ in byte length from `text`; map through the char index.
    let char_at = lower[..at].chars().count();
    let start = char_at.saturating_sub(SNIPPET_CHARS / 3);
    let s: String = text
        .chars()
        .skip(start)
        .take(SNIPPET_CHARS)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let lead = if start > 0 { "…" } else { "" };
    format!("{lead}{}", s.trim())
}

struct Hit {
    score: usize,
    id: String,
    when: std::time::SystemTime,
    role: &'static str,
    snippet: String,
}

fn search(dir: &Path, current: Option<&Path>, query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.chars().count() >= 2)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter(|e| current.is_none_or(|c| e.path() != c))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let when = m.modified().ok()?;
            (m.len() <= MAX_SESSION_BYTES).then(|| (e.path(), when))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.truncate(MAX_SESSIONS);
    let mut hits = Vec::new();
    for (path, when) in files {
        let Ok(messages) = crate::session::load(&path) else {
            continue;
        };
        let best = messages
            .iter()
            .filter(|m| matches!(m.role, Role::User | Role::Assistant))
            .filter_map(|m| {
                let text = text_of(m);
                let lower = text.to_lowercase();
                let score = terms.iter().filter(|t| lower.contains(t.as_str())).count();
                (score > 0).then_some((score, m.role, text, lower))
            })
            .max_by_key(|(score, ..)| *score);
        if let Some((score, role, text, lower)) = best {
            hits.push(Hit {
                score,
                id: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                when,
                role: if role == Role::User {
                    "user"
                } else {
                    "assistant"
                },
                snippet: snippet(&text, &lower, &terms),
            });
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.when.cmp(&a.when)));
    hits.truncate(limit);
    hits
}

#[async_trait]
impl Tool for ConversationSearch {
    fn name(&self) -> &'static str {
        "conversation_search"
    }
    fn rule_name(&self) -> &'static str {
        "conversation_search"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Search earlier Kor conversations on this computer (not the current one) by keywords. Returns the best matching message of each conversation with its session id and date. Use it when the user refers to something discussed before (\"like last time\", \"the plan we made yesterday\").".into(),
            input_schema: json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "Keywords; conversations matching more of them rank first"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20}
            }, "required": ["query"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["query"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let a: SearchIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.query.trim().is_empty() || a.query.len() > 500 {
            return ToolOutput::err("query must be 1 to 500 characters");
        }
        let limit = a.limit.unwrap_or(8).clamp(1, 20);
        let (dir, current) = (self.dir.clone(), self.current.clone());
        let hits = match tokio::task::spawn_blocking(move || {
            search(&dir, current.as_deref(), &a.query, limit)
        })
        .await
        {
            Ok(h) => h,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        if hits.is_empty() {
            return ToolOutput::ok("No earlier conversation matches.");
        }
        let out: Vec<String> = hits
            .iter()
            .map(|h| {
                let days = h.when.elapsed().map_or(0, |d| d.as_secs() / 86_400);
                let ago = match days {
                    0 => "today".to_owned(),
                    1 => "yesterday".to_owned(),
                    n => format!("{n} days ago"),
                };
                format!("session {} ({ago}) [{}]: {}", h.id, h.role, h.snippet)
            })
            .collect();
        ToolOutput::ok(out.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe_slugs() {
        assert_eq!(
            file_name("Kullanıcı sekme ister!"),
            "kullanici-sekme-ister.md"
        );
        assert_eq!(file_name("../../etc/passwd"), "etc-passwd.md");
        assert!(file_name("???").starts_with("note-"));
    }

    #[tokio::test]
    async fn remember_then_forget() {
        let dir = tempfile::tempdir().unwrap();
        let lay = memory::layout(dir.path(), Path::new("/p"));
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let tools = tools(Some(lay.clone()), None, None);
        let (remember, forget) = (&tools[0], &tools[1]);

        let out = remember
            .call(json!({"title": "Prefers tabs", "body": "Use tabs, not spaces.", "type": "feedback"}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        let topic = std::fs::read_to_string(lay.auto_dir.join("prefers-tabs.md")).unwrap();
        assert!(topic.contains("type: feedback") && topic.contains("Use tabs"));
        assert!(
            memory::read_entrypoint(&lay)
                .unwrap()
                .contains("(prefers-tabs.md)")
        );

        let secret = remember
            .call(
                json!({"title": "key", "body": "aws AKIAIOSFODNN7EXAMPLE"}),
                &ctx,
            )
            .await;
        assert!(secret.is_error, "{}", secret.content);
        let bad = remember
            .call(json!({"title": "x", "body": "y", "type": "other"}), &ctx)
            .await;
        assert!(bad.is_error);

        let out = forget.call(json!({"file": "prefers-tabs.md"}), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(!lay.auto_dir.join("prefers-tabs.md").exists());
        assert!(
            !memory::read_entrypoint(&lay)
                .unwrap()
                .contains("prefers-tabs")
        );
        assert!(
            forget
                .call(json!({"file": "prefers-tabs.md"}), &ctx)
                .await
                .is_error
        );
        assert!(
            forget
                .call(json!({"file": "../MEMORY.md"}), &ctx)
                .await
                .is_error
        );
    }

    #[tokio::test]
    async fn conversation_search_ranks_and_skips_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let save = |id: &str, text: &str| {
            let p = dir.path().join(format!("{id}.jsonl"));
            crate::session::save(&p, &[Message::text(Role::User, text)]).unwrap();
            p
        };
        save("a", "Rust bellek aracı nasıl yazılır?");
        save("b", "Rust ile OCR ve bellek planı");
        let current = save("c", "Rust bellek planı (şu anki sohbet)");
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let tools = tools(None, Some(dir.path().to_owned()), Some(current));
        let out = tools[0]
            .call(json!({"query": "rust bellek planı"}), &ctx)
            .await;
        let lines: Vec<&str> = out.content.lines().collect();
        assert_eq!(lines.len(), 2, "{}", out.content);
        assert!(lines[0].starts_with("session b "), "{}", out.content);
        assert!(!out.content.contains("session c "));
        let none = tools[0].call(json!({"query": "zzz"}), &ctx).await;
        assert_eq!(none.content, "No earlier conversation matches.");
    }
}
