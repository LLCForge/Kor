//! Private documents for the agent (Master Build Prompt §28.7, ADR 0063): `kor docs add
//! ./spec.pdf | ./folder | https://intranet/wiki`.
//!
//! Text is extracted locally (Markdown, text, HTML, PDF via lopdf, DOCX via its XML, Confluence /
//! Notion HTML or Markdown exports as folders), split into chunks and stored in the knowledge base
//! as `doc` records of the project (FTS plus local embeddings when the model is installed), so the
//! agent's automatic recall and `kor recall` find them. The extracted text is also kept as Markdown
//! in `.kor/docs/<id>.md` and listed in `.kor/docs/index.json` with the source's SHA-256;
//! `kor docs refresh` re-indexes sources whose content changed. URLs go through the network guard
//! (privacy/offline modes refuse non-local hosts); a page behind a login can be read through the
//! user's own Chrome with the paired extension (`--browser`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_memory::{Knowledge, Record};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_SOURCE: u64 = 50 * 1024 * 1024;
const MAX_TEXT: usize = 4_000_000;
const MAX_DIR_FILES: usize = 500;
const CHUNK: usize = 1_500;
const MAX_CHUNKS: usize = 2_000;
const EXTS: &[&str] = &[
    "md", "markdown", "txt", "rst", "adoc", "html", "htm", "pdf", "docx",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Path (as given, made absolute) or URL.
    pub source: String,
    pub title: String,
    pub sha256: String,
    /// Unix seconds.
    pub added: i64,
    pub updated: i64,
    pub chars: usize,
    /// Knowledge records of the chunks.
    pub records: Vec<i64>,
    /// Read through the user's browser (a page behind a login).
    #[serde(default)]
    pub browser: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    docs: Vec<Entry>,
}

/// `.kor/docs/` of the project.
pub fn dir(root: &Path) -> PathBuf {
    root.join(".kor").join("docs")
}

fn index_path(root: &Path) -> PathBuf {
    dir(root).join("index.json")
}

fn read_index(root: &Path) -> Index {
    std::fs::read(index_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_index(root: &Path, i: &Index) -> Result<(), String> {
    let d = dir(root);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let tmp = d.join("index.json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(i).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, index_path(root)).map_err(|e| e.to_string())
}

/// Every added document.
pub fn list(root: &Path) -> Vec<Entry> {
    read_index(root).docs
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let t = out.trim_matches('-').to_owned();
    if t.is_empty() { "doc".into() } else { t }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

// ---------------------------------------------------------------------------------------------
// Text extraction
// ---------------------------------------------------------------------------------------------

/// Plain text of HTML: scripts and styles dropped, block tags as line breaks, entities decoded.
pub fn html_text(html: &str) -> String {
    let mut s = html.to_owned();
    for tag in ["script", "style", "noscript", "svg", "head"] {
        while let Some(start) = s.to_ascii_lowercase().find(&format!("<{tag}")) {
            let lower = s.to_ascii_lowercase();
            let end = lower[start..]
                .find(&format!("</{tag}>"))
                .map_or(s.len(), |e| start + e + tag.len() + 3);
            s.replace_range(start..end, " ");
        }
    }
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    for c in s.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "p" | "br"
                        | "div"
                        | "li"
                        | "tr"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "pre"
                        | "section"
                        | "article"
                        | "table"
                        | "ul"
                        | "ol"
                ) {
                    out.push('\n');
                }
            }
            (true, c) => tag.push(c),
            (false, c) => out.push(c),
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    // Closing and opening block tags both break the line: one line per block, no blank ones.
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapse runs of blank lines and trailing spaces.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for l in s.lines().map(str::trim_end) {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l.trim_start_matches(['\t']));
        out.push('\n');
    }
    out.trim().to_owned()
}

/// Text of a PDF (pages in order).
fn pdf_text(bytes: &[u8]) -> Result<String, String> {
    let doc = lopdf::Document::load_mem(bytes).map_err(|e| format!("PDF: {e}"))?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    let mut out = String::new();
    for p in pages {
        if let Ok(t) = doc.extract_text(&[p]) {
            out.push_str(&t);
            out.push('\n');
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Ok(tidy(&out))
}

/// Text of a DOCX: `<w:t>` runs, one paragraph per `<w:p>`.
fn docx_text(bytes: &[u8]) -> Result<String, String> {
    let mut z =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("DOCX: {e}"))?;
    let mut f = z
        .by_name("word/document.xml")
        .map_err(|e| format!("DOCX: {e}"))?;
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(&mut f, 64 * 1024 * 1024), &mut xml)
        .map_err(|e| format!("DOCX: {e}"))?;
    let mut out = String::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find('<') {
        let Some(j) = rest[i..].find('>') else { break };
        let tag = &rest[i + 1..i + j];
        let after = &rest[i + j + 1..];
        if tag == "w:t" || tag.starts_with("w:t ") {
            let end = after.find("</w:t>").unwrap_or(after.len());
            out.push_str(&after[..end]);
            rest = &after[end..];
            continue;
        }
        if tag == "/w:p" || tag == "w:br/" || tag == "w:tab/" {
            out.push(if tag == "w:tab/" { '\t' } else { '\n' });
        }
        rest = after;
    }
    Ok(tidy(&html_text(&out.replace('<', "&lt;"))))
}

/// Text of one local file by extension; `None` for an unsupported type.
pub fn file_text(path: &Path) -> Result<Option<String>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !EXTS.contains(&ext.as_str()) {
        return Ok(None);
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_SOURCE {
        return Err(format!("{}: larger than 50 MB", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(match ext.as_str() {
        "pdf" => pdf_text(&bytes)?,
        "docx" => docx_text(&bytes)?,
        "html" | "htm" => html_text(&String::from_utf8_lossy(&bytes)),
        _ => tidy(&String::from_utf8_lossy(&bytes)),
    }))
}

/// Text of a folder (exports from Confluence, Notion, wikis): every supported file, in path order.
#[allow(clippy::unnecessary_wraps)] // same signature as the other source readers
fn dir_text(path: &Path) -> Result<String, String> {
    let mut files: Vec<PathBuf> = ignore::WalkBuilder::new(path)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .take(MAX_DIR_FILES)
        .collect();
    files.sort();
    let mut out = String::new();
    for f in files {
        if let Ok(Some(t)) = file_text(&f) {
            let rel = f
                .strip_prefix(path)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            let _ = write!(out, "\n\n# {rel}\n\n{t}");
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Ok(out.trim().to_owned())
}

/// A page fetched over HTTP(S) through the network guard.
async fn fetch(url: &str) -> Result<String, String> {
    kor_core::net::check_url(url).map_err(|b| b.message)?;
    let http = kor_core::net::client_builder()
        .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| match kor_core::net::blocked_in(&e) {
            Some(b) => b.message.clone(),
            None => kor_core::net::describe(&e),
        })?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!(
            "HTTP {}: the page needs a login; add it with --browser (your signed-in Chrome)",
            status.as_u16()
        ));
    }
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_SOURCE {
        return Err("larger than 50 MB".into());
    }
    Ok(
        if ctype.contains("pdf") || url.to_ascii_lowercase().ends_with(".pdf") {
            pdf_text(&bytes)?
        } else if ctype.contains("html") || ctype.is_empty() {
            html_text(&String::from_utf8_lossy(&bytes))
        } else {
            tidy(&String::from_utf8_lossy(&bytes))
        },
    )
}

/// A page read through the user's own Chrome (their login), via the paired extension.
async fn fetch_browser(url: &str) -> Result<String, String> {
    // A background tab in Kor's own tab group (ADR 0058), closed after reading.
    let (_title, text) = kor_tools::chrome::read_page(url).await?;
    Ok(tidy(&text))
}

/// The text and title of a source.
///
/// # Errors
/// Unsupported type, unreadable file, refused or failed fetch.
pub async fn extract(source: &str, browser: bool) -> Result<(String, String), String> {
    if is_url(source) {
        let text = if browser {
            fetch_browser(source).await?
        } else {
            fetch(source).await?
        };
        let title = source
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(source)
            .to_owned();
        return Ok((text, title));
    }
    let p = Path::new(source);
    let title = p
        .file_name()
        .map_or_else(|| source.to_owned(), |n| n.to_string_lossy().into_owned());
    if p.is_dir() {
        return Ok((dir_text(p)?, title));
    }
    match file_text(p)? {
        Some(t) => Ok((t, title)),
        None => Err(format!(
            "{source}: use .md, .txt, .html, .pdf, .docx or a folder"
        )),
    }
}

/// Chunks of about [`CHUNK`] characters on paragraph boundaries.
pub fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        if cur.chars().count() + para.chars().count() > CHUNK && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        // A single huge paragraph is cut by characters.
        if para.chars().count() > CHUNK {
            let cs: Vec<char> = para.chars().collect();
            for part in cs.chunks(CHUNK) {
                out.push(part.iter().collect());
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
        if out.len() >= MAX_CHUNKS {
            break;
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.truncate(MAX_CHUNKS);
    out
}

fn sha(text: &str) -> String {
    let d = Sha256::digest(text.as_bytes());
    d.iter().fold(String::new(), |mut s, b| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        s
    })
}

/// Store `text` as the chunks of `entry` and return the record ids (newest records of the
/// project whose `files` name this source).
fn store(k: &Knowledge, root: &Path, entry: &Entry, text: &str) -> Result<Vec<i64>, String> {
    let project = kor_memory::project_key(root);
    let parts = chunks(text);
    let n = parts.len();
    let records: Vec<Record> = parts
        .into_iter()
        .enumerate()
        .map(|(i, c)| Record {
            ts: now(),
            project: project.clone(),
            kind: "doc".into(),
            task: if n > 1 {
                format!("{} ({}/{n})", entry.title, i + 1)
            } else {
                entry.title.clone()
            },
            summary: c,
            files: vec![entry.source.clone()],
            ..Record::default()
        })
        .collect();
    k.record_all(records).map_err(|e| e.to_string())?;
    k.flush().map_err(|e| e.to_string())?;
    let ids = k
        .recent(Some(&project), n + 200)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| r.kind == "doc" && r.files.first() == Some(&entry.source))
        .map(|r| r.id)
        .take(n)
        .collect();
    Ok(ids)
}

fn forget(k: &Knowledge, ids: &[i64]) {
    for id in ids {
        let _ = k.forget(*id);
    }
}

/// Add (or re-add) a source: extract, index, write `.kor/docs/<id>.md`.
///
/// # Errors
/// Extraction or storage errors.
pub async fn add(k: &Knowledge, root: &Path, source: &str, browser: bool) -> Result<Entry, String> {
    let source = if is_url(source) {
        source.to_owned()
    } else {
        let p = Path::new(source);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        kor_tools::without_verbatim(&abs.canonicalize().map_err(|e| format!("{source}: {e}"))?)
            .to_string_lossy()
            .into_owned()
    };
    let (text, title) = extract(&source, browser).await?;
    if text.trim().is_empty() {
        return Err(format!("{source}: no text found"));
    }
    let mut index = read_index(root);
    let old = index.docs.iter().position(|d| d.source == source);
    let id = if let Some(i) = old {
        index.docs[i].id.clone()
    } else {
        let base = slug(&title);
        let mut id = base.clone();
        let mut n = 2;
        while index.docs.iter().any(|d| d.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    };
    if let Some(i) = old {
        forget(k, &index.docs[i].records);
    }
    let mut entry = Entry {
        id,
        source: source.clone(),
        title,
        sha256: sha(&text),
        added: old.map_or_else(now, |i| index.docs[i].added),
        updated: now(),
        chars: text.chars().count(),
        records: Vec::new(),
        browser,
    };
    entry.records = store(k, root, &entry, &text)?;
    std::fs::create_dir_all(dir(root)).map_err(|e| e.to_string())?;
    std::fs::write(
        dir(root).join(format!("{}.md", entry.id)),
        format!("# {}\n\nSource: {}\n\n{text}\n", entry.title, entry.source),
    )
    .map_err(|e| e.to_string())?;
    match old {
        Some(i) => index.docs[i] = entry.clone(),
        None => index.docs.push(entry.clone()),
    }
    write_index(root, &index)?;
    Ok(entry)
}

/// Re-index every source whose text changed (or `only` that one). Returns `(id, changed)`.
///
/// # Errors
/// When the index cannot be written; a source that fails is reported as an error string.
pub async fn refresh(
    k: &Knowledge,
    root: &Path,
    only: Option<&str>,
) -> Result<Vec<(String, Result<bool, String>)>, String> {
    let index = read_index(root);
    let mut out = Vec::new();
    for d in index.docs.iter().filter(|d| only.is_none_or(|o| o == d.id)) {
        let r = match extract(&d.source, d.browser).await {
            Ok((text, _)) if sha(&text) == d.sha256 => Ok(false),
            Ok(_) => add(k, root, &d.source, d.browser).await.map(|_| true),
            Err(e) => Err(e),
        };
        out.push((d.id.clone(), r));
    }
    Ok(out)
}

/// Remove a document and its records.
///
/// # Errors
/// Unknown id or the index cannot be written.
pub fn remove(k: &Knowledge, root: &Path, id: &str) -> Result<(), String> {
    let mut index = read_index(root);
    let i = index
        .docs
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| format!("unknown document `{id}`"))?;
    let e = index.docs.remove(i);
    forget(k, &e.records);
    let _ = std::fs::remove_file(dir(root).join(format!("{}.md", e.id)));
    write_index(root, &index)
}

/// Document chunks matching `query` in this project (best first).
///
/// # Errors
/// On a database error.
pub fn search(
    k: &Knowledge,
    root: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<kor_memory::Hit>, String> {
    let project = kor_memory::project_key(root);
    let hits = k
        .recall(query, Some(&project), None, (limit * 4).clamp(1, 200))
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .filter(|h| h.record.kind == "doc")
        .take(limit)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::similar_names)] // paired names (in/out, old/new) read best side by side
    fn pdf(text: &str) -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(
            dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier"},
        );
        let resources_id = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font_id}});
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(
            dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => content_id},
        );
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1, "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]}));
        let catalog_id = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", catalog_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[allow(clippy::format_collect)] // test fixture
    fn docx(paragraphs: &[&str]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("word/document.xml", o).unwrap();
            let body: String = paragraphs
                .iter()
                .map(|p| format!("<w:p><w:r><w:t xml:space=\"preserve\">{p}</w:t></w:r></w:p>"))
                .collect();
            std::io::Write::write_all(
                &mut z,
                format!("<?xml version=\"1.0\"?><w:document><w:body>{body}</w:body></w:document>")
                    .as_bytes(),
            )
            .unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn extractors() {
        let h = html_text(
            "<html><head><title>x</title><style>p{}</style></head><body><h1>Spec</h1><p>Rate&nbsp;limit &amp; retry</p><script>evil()</script><ul><li>a</li><li>b</li></ul></body></html>",
        );
        assert_eq!(h, "Spec\nRate limit & retry\na\nb");
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("s.pdf"), pdf("Payments retry three times")).unwrap();
        let t = file_text(&d.path().join("s.pdf")).unwrap().unwrap();
        assert!(t.contains("Payments retry three times"), "{t}");
        std::fs::write(
            d.path().join("w.docx"),
            docx(&["First para", "Second & last"]),
        )
        .unwrap();
        assert_eq!(
            file_text(&d.path().join("w.docx")).unwrap().unwrap(),
            "First para\nSecond & last"
        );
        assert!(file_text(&d.path().join("x.exe")).unwrap().is_none());
        let long = "word ".repeat(1000);
        let c = chunks(&format!("a\n\n{long}\n\nb"));
        assert!(c.len() >= 3 && c.iter().all(|x| x.chars().count() <= CHUNK));
    }

    #[tokio::test]
    async fn add_search_refresh_remove() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("proj");
        std::fs::create_dir_all(root.join("spec")).unwrap();
        std::fs::write(
            root.join("spec/api.md"),
            "# API\n\nThe ledger endpoint is idempotent.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("spec/ops.html"),
            "<p>Deploys happen on Tuesdays.</p>",
        )
        .unwrap();
        let k = Knowledge::open(&d.path().join("kb.sqlite")).unwrap();
        let e = add(&k, &root, "spec", false).await.unwrap();
        assert_eq!(e.id, "spec");
        assert!(!e.records.is_empty());
        assert!(dir(&root).join("spec.md").is_file());
        let hits = search(&k, &root, "ledger idempotent", 5).unwrap();
        assert!(hits[0].record.summary.contains("idempotent"));
        assert_eq!(list(&root).len(), 1);
        // Unchanged source: nothing re-indexed; changed: new records.
        assert_eq!(refresh(&k, &root, None).await.unwrap()[0].1, Ok(false));
        std::fs::write(
            root.join("spec/api.md"),
            "# API\n\nThe ledger endpoint retries.\n",
        )
        .unwrap();
        assert_eq!(
            refresh(&k, &root, Some("spec")).await.unwrap()[0].1,
            Ok(true)
        );
        assert!(search(&k, &root, "idempotent", 5).unwrap().is_empty());
        remove(&k, &root, "spec").unwrap();
        assert!(list(&root).is_empty());
        assert!(search(&k, &root, "retries", 5).unwrap().is_empty());
        assert!(add(&k, &root, "missing.pdf", false).await.is_err());
    }

    #[tokio::test]
    async fn urls_through_the_guard() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/wiki/Runbook"))
            .respond_with(
                ResponseTemplate::new(200)
                    // `set_body_string` would override the content type with text/plain.
                    .set_body_raw("<h1>Runbook</h1><p>Restart the worker.</p>", "text/html"),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/private"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&s)
            .await;
        let (t, title) = extract(&format!("{}/wiki/Runbook", s.uri()), false)
            .await
            .unwrap();
        assert_eq!(
            (t.as_str(), title.as_str()),
            ("Runbook\nRestart the worker.", "Runbook")
        );
        let e = extract(&format!("{}/private", s.uri()), false)
            .await
            .unwrap_err();
        assert!(e.contains("--browser"), "{e}");
    }
}
