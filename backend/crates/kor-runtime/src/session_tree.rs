//! Session tree shared by the CLI and the desktop (Master Build Prompt §28.6, ADR 0050).
//!
//! Transcripts stay in `sessions/*.jsonl`; folders (the desktop's sidebar sections), pin,
//! archive, title, tags and colour live in the desktop's `desktop.json` (`kor_home()`), so
//! `kor sessions mv|pin|tag` and the sidebar change the same records. This module edits only the
//! `sessions` and `sections` keys and keeps every other key of the file as it is; writes are
//! atomic and serialized with a lock file.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_core::{Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Colours a session may carry (the sidebar dot); anything else is refused.
pub const COLORS: &[&str] = &["red", "orange", "yellow", "green", "blue", "purple", "gray"];
const MAX_TAGS: usize = 20;
const MAX_TAG: usize = 32;
const MAX_FOLDER: usize = 60;
const MAX_TITLE: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error("unknown session `{0}`")]
    UnknownSession(String),
    #[error("invalid {what}: `{value}`")]
    Invalid { what: &'static str, value: String },
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{path} is corrupt, fix or delete it: {error}")]
    Corrupt { path: String, error: String },
}

/// One row of `kor sessions` / the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub folder: Option<String>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub color: Option<String>,
    pub root: Option<String>,
    /// Unix seconds of the transcript's last change.
    pub modified: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub tag: Option<String>,
    pub folder: Option<String>,
    pub pinned: bool,
    pub archived: bool,
}

/// `kor_home()/desktop.json`.
pub fn meta_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("desktop.json"))
}

/// A session id is a transcript file stem: letters, digits, `-` and `_` only.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `#Auth` → `auth`; `None` when nothing valid is left.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let t: String = tag.trim().trim_start_matches('#').to_lowercase();
    let ok = !t.is_empty()
        && t.chars().count() <= MAX_TAG
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    ok.then_some(t)
}

fn read(path: &Path) -> Result<Value, TreeError> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| TreeError::Corrupt {
            path: path.display().to_string(),
            error: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({ "version": 1 })),
        Err(e) => Err(e.into()),
    }
}

fn write(path: &Path, v: &Value) -> Result<(), TreeError> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read, change, write `desktop.json` under its lock file.
fn update<T>(
    path: &Path,
    f: impl FnOnce(&mut Map<String, Value>) -> Result<T, TreeError>,
) -> Result<T, TreeError> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let _lock = kor_platform::lock_file(&path.with_extension("lock"))?;
    let mut v = read(path)?;
    if !v.is_object() {
        v = json!({ "version": 1 });
    }
    let out = f(v.as_object_mut().expect("object"))?;
    write(path, &v)?;
    Ok(out)
}

fn session<'a>(root: &'a mut Map<String, Value>, id: &str) -> &'a mut Map<String, Value> {
    let sessions = root.entry("sessions").or_insert_with(|| json!({}));
    if !sessions.is_object() {
        *sessions = json!({});
    }
    let s = sessions
        .as_object_mut()
        .expect("object")
        .entry(id.to_owned())
        .or_insert_with(|| json!({}));
    if !s.is_object() {
        *s = json!({});
    }
    s.as_object_mut().expect("object")
}

/// Drop a record that has nothing left (the desktop's `tidy`).
fn tidy(root: &mut Map<String, Value>, id: &str) {
    if let Some(Value::Object(s)) = root.get_mut("sessions")
        && s.get(id)
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty)
    {
        s.remove(id);
    }
}

fn check_session(sessions_dir: &Path, id: &str) -> Result<PathBuf, TreeError> {
    let p = sessions_dir.join(format!("{id}.jsonl"));
    if valid_id(id) && p.is_file() {
        Ok(p)
    } else {
        Err(TreeError::UnknownSession(id.to_owned()))
    }
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn first_user_line(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.plain_text()
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default()
}

fn sections(root: &Map<String, Value>) -> BTreeMap<String, String> {
    root.get("sections")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some((s["id"].as_str()?.to_owned(), s["name"].as_str()?.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Every session in `sessions_dir` with its tree data, pinned first then newest first.
///
/// # Errors
/// When `desktop.json` is corrupt.
pub fn list(sessions_dir: &Path, meta: &Path, filter: &Filter) -> Result<Vec<Entry>, TreeError> {
    let root = read(meta)?;
    let root = root.as_object().cloned().unwrap_or_default();
    let secs = sections(&root);
    let empty = Map::new();
    let records = root
        .get("sessions")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut out = Vec::new();
    for p in kor_agent::session::list(sessions_dir) {
        let Some(id) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let r = records.get(&id).cloned().unwrap_or_else(|| json!({}));
        let title = r["customTitle"].as_str().map_or_else(
            || {
                kor_agent::session::load(&p)
                    .map(|m| first_user_line(&m))
                    .unwrap_or_default()
            },
            str::to_owned,
        );
        let e = Entry {
            title,
            folder: r["sectionId"].as_str().and_then(|s| secs.get(s).cloned()),
            tags: strings(&r["tags"]),
            pinned: r["pinned"].as_bool().unwrap_or(false),
            archived: r["archived"].as_bool().unwrap_or(false),
            color: r["color"].as_str().map(str::to_owned),
            root: r["root"].as_str().map(str::to_owned),
            modified: mtime(&p),
            id,
        };
        let tag_ok = filter
            .tag
            .as_deref()
            .and_then(normalize_tag)
            .is_none_or(|t| e.tags.contains(&t));
        let folder_ok = filter.folder.as_deref().is_none_or(|f| {
            e.folder
                .as_deref()
                .is_some_and(|ef| ef.eq_ignore_ascii_case(f))
        });
        if tag_ok && folder_ok && (!filter.pinned || e.pinned) && (filter.archived || !e.archived) {
            out.push(e);
        }
    }
    out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.modified.cmp(&a.modified)));
    Ok(out)
}

/// Add and remove tags; returns the session's tags after the change.
///
/// # Errors
/// Unknown session, invalid tag, too many tags, or the file cannot be written.
pub fn tag(
    sessions_dir: &Path,
    meta: &Path,
    id: &str,
    add: &[String],
    remove: &[String],
) -> Result<Vec<String>, TreeError> {
    check_session(sessions_dir, id)?;
    let norm = |t: &String| {
        normalize_tag(t).ok_or_else(|| TreeError::Invalid {
            what: "tag",
            value: t.clone(),
        })
    };
    let add = add.iter().map(norm).collect::<Result<Vec<_>, _>>()?;
    let remove = remove.iter().map(norm).collect::<Result<Vec<_>, _>>()?;
    update(meta, |root| {
        let s = session(root, id);
        let mut tags = strings(s.get("tags").unwrap_or(&Value::Null));
        tags.retain(|t| !remove.contains(t));
        for t in add {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        if tags.len() > MAX_TAGS {
            return Err(TreeError::Invalid {
                what: "tag count",
                value: tags.len().to_string(),
            });
        }
        if tags.is_empty() {
            s.remove("tags");
        } else {
            s.insert("tags".into(), json!(tags));
        }
        tidy(root, id);
        Ok(tags)
    })
}

/// Put a session into a folder (created when missing); `None` or empty takes it out.
///
/// # Errors
/// Unknown session, invalid folder name, or the file cannot be written.
pub fn move_to(
    sessions_dir: &Path,
    meta: &Path,
    id: &str,
    folder: Option<&str>,
) -> Result<(), TreeError> {
    check_session(sessions_dir, id)?;
    let folder = folder.map(str::trim).filter(|f| !f.is_empty());
    if let Some(f) = folder
        && (f.chars().count() > MAX_FOLDER || f.chars().any(char::is_control))
    {
        return Err(TreeError::Invalid {
            what: "folder",
            value: f.to_owned(),
        });
    }
    update(meta, |root| {
        let section_id = folder.map(|name| {
            if let Some((sid, _)) = sections(root)
                .into_iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(name))
            {
                return sid;
            }
            let sid = new_id();
            let entry = json!({ "id": sid, "name": name, "createdAt": now_ms() });
            match root.get_mut("sections") {
                Some(Value::Array(list)) => list.push(entry),
                _ => {
                    root.insert("sections".into(), Value::Array(vec![entry]));
                }
            }
            sid
        });
        let s = session(root, id);
        match section_id {
            Some(sid) => {
                s.insert("sectionId".into(), json!(sid));
            }
            None => {
                s.remove("sectionId");
            }
        }
        tidy(root, id);
        Ok(())
    })
}

/// One boolean or text field of a session record (`pinned`, `archived`, `color`, `customTitle`).
fn set_field(
    sessions_dir: &Path,
    meta: &Path,
    id: &str,
    key: &str,
    value: Option<Value>,
) -> Result<(), TreeError> {
    check_session(sessions_dir, id)?;
    update(meta, |root| {
        let s = session(root, id);
        match value {
            Some(v) if v != Value::Bool(false) => {
                s.insert(key.into(), v);
            }
            _ => {
                s.remove(key);
            }
        }
        tidy(root, id);
        Ok(())
    })
}

/// # Errors
/// Unknown session or the file cannot be written.
pub fn pin(sessions_dir: &Path, meta: &Path, id: &str, on: bool) -> Result<(), TreeError> {
    set_field(sessions_dir, meta, id, "pinned", Some(Value::Bool(on)))
}

/// # Errors
/// Unknown session or the file cannot be written.
pub fn archive(sessions_dir: &Path, meta: &Path, id: &str, on: bool) -> Result<(), TreeError> {
    set_field(sessions_dir, meta, id, "archived", Some(Value::Bool(on)))
}

/// # Errors
/// Unknown session, a colour not in [`COLORS`], or the file cannot be written.
pub fn color(
    sessions_dir: &Path,
    meta: &Path,
    id: &str,
    color: Option<&str>,
) -> Result<(), TreeError> {
    if let Some(c) = color
        && !COLORS.contains(&c)
    {
        return Err(TreeError::Invalid {
            what: "color",
            value: c.to_owned(),
        });
    }
    set_field(sessions_dir, meta, id, "color", color.map(|c| json!(c)))
}

/// # Errors
/// Unknown session, a title too long, or the file cannot be written.
pub fn rename(
    sessions_dir: &Path,
    meta: &Path,
    id: &str,
    title: Option<&str>,
) -> Result<(), TreeError> {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = title
        && t.chars().count() > MAX_TITLE
    {
        return Err(TreeError::Invalid {
            what: "title",
            value: t.chars().take(40).collect(),
        });
    }
    set_field(
        sessions_dir,
        meta,
        id,
        "customTitle",
        title.map(|t| json!(t)),
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// UUID-v4-shaped id for a new section (same shape as the desktop's).
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut b = [0u8; 16];
    for (i, chunk) in b.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(u128::from(now_ms()) + i as u128);
        chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = b.iter().fold(String::new(), |mut out, x| {
        let _ = write!(out, "{x:02x}");
        out
    });
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// A search result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub snippet: String,
    /// Higher is better: matched terms, plus the semantic similarity when the local model is
    /// installed.
    pub score: f32,
    /// The semantic model took part in the ranking.
    pub semantic: bool,
}

fn snippet(text: &str, term: &str) -> String {
    let lower = text.to_lowercase();
    let at = lower.find(term).map_or(0, |b| lower[..b].chars().count());
    let start = at.saturating_sub(40);
    let s: String = text
        .chars()
        .skip(start)
        .take(140)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    format!("{}{}", if start > 0 { "…" } else { "" }, s.trim())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Full-text search over titles, tags and transcripts; with an embedder (the local model, never
/// a remote one) the candidates are also ranked by meaning, so "redis timeout" finds "cache
/// connection kept dropping".
///
/// # Errors
/// When `desktop.json` is corrupt.
pub fn search(
    sessions_dir: &Path,
    meta: &Path,
    query: &str,
    limit: usize,
    embed: Option<&dyn kor_memory::Embed>,
) -> Result<Vec<Hit>, TreeError> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.chars().count() >= 2)
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let entries = list(
        sessions_dir,
        meta,
        &Filter {
            archived: true,
            ..Filter::default()
        },
    )?;
    let mut hits = Vec::new();
    let mut docs = Vec::new();
    for e in entries.into_iter().take(500) {
        let text = kor_agent::session::load(&sessions_dir.join(format!("{}.jsonl", e.id)))
            .map(|m| {
                m.iter()
                    .filter(|m| matches!(m.role, Role::User | Role::Assistant))
                    .map(Message::plain_text)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let hay = format!("{} {} {}", e.title, e.tags.join(" "), text).to_lowercase();
        let matched: Vec<&String> = terms.iter().filter(|t| hay.contains(t.as_str())).collect();
        let doc: String = format!("{}\n{}", e.title, text)
            .chars()
            .take(1500)
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let score = matched.len() as f32
            + if e.tags.iter().any(|t| terms.contains(t)) {
                0.5
            } else {
                0.0
            };
        let snip = matched
            .first()
            .map_or_else(|| text.chars().take(140).collect(), |t| snippet(&text, t));
        hits.push(Hit {
            id: e.id,
            title: e.title,
            snippet: snip,
            score,
            semantic: false,
        });
        docs.push(doc);
    }
    if let Some(m) = embed
        && let Ok(q) = m.embed(&[query.to_owned()], kor_memory::embed::Kind::Query)
        && let Some(q) = q.first()
        && let Ok(vs) = m.embed(&docs, kor_memory::embed::Kind::Passage)
    {
        for (h, v) in hits.iter_mut().zip(vs) {
            let sim = cosine(q, &v);
            // Weighted so one exact term still beats a vague semantic neighbour.
            h.score += sim * 2.0;
            h.semantic = sim >= 0.80;
        }
        hits.retain(|h| h.score >= 1.0 || h.semantic);
    } else {
        hits.retain(|h| h.score >= 1.0);
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit);
    Ok(hits)
}

/// Title and tag suggestions for a finished session, from its first request and the words that
/// matter in it (no model call).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub title: String,
    pub tags: Vec<String>,
}

pub fn suggest(messages: &[Message]) -> Suggestion {
    const TOPICS: &[(&str, &[&str])] = &[
        (
            "bug",
            &[
                "bug", "hata", "fix", "düzelt", "crash", "çök", "erreur", "panic", "broken",
                "bozuk",
            ],
        ),
        (
            "auth",
            &[
                "auth",
                "login",
                "oturum aç",
                "giriş",
                "token",
                "oauth",
                "password",
                "şifre",
                "connexion",
            ],
        ),
        (
            "test",
            &["test", "spec", "vitest", "jest", "pytest", "cargo test"],
        ),
        (
            "refactor",
            &[
                "refactor",
                "yeniden düzenle",
                "clean up",
                "temizle",
                "rename",
            ],
        ),
        (
            "docs",
            &["readme", "docs", "documentation", "doküman", "belge"],
        ),
        (
            "perf",
            &["perf", "slow", "yavaş", "latency", "optimiz", "lent"],
        ),
        (
            "db",
            &[
                "sql",
                "database",
                "veritabanı",
                "migration",
                "postgres",
                "sqlite",
                "redis",
            ],
        ),
        (
            "ui",
            &[
                "css",
                "button",
                "buton",
                "layout",
                "ui",
                "tasarım",
                "component",
            ],
        ),
        (
            "deploy",
            &["deploy", "release", "yayın", "docker", "ci", "pipeline"],
        ),
        (
            "security",
            &[
                "security",
                "güvenlik",
                "xss",
                "injection",
                "cve",
                "vulnerab",
            ],
        ),
    ];
    let first = first_user_line(messages);
    let text: String = messages
        .iter()
        .filter(|m| m.role == Role::User)
        .take(3)
        .map(Message::plain_text)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let tags = TOPICS
        .iter()
        .filter(|(_, words)| words.iter().any(|w| text.contains(w)))
        .map(|(t, _)| (*t).to_owned())
        .take(4)
        .collect();
    let title = first
        .trim_end_matches(['.', '!', '?', ':'])
        .chars()
        .take(60)
        .collect::<String>();
    Suggestion { title, tags }
}

/// `kor sessions export <dir>`: one Markdown file per session in `<dir>/<folder>/` (the same tree
/// as the sidebar), front matter with tags, pin and colour. Returns how many were written.
///
/// # Errors
/// When `desktop.json` is corrupt or a file cannot be written.
pub fn export_markdown(sessions_dir: &Path, meta: &Path, out: &Path) -> Result<usize, TreeError> {
    let entries = list(
        sessions_dir,
        meta,
        &Filter {
            archived: true,
            ..Filter::default()
        },
    )?;
    let mut n = 0;
    for e in entries {
        let Ok(messages) = kor_agent::session::load(&sessions_dir.join(format!("{}.jsonl", e.id)))
        else {
            continue;
        };
        let dir = out.join(
            e.folder
                .as_deref()
                .map_or_else(|| "_".to_owned(), safe_name),
        );
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{}-{}.md", kor_tools::plans::slug(&e.title), e.id));
        std::fs::write(file, to_markdown(&e, &messages))?;
        n += 1;
    }
    Ok(n)
}

fn safe_name(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().trim_start_matches('.').to_owned();
    if s.is_empty() { "_".into() } else { s }
}

pub fn to_markdown(e: &Entry, messages: &[Message]) -> String {
    let mut s = format!(
        "---\nid: {}\ntitle: {}\nfolder: {}\ntags: [{}]\npinned: {}\ncolor: {}\n---\n",
        e.id,
        e.title.replace('\n', " "),
        e.folder.as_deref().unwrap_or(""),
        e.tags.join(", "),
        e.pinned,
        e.color.as_deref().unwrap_or("")
    );
    for m in messages {
        let text = m.plain_text();
        if text.trim().is_empty() {
            continue;
        }
        let who = match m.role {
            Role::User => "## user",
            _ => "## assistant",
        };
        let _ = write!(s, "\n{who}\n\n{}\n", text.trim_end());
    }
    s
}

/// `kor sessions import <file.md>`: a Markdown export becomes a session again (new id when the
/// id is taken), with its folder, tags, pin and colour. Returns the id.
///
/// # Errors
/// When the file is not an export, or a file cannot be written.
pub fn import_markdown(sessions_dir: &Path, meta: &Path, file: &Path) -> Result<String, TreeError> {
    let text = std::fs::read_to_string(file)?.replace("\r\n", "\n");
    let bad = || TreeError::Invalid {
        what: "session export",
        value: file.display().to_string(),
    };
    let rest = text.strip_prefix("---\n").ok_or_else(bad)?;
    let (head, body) = rest.split_once("\n---\n").ok_or_else(bad)?;
    let field = |k: &str| {
        head.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}: ")).map(str::trim))
            .unwrap_or("")
    };
    let mut messages = Vec::new();
    for block in body.split("\n## ").skip(1) {
        let (who, text) = block.split_once('\n').unwrap_or((block, ""));
        let role = if who.trim() == "user" {
            Role::User
        } else {
            Role::Assistant
        };
        messages.push(Message::text(role, text.trim().to_owned()));
    }
    if messages.is_empty() {
        return Err(bad());
    }
    std::fs::create_dir_all(sessions_dir)?;
    let wanted = field("id");
    let id = if valid_id(wanted) && !sessions_dir.join(format!("{wanted}.jsonl")).exists() {
        wanted.to_owned()
    } else {
        format!("import-{}", now_ms())
    };
    kor_agent::session::save(&sessions_dir.join(format!("{id}.jsonl")), &messages)?;
    let tags: Vec<String> = field("tags")
        .trim_matches(['[', ']'])
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    if !tags.is_empty() {
        tag(sessions_dir, meta, &id, &tags, &[])?;
    }
    let folder = field("folder");
    if !folder.is_empty() {
        move_to(sessions_dir, meta, &id, Some(folder))?;
    }
    if field("pinned") == "true" {
        pin(sessions_dir, meta, &id, true)?;
    }
    let c = field("color");
    if COLORS.contains(&c) {
        color(sessions_dir, meta, &id, Some(c))?;
    }
    let title = field("title");
    if !title.is_empty() {
        rename(sessions_dir, meta, &id, Some(title))?;
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let sessions = t.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        for (id, text) in [
            ("a1", "Fix the login bug in auth"),
            ("b2", "redis timeout keeps happening"),
            ("c3", "write docs"),
        ] {
            kor_agent::session::save(
                &sessions.join(format!("{id}.jsonl")),
                &[
                    Message::text(Role::User, text),
                    Message::text(Role::Assistant, "ok"),
                ],
            )
            .unwrap();
        }
        let meta = t.path().join("desktop.json");
        // The desktop's own keys must survive our edits.
        std::fs::write(
            &meta,
            r#"{"version":1,"theme":"dark","sessions":{"c3":{"model":"x/y"}}}"#,
        )
        .unwrap();
        (t, sessions, meta)
    }

    #[test]
    fn tags_folders_pins_and_filters_share_desktop_json() {
        let (_t, s, m) = setup();
        assert_eq!(
            tag(&s, &m, "a1", &["#Auth".into(), "bug".into()], &[]).unwrap(),
            ["auth", "bug"]
        );
        assert!(tag(&s, &m, "a1", &["no spaces".into()], &[]).is_err());
        assert!(tag(&s, &m, "zz", &["x".into()], &[]).is_err());
        assert!(tag(&s, &m, "../x", &["x".into()], &[]).is_err());
        move_to(&s, &m, "a1", Some("Backend")).unwrap();
        move_to(&s, &m, "b2", Some("backend")).unwrap();
        pin(&s, &m, "b2", true).unwrap();
        color(&s, &m, "b2", Some("blue")).unwrap();
        assert!(color(&s, &m, "b2", Some("#fff")).is_err());
        archive(&s, &m, "c3", true).unwrap();

        let all = list(&s, &m, &Filter::default()).unwrap();
        assert_eq!(
            all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["b2", "a1"],
            "pinned first, archived hidden"
        );
        assert_eq!(
            all[0].folder.as_deref(),
            Some("Backend"),
            "one folder, matched case-insensitively"
        );
        let auth = list(
            &s,
            &m,
            &Filter {
                tag: Some("#auth".into()),
                ..Filter::default()
            },
        )
        .unwrap();
        assert_eq!(auth.len(), 1);
        assert_eq!(
            list(
                &s,
                &m,
                &Filter {
                    archived: true,
                    ..Filter::default()
                }
            )
            .unwrap()
            .len(),
            3
        );

        let raw: Value = serde_json::from_slice(&std::fs::read(&m).unwrap()).unwrap();
        assert_eq!(raw["theme"], "dark");
        assert_eq!(raw["sessions"]["c3"]["model"], "x/y");
        assert_eq!(raw["sections"].as_array().unwrap().len(), 1);
        move_to(&s, &m, "a1", None).unwrap();
        tag(&s, &m, "a1", &[], &["auth".into(), "bug".into()]).unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&m).unwrap()).unwrap();
        assert!(raw["sessions"].get("a1").is_none(), "empty record tidied");
    }

    struct Fake;
    impl kor_memory::Embed for Fake {
        #[allow(clippy::unnecessary_literal_bound)] // the trait's signature
        fn model(&self) -> &str {
            "fake"
        }
        fn min_similarity(&self) -> f32 {
            0.5
        }
        fn embed(
            &self,
            texts: &[String],
            _k: kor_memory::embed::Kind,
        ) -> Result<Vec<Vec<f32>>, String> {
            // "cache" and "redis" mean the same thing to this model.
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    vec![
                        f32::from(u8::from(t.contains("redis") || t.contains("cache"))),
                        f32::from(u8::from(t.contains("login"))),
                        0.1,
                    ]
                })
                .collect())
        }
    }

    #[test]
    fn search_full_text_and_semantic() {
        let (_t, s, m) = setup();
        let hits = search(&s, &m, "redis timeout", 10, None).unwrap();
        assert_eq!(hits[0].id, "b2");
        assert!(hits[0].snippet.contains("redis"));
        assert!(search(&s, &m, "cache", 10, None).unwrap().is_empty());
        let sem = search(&s, &m, "cache", 10, Some(&Fake)).unwrap();
        assert_eq!(sem.first().map(|h| h.id.as_str()), Some("b2"));
        assert!(sem[0].semantic);
    }

    #[test]
    fn suggest_export_and_import_round_trip() {
        let (t, s, m) = setup();
        let msgs = kor_agent::session::load(&s.join("a1.jsonl")).unwrap();
        let sug = suggest(&msgs);
        assert_eq!(sug.title, "Fix the login bug in auth");
        assert!(sug.tags.contains(&"bug".to_owned()) && sug.tags.contains(&"auth".to_owned()));
        tag(&s, &m, "a1", &["auth".into()], &[]).unwrap();
        move_to(&s, &m, "a1", Some("Backend")).unwrap();
        let out = t.path().join("export");
        assert_eq!(export_markdown(&s, &m, &out).unwrap(), 3);
        let file = std::fs::read_dir(out.join("Backend"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let s2 = t.path().join("sessions2");
        let m2 = t.path().join("desktop2.json");
        let id = import_markdown(&s2, &m2, &file).unwrap();
        assert_eq!(id, "a1");
        let e = &list(&s2, &m2, &Filter::default()).unwrap()[0];
        assert_eq!(
            (e.folder.as_deref(), e.tags.clone()),
            (Some("Backend"), vec!["auth".to_owned()])
        );
        assert_eq!(
            kor_agent::session::load(&s2.join("a1.jsonl"))
                .unwrap()
                .len(),
            2
        );
    }
}
