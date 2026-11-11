//! Arayüzün okuduğu veriler: CLI ile ortak oturum deposu ve açık çalışma klasörü.
//! Dönen JSON şekilleri `frontend/src/desktop/ipc.ts` ve `src/core/sessions.ts` sözleşmesindedir.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use kor_core::{ContentPart, Message, Role};
use serde_json::{Value, json};

/// Arayüzde gösterilecek en büyük dosya; daha büyüğü pencereyi kilitler.
const MAX_READ_BYTES: u64 = 5 * 1024 * 1024;
const TITLE_CHARS: usize = 80;
/// Başlık için ilk kullanıcı mesajı en fazla bu kadar satırda aranır (tüm dökümü okumamak için).
const TITLE_SCAN_LINES: usize = 64;

/// `kor` CLI ile aynı depo: `%LOCALAPPDATA%\kor\sessions` (`kor resume` bu dosyaları açar).
pub fn sessions_dir() -> Option<PathBuf> {
    // `KOR_HOME` (isolated E2E runs, ADR 0061) moves it together with the CLI's.
    kor_runtime::sessions_dir()
}

/// Oturum kimliği yalnızca dosya adı karakterlerinden oluşur; yol dışarı kaçamaz (CWE-22).
pub fn session_path(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err("Geçersiz oturum kimliği".into());
    }
    Ok(dir.join(format!("{id}.jsonl")))
}

fn millis(t: std::io::Result<std::time::SystemTime>) -> u64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn user_text(m: &Message) -> Option<String> {
    if m.role != Role::User {
        return None;
    }
    let text: Vec<&str> = m
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

fn first_user_text(path: &Path) -> Option<String> {
    let f = fs::File::open(path).ok()?;
    BufReader::new(f)
        .lines()
        .take(TITLE_SCAN_LINES)
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Message>(&l).ok())
        .find_map(|m| user_text(&m))
}

fn title_from(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut t: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        t.push('…');
    }
    t
}

/// `SessionMeta[]`, en yeni önce. Masaüstü bilgileri (başlık, sabit, arşiv, bölüm, klasör, model) `meta`dan.
pub fn list_sessions(dir: &Path) -> Value {
    let meta = crate::meta::read().unwrap_or_default();
    list_with(dir, &meta)
}

pub fn list_with(dir: &Path, meta: &crate::meta::Meta) -> Value {
    let mut items: Vec<Value> = kor_agent::session::list(dir)
        .into_iter()
        .filter_map(|path| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let fmeta = fs::metadata(&path).ok()?;
            let auto =
                first_user_text(&path).map_or_else(|| "Yeni sohbet".to_owned(), |t| title_from(&t));
            let m = meta.sessions.get(&id).cloned().unwrap_or_default();
            let mut item = json!({
                "id": id,
                "title": m.custom_title.clone().unwrap_or_else(|| auto.clone()),
                "autoTitle": auto,
                // CLI oturumlarında klasör ve model bilinmez; arayüz boş değeri "bilinmiyor" gösterir.
                "root": m.root.clone().unwrap_or_default(),
                "model": m.model.clone().unwrap_or_default(),
                "createdAt": millis(fmeta.created()),
                "updatedAt": millis(fmeta.modified()),
                "pinned": m.pinned,
                "archived": m.archived,
            });
            if let Some(t) = &m.custom_title {
                item["customTitle"] = json!(t);
            }
            // Silinmiş bölüme işaret eden kayıt yok sayılır.
            if let Some(sec) = m
                .section_id
                .as_ref()
                .filter(|s| meta.sections.iter().any(|x| &x.id == *s))
            {
                item["sectionId"] = json!(sec);
            }
            if let Some(mode) = &m.permission_mode {
                item["permissionMode"] = json!(mode);
            }
            // §28.6: CLI ile ortak etiket ve renk.
            if !m.tags.is_empty() {
                item["tags"] = json!(m.tags);
            }
            if let Some(c) = &m.color {
                item["color"] = json!(c);
            }
            Some(item)
        })
        .collect();
    // Dosya adı sırası yalnızca CLI'ın zaman damgalı kimliklerinde doğrudur; masaüstü UUID kullanır.
    items.sort_by_key(|s| std::cmp::Reverse(s["updatedAt"].as_u64().unwrap_or(0)));
    Value::Array(items)
}

fn tool_summary(input: &Value) -> String {
    let s = match input {
        Value::Object(o) => o
            .values()
            .find_map(Value::as_str)
            .map_or_else(|| input.to_string(), str::to_owned),
        other => other.to_string(),
    };
    title_from(&s)
}

/// Kayıtlı mesajlardan arayüz dökümü (`TranscriptItem[]`).
pub fn transcript(messages: &[Message]) -> Value {
    let results: HashMap<&str, &str> = messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some((tool_use_id.as_str(), content.as_str())),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for m in messages {
        match m.role {
            Role::User => {
                if let Some(text) = user_text(m) {
                    out.push(json!({ "kind": "user", "text": text, "attachments": [] }));
                }
            }
            Role::Assistant => {
                for p in &m.parts {
                    match p {
                        ContentPart::Text { text } if !text.trim().is_empty() => {
                            out.push(json!({ "kind": "assistant", "text": text }));
                        }
                        ContentPart::ToolUse { id, name, input } => out.push(json!({
                            "kind": "tool",
                            "id": id,
                            "name": name,
                            // Card tools are redrawn from their whole input.
                            "summary": if kor_agent::assistant::is_card_tool(name) {
                                input.to_string()
                            } else {
                                tool_summary(input)
                            },
                            "result": results.get(id.as_str()).copied().unwrap_or(""),
                        })),
                        _ => {}
                    }
                }
            }
            Role::System | Role::Tool => {}
        }
    }
    Value::Array(out)
}

pub fn open_session(dir: &Path, id: &str) -> Result<Value, String> {
    let path = session_path(dir, id)?;
    let messages = kor_agent::session::load(&path).map_err(|e| format!("Oturum okunamadı: {e}"))?;
    Ok(transcript(&messages))
}

pub fn delete_session(dir: &Path, id: &str) -> Result<(), String> {
    let path = session_path(dir, id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Oturum silinemedi: {e}")),
    }
}

/// Sohbetin kayıtlı dökümünü yeni bir kimlikle kopyalar; yeni kimliği döner.
/// `new_id` saniye çözünürlüklü olduğundan aynı saniyedeki kopyalar sonek alır; var olan dosya ezilmez.
pub fn duplicate_session(dir: &Path, id: &str) -> Result<String, String> {
    let src = session_path(dir, id)?;
    let data = fs::read(&src).map_err(|e| format!("Oturum okunamadı: {e}"))?;
    let base = kor_agent::session::new_id();
    for n in 0..100 {
        let new = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let dst = session_path(dir, &new)?;
        let mut f = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dst)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Oturum kopyalanamadı: {e}")),
        };
        return match std::io::Write::write_all(&mut f, &data) {
            Ok(()) => Ok(new),
            Err(e) => {
                drop(f);
                let _ = fs::remove_file(&dst);
                Err(format!("Oturum kopyalanamadı: {e}"))
            }
        };
    }
    Err("Oturum kopyalanamadı: boş kimlik bulunamadı".into())
}

/// Arayüzden gelen göreli yolu kökün içinde çözer; `..`, mutlak yol ve kök dışına çıkan bağlar reddedilir (CWE-22).
pub fn resolve_in(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(rel);
    if rel
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("Yol çalışma klasörünün dışında".into());
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("Çalışma klasörü açılamadı: {e}"))?;
    let full = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("Bulunamadı: {e}"))?;
    if !full.starts_with(&root) {
        return Err("Yol çalışma klasörünün dışında".into());
    }
    Ok(full)
}

fn rel_slash(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// `FileEntry[]`: önce klasörler, sonra ada göre.
pub fn list_dir(root: &Path, rel: &str) -> Result<Value, String> {
    let dir = resolve_in(root, rel)?;
    let base = root.canonicalize().map_err(|e| e.to_string())?;
    let mut entries: Vec<(bool, String, String)> = fs::read_dir(&dir)
        .map_err(|e| format!("Klasör okunamadı: {e}"))?
        .filter_map(Result::ok)
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            (
                is_dir,
                e.file_name().to_string_lossy().into_owned(),
                rel_slash(&base, &e.path()),
            )
        })
        .collect();
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    Ok(Value::Array(
        entries
            .into_iter()
            .map(|(is_dir, name, path)| json!({ "name": name, "path": path, "isDir": is_dir }))
            .collect(),
    ))
}

pub fn read_file(root: &Path, rel: &str) -> Result<String, String> {
    read_file_bytes(root, rel).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `readFile`'ın baytları: arayüz bunu `kor_bytes` ile JSON'suz alır (ipc_bytes.rs, ADR 0062).
pub fn read_file_bytes(root: &Path, rel: &str) -> Result<Vec<u8>, String> {
    let path = resolve_in(root, rel)?;
    let len = fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if len > MAX_READ_BYTES {
        return Err(format!(
            "Dosya çok büyük ({} MB); en fazla 5 MB gösterilir",
            len / (1024 * 1024)
        ));
    }
    fs::read(&path).map_err(|e| format!("Dosya okunamadı: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_paths_and_ids() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "merhaba").unwrap();
        assert_eq!(read_file(dir.path(), "a.txt").unwrap(), "merhaba");
        assert!(resolve_in(dir.path(), "../x").is_err());
        assert!(resolve_in(dir.path(), "/etc/passwd").is_err());
        assert!(session_path(dir.path(), "../evil").is_err());
        assert!(session_path(dir.path(), "a/b").is_err());
        assert!(session_path(dir.path(), "20260101-000000-1").is_ok());
    }

    #[test]
    fn transcript_pairs_tool_results() {
        let msgs = vec![
            Message::text(Role::User, "dosyayı oku"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    ContentPart::Text {
                        text: "Okuyorum.".into(),
                    },
                    ContentPart::ToolUse {
                        id: "t1".into(),
                        name: "read".into(),
                        input: json!({ "path": "a.txt" }),
                    },
                ],
            },
            Message {
                role: Role::Tool,
                parts: vec![ContentPart::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "merhaba".into(),
                    is_error: false,
                }],
            },
        ];
        let t = transcript(&msgs);
        assert_eq!(t[0]["kind"], "user");
        assert_eq!(t[1]["text"], "Okuyorum.");
        assert_eq!(t[2]["summary"], "a.txt");
        assert_eq!(t[2]["result"], "merhaba");
    }

    #[test]
    fn lists_sessions_with_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("20260101-000000-1.jsonl");
        kor_agent::session::save(&p, &[Message::text(Role::User, "ilk soru\nikinci satır")])
            .unwrap();
        let list = list_sessions(dir.path());
        assert_eq!(list[0]["id"], "20260101-000000-1");
        assert_eq!(list[0]["title"], "ilk soru");
    }

    #[test]
    fn duplicates_get_distinct_ids_and_the_same_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("20260101-000000-1.jsonl");
        kor_agent::session::save(&p, &[Message::text(Role::User, "soru")]).unwrap();
        let a = duplicate_session(dir.path(), "20260101-000000-1").unwrap();
        let b = duplicate_session(dir.path(), "20260101-000000-1").unwrap();
        assert_ne!(a, b);
        for id in [&a, &b] {
            assert_eq!(
                fs::read(dir.path().join(format!("{id}.jsonl"))).unwrap(),
                fs::read(&p).unwrap()
            );
        }
        assert!(duplicate_session(dir.path(), "../x").is_err());
        assert!(duplicate_session(dir.path(), "yok").is_err());
    }
}
