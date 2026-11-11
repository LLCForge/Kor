//! Generated Markdown views of `~/Kor/` and the way back (front matter → store).

use std::fmt::Write as _;
use std::path::PathBuf;

use kor_core::{ContentPart, Message, Role};
use serde_json::Value;

use super::{DIFFS, Layout, MEDIA, Manifest, SESSIONS, SyncReport, hash, rel, write_generated};

/// Largest transcript rendered; larger ones keep their last part.
const MAX_VIEW: usize = 512 * 1024;
const MEDIA_EXT: [&str; 12] = [
    "png", "jpg", "jpeg", "webp", "gif", "svg", "mp4", "webm", "mov", "mp3", "wav", "ogg",
];

fn desktop_json(l: &Layout) -> PathBuf {
    l.home.join("desktop.json")
}

fn read_desktop(l: &Layout) -> Value {
    std::fs::read(desktop_json(l))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// Project folders the desktop's sessions were opened in.
pub(super) fn session_roots(l: &Layout) -> Vec<PathBuf> {
    let d = read_desktop(l);
    let mut v: Vec<PathBuf> = d["sessions"]
        .as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .filter_map(|s| s["root"].as_str().map(PathBuf::from))
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Session fields that can be edited from the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Front {
    pub title: Option<String>,
    pub pinned: bool,
    pub archived: bool,
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Split `---\n<yaml>\n---\n<body>`.
pub(super) fn split_front(text: &str) -> (Option<serde_yaml::Value>, &str) {
    let t = text.trim_start_matches('\u{feff}');
    let Some(rest) = t
        .strip_prefix("---")
        .map(|r| r.trim_start_matches('\r'))
        .and_then(|r| r.strip_prefix('\n'))
    else {
        return (None, t);
    };
    let end = rest.find("\n---").map(|i| (i, i + 4));
    let Some((yaml_end, after)) = end else {
        return (None, t);
    };
    let body = rest[after..].trim_start_matches(['\r', '\n']);
    (serde_yaml::from_str(&rest[..yaml_end]).ok(), body)
}

fn front_of(v: &serde_yaml::Value) -> Front {
    Front {
        title: v["title"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        pinned: v["pinned"].as_bool().unwrap_or(false),
        archived: v["archived"].as_bool().unwrap_or(false),
    }
}

fn first_user_text(msgs: &[Message]) -> String {
    msgs.iter()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.plain_text()
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default()
}

/// Markdown for one transcript.
fn loc() -> kor_i18n::Localizer {
    kor_i18n::Localizer::new(kor_i18n::Lang::from_system().unwrap_or(kor_i18n::Lang::En))
}

pub(super) fn render_session(id: &str, msgs: &[Message], meta: &Value) -> String {
    let l = loc();
    let title = meta["customTitle"]
        .as_str()
        .map_or_else(|| first_user_text(msgs), str::to_owned);
    let mut out = format!(
        "---\nid: {}\ntitle: {}\npinned: {}\narchived: {}\n",
        yaml_str(id),
        yaml_str(&title),
        meta["pinned"].as_bool().unwrap_or(false),
        meta["archived"].as_bool().unwrap_or(false),
    );
    for k in ["root", "model"] {
        if let Some(v) = meta[k].as_str() {
            let _ = writeln!(
                out,
                "{}: {}",
                if k == "root" { "project" } else { k },
                yaml_str(v)
            );
        }
    }
    out.push_str("---\n\n");
    let _ = writeln!(out, "# {}", if title.is_empty() { id } else { &title });
    let mut attachments = Vec::new();
    for m in msgs {
        let who = match m.role {
            Role::User => format!("## {}", l.t("korfolder-view-user")),
            Role::Assistant => "## Kor".to_owned(),
            Role::System | Role::Tool => continue,
        };
        let mut section = String::new();
        for p in &m.parts {
            match p {
                ContentPart::Text { text } if !text.trim().is_empty() => {
                    section.push_str(text.trim());
                    section.push_str("\n\n");
                }
                ContentPart::ToolUse { name, input, .. } => {
                    let arg = ["path", "file_path", "command", "pattern", "url"]
                        .iter()
                        .find_map(|k| input[*k].as_str())
                        .unwrap_or("");
                    let _ = write!(section, "> `{name}` {arg}\n\n");
                }
                ContentPart::File { path, .. } => {
                    attachments.push(path.clone());
                    let _ = write!(
                        section,
                        "{}: [{path}](<{path}>)\n\n",
                        l.t("korfolder-view-attachment")
                    );
                }
                ContentPart::Image { .. } => {
                    let _ = write!(section, "({})\n\n", l.t("korfolder-view-image"));
                }
                _ => {}
            }
        }
        if !section.is_empty() {
            let _ = write!(out, "\n{who}\n\n{section}");
        }
    }
    if !attachments.is_empty() {
        out.push_str("\n## Ekler\n\n");
        for a in attachments {
            let _ = writeln!(out, "- [{a}](<{a}>)");
        }
    }
    if out.len() > MAX_VIEW {
        let cut = out.len() - MAX_VIEW;
        let cut = (cut..out.len())
            .find(|i| out.is_char_boundary(*i))
            .unwrap_or(out.len());
        out = format!("…\n{}", &out[cut..]);
    }
    out
}

/// Apply an edited view's front matter to `desktop.json` (`sessions.<id>`). Returns `true`
/// when the store changed.
#[allow(clippy::many_single_char_names)] // short names of the math or the protocol
pub(super) fn apply_front(l: &Layout, id: &str, f: &Front) -> std::io::Result<bool> {
    let path = desktop_json(l);
    let mut d = read_desktop(l);
    if !d.is_object() {
        if path.exists() {
            // Damaged desktop.json: never overwritten (meta.rs rule).
            return Ok(false);
        }
        d = serde_json::json!({ "version": 1, "sessions": {} });
    }
    if !d["sessions"].is_object() {
        d["sessions"] = serde_json::json!({});
    }
    let s = &mut d["sessions"][id];
    if !s.is_object() {
        *s = serde_json::json!({});
    }
    let before = s.clone();
    match &f.title {
        Some(t) => s["customTitle"] = Value::String(t.chars().take(200).collect()),
        None => {
            if let Some(o) = s.as_object_mut() {
                o.remove("customTitle");
            }
        }
    }
    for (k, v) in [("pinned", f.pinned), ("archived", f.archived)] {
        if let Some(o) = s.as_object_mut() {
            if v {
                o.insert(k.into(), Value::Bool(true));
            } else {
                o.remove(k);
            }
        }
    }
    if *s == before {
        return Ok(false);
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&d).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, path)?;
    Ok(true)
}

pub(super) fn sessions(l: &Layout, m: &mut Manifest, r: &mut SyncReport) {
    let dir = l.home.join("sessions");
    let desk = read_desktop(l);
    let mut live = Vec::new();
    for path in kor_agent::session::list(&dir) {
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if !crate::valid_session_id(&id) {
            continue;
        }
        let view = l.dir(SESSIONS).join(format!("{id}.md"));
        live.push(view.clone());
        let key = rel(l, &view);
        // The way back: a view the user edited since Kor wrote it.
        if let Ok(cur) = std::fs::read_to_string(&view)
            && m.generated
                .get(&key)
                .is_some_and(|h| *h != hash(cur.as_bytes()))
        {
            if let (Some(y), _) = split_front(&cur) {
                match apply_front(l, &id, &front_of(&y)) {
                    Ok(_) => {
                        // The file wins: accepted as is, regenerated on the next change only.
                        m.generated.insert(key, hash(cur.as_bytes()));
                        r.applied.push(view);
                    }
                    Err(e) => r.notes.push(format!("{}: {e}", view.display())),
                }
            } else {
                r.kept.push(view);
            }
            continue;
        }
        let Ok(msgs) = kor_agent::session::load(&path) else {
            continue;
        };
        let body = render_session(&id, &msgs, &desk["sessions"][&id]);
        write_generated(l, m, r, &view, &body);
    }
    drop_stale(l, m, SESSIONS, &live);
}

/// Forget (and delete when unchanged) generated files of `folder` that no longer have a source.
fn drop_stale(l: &Layout, m: &mut Manifest, folder: &str, live: &[PathBuf]) {
    let prefix = format!("{folder}/");
    let stale: Vec<String> = m
        .generated
        .keys()
        .filter(|k| k.starts_with(&prefix) && !live.contains(&l.root.join(k.as_str())))
        .cloned()
        .collect();
    for k in stale {
        let p = l.root.join(&k);
        if std::fs::read(&p).is_ok_and(|b| Some(&hash(&b)) == m.generated.get(&k)) {
            let _ = std::fs::remove_file(&p);
        }
        m.generated.remove(&k);
    }
}

/// `Diffler/<session>.diff`: every undo journal with pending entries, original vs. now.
pub(super) fn diffs(l: &Layout, m: &mut Manifest, r: &mut SyncReport) {
    let base = l.home.join("undo");
    let mut live = Vec::new();
    for j in kor_tools::undo::list(&base) {
        let entries: Vec<kor_tools::undo::Entry> =
            std::fs::read_to_string(j.dir.join("manifest.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|x| serde_json::from_str(x).ok())
                .collect();
        let mut out = String::new();
        for e in entries {
            let shown = e.path.display().to_string();
            if e.skipped {
                let _ = writeln!(out, "# {shown}: (büyük dosya ya da klasör, fark yok)");
                continue;
            }
            let old = e
                .blob
                .and_then(|n| std::fs::read(j.dir.join("blobs").join(n.to_string())).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let new = std::fs::read(&e.path)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            if old != new {
                out.push_str(&kor_agent::port::unified_diff(&shown, &old, &new));
                out.push('\n');
            }
        }
        if out.is_empty() {
            continue;
        }
        let view = l.dir(DIFFS).join(format!("{}.diff", j.session));
        live.push(view.clone());
        write_generated(l, m, r, &view, &out);
    }
    drop_stale(l, m, DIFFS, &live);
}

/// `Görseller/Galeri.md`: every media file of the linked projects with size and date.
pub(super) fn gallery(l: &Layout, m: &mut Manifest, r: &mut SyncReport) {
    let mut out = String::from("---\ntitle: \"Kor görselleri\"\n---\n\n# Görseller\n");
    let projects = m.projects.clone();
    for p in &projects {
        let dir = p.join(kor_agent::media::MEDIA_DIR);
        let mut files: Vec<(PathBuf, u64, String)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|f| {
                f.extension().is_some_and(|x| {
                    MEDIA_EXT.contains(&x.to_string_lossy().to_lowercase().as_str())
                })
            })
            .map(|f| {
                let md = std::fs::metadata(&f).ok();
                let size = md.as_ref().map_or(0, std::fs::Metadata::len);
                let date = md
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        chrono::DateTime::<chrono::Local>::from(t)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                (f, size, date)
            })
            .collect();
        if files.is_empty() {
            continue;
        }
        files.sort_by(|a, b| b.2.cmp(&a.2));
        let name = super::project_name(m, p);
        let _ = write!(
            out,
            "\n## {name}\n\n| Dosya | Boyut | Tarih |\n| --- | ---: | --- |\n"
        );
        for (f, size, date) in files {
            let file = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let _ = writeln!(out, "| [{file}](<{name}/{file}>) | {size} B | {date} |");
        }
    }
    let view = l.dir(MEDIA).join("Galeri.md");
    write_generated(l, m, r, &view, &out);
}
