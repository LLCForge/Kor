//! `Read`, `Write`, and `Edit`, plus `list`.
//!
//! Paths stay inside the project root, plus any `extra_roots` (auto memory). `Edit` and `Write` of an existing file require
//! a prior full `Read` of that file.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jail::rel;
use crate::{ReadStamp, Tool, ToolCtx, ToolKind, ToolOutput, field, lock, parse};

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 2000;
const MAX_LIST: usize = 1000;
const FILE_UNCHANGED: &str = "File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current — refer to that instead of re-reading.";

const DEVICES: &[&str] = &[
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/full",
    "/dev/stdin",
    "/dev/tty",
    "/dev/console",
];

fn path_of(input: &Value) -> Option<String> {
    field(input, &["file_path", "path"]).map(str::to_owned)
}

async fn read_pdf(path: &Path, pages: Option<&str>) -> ToolOutput {
    let range = if let Some(spec) = pages {
        match crate::pdf::parse_range(spec) {
            Ok(r) => Some(r),
            Err(e) => return ToolOutput::err(e),
        }
    } else {
        None
    };
    let out = std::env::temp_dir().join(format!("kor-pdf-{}", std::process::id()));
    match crate::pdf::render(path, &out, range).await {
        Ok(n) => ToolOutput::ok(format!(
            "PDF pages extracted: {n} page(s) from {} into {}",
            crate::display_path(path),
            crate::display_path(&out)
        )),
        Err(e) => ToolOutput::err(e),
    }
}

fn blocked(input: &str) -> Option<&'static str> {
    let norm = input.replace('\\', "/");
    if input.starts_with(r"\\") || norm.starts_with("//") {
        return Some("UNC paths are refused");
    }
    let lower = norm.to_ascii_lowercase();
    if DEVICES
        .iter()
        .any(|d| lower == *d || lower.starts_with("/dev/fd/"))
    {
        return Some("this device file would block or produce infinite output");
    }
    None
}

pub(super) fn resolve_in(ctx: &ToolCtx, input: &str) -> Result<PathBuf, ToolOutput> {
    if let Some(why) = blocked(input) {
        return Err(ToolOutput::err(format!("{why}: {input}")));
    }
    ctx.resolve(input).map_err(ToolOutput::err)
}

/// For previews: the resolved path when the write tools would accept writing `content` there.
pub(super) fn writable(ctx: &ToolCtx, input: &str, content: &str) -> Option<PathBuf> {
    let path = resolve_in(ctx, input).ok()?;
    let refused = content.len() > MAX_WRITE_BYTES
        || path.extension().is_some_and(|e| e == "ipynb")
        || ctx
            .team_memory
            .as_ref()
            .is_some_and(|t| crate::secret_scan::block_team_write(&path, t, content).is_some());
    (!refused).then_some(path)
}

async fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let meta = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!("file is larger than {MAX_READ_BYTES} bytes"));
    }
    tokio::fs::read(path).await.map_err(|e| e.to_string())
}

pub(super) fn mtime_ms(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_millis())
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// Line numbers: six-wide padded number and `→`.
pub(crate) fn add_line_numbers(lines: &[&str], start: usize) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let n = (start + i).to_string();
            let line: String = line.chars().take(MAX_LINE_CHARS).collect();
            if n.len() >= 6 {
                format!("{n}→{line}")
            } else {
                format!("{n:>6}→{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn remember(ctx: &ToolCtx, path: &Path, stamp: ReadStamp) {
    lock(&ctx.read_files).insert(path.to_owned(), stamp);
}

pub(super) fn require_fresh_read(ctx: &ToolCtx, path: &Path, content: &str) -> Result<(), String> {
    let stamp = lock(&ctx.read_files).get(path).cloned();
    let Some(stamp) = stamp else {
        return Err("File has not been read yet. Read it first before writing to it.".into());
    };
    if stamp.partial || !stamp.from_read {
        return Err("File has not been read yet. Read it first before writing to it.".into());
    }
    if mtime_ms(path) > stamp.mtime_ms && content != stamp.content {
        return Err(
            "File has been modified since read, either by the user or by a linter. Read it again before attempting to write it."
                .into(),
        );
    }
    Ok(())
}

fn normalize_quotes(s: &str) -> String {
    s.replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
}

/// Exact match, then a match after curly quotes are folded to straight quotes.
fn find_actual_string(file: &str, search: &str) -> Option<String> {
    if search.is_empty() {
        return None;
    }
    if file.contains(search) {
        return Some(search.to_owned());
    }
    let file_n = normalize_quotes(file);
    let search_n = normalize_quotes(search);
    let byte = file_n.find(&search_n)?;
    let char_at = file_n[..byte].chars().count();
    let char_len = search_n.chars().count();
    let start = file
        .char_indices()
        .nth(char_at)
        .map_or(file.len(), |(i, _)| i);
    let end = file
        .char_indices()
        .nth(char_at + char_len)
        .map_or(file.len(), |(i, _)| i);
    Some(file[start..end].to_owned())
}

fn preserve_quote_style(old: &str, actual: &str, new: &str) -> String {
    if old == actual {
        return new.to_owned();
    }
    let doubles = actual.contains('\u{201c}') || actual.contains('\u{201d}');
    let singles = actual.contains('\u{2018}') || actual.contains('\u{2019}');
    let mut out = String::new();
    let mut open_d = true;
    let mut open_s = true;
    for c in new.chars() {
        match c {
            '"' if doubles => {
                out.push(if open_d { '\u{201c}' } else { '\u{201d}' });
                open_d = !open_d;
            }
            '\'' if singles => {
                out.push(if open_s { '\u{2018}' } else { '\u{2019}' });
                open_s = !open_s;
            }
            other => out.push(other),
        }
    }
    out
}

fn to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Apply one replacement. `file` may use CRLF; the result keeps that ending.
pub(super) fn apply_one(
    file: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<String, String> {
    if old == new {
        return Err("No changes to make: old_string and new_string are exactly the same.".into());
    }
    let crlf = file.contains("\r\n");
    let file_lf = to_lf(file);
    let old_lf = to_lf(old);
    let new_lf = to_lf(new);
    let actual = find_actual_string(&file_lf, &old_lf)
        .ok_or_else(|| format!("String to replace not found in file.\nString: {old}"))?;
    let matches = file_lf.matches(&actual).count();
    if matches > 1 && !replace_all {
        return Err(format!(
            "Found {matches} matches of the string to replace, but replace_all is false. To replace all occurrences, set replace_all to true. To replace only one occurrence, please provide more context to uniquely identify the instance.\nString: {old}"
        ));
    }
    let replacement = preserve_quote_style(&old_lf, &actual, &new_lf);
    let replaced = if replace_all {
        file_lf.replace(&actual, &replacement)
    } else {
        file_lf.replacen(&actual, &replacement, 1)
    };
    Ok(if crlf {
        replaced.replace('\n', "\r\n")
    } else {
        replaced
    })
}

/// Text of `path`: the editor's buffer when an ACP editor offers reads (unsaved changes
/// included, ADR 0057), else the file on disk.
pub(super) async fn read_text(ctx: &ToolCtx, path: &Path) -> std::io::Result<String> {
    if let Some(e) = ctx.editor.as_ref().filter(|e| e.can_read())
        && let Ok(t) = e.read_text(path).await
    {
        return Ok(t);
    }
    tokio::fs::read_to_string(path).await
}

/// Write text through the ACP editor when it offers writes (its undo works), else atomically.
pub(super) async fn write_text(ctx: &ToolCtx, path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(e) = ctx.editor.as_ref().filter(|e| e.can_write()) {
        return e
            .write_text(path, content)
            .await
            .map_err(std::io::Error::other);
    }
    write_atomic(path, content.as_bytes()).await
}

pub(super) async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.kor-tmp"));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await.inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn image_type(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn notebook_text(bytes: &[u8]) -> Result<String, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("invalid notebook: {e}"))?;
    let cells = v["cells"].as_array().ok_or("notebook has no cells")?;
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        let kind = cell["cell_type"].as_str().unwrap_or("code");
        let id = cell["id"].as_str().unwrap_or("");
        let source = match &cell["source"] {
            Value::String(s) => s.clone(),
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => String::new(),
        };
        let _ = writeln!(out, "--- cell {i} ({kind}) id={id}");
        out.push_str(&source);
        if !source.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub(crate) struct ReadFile;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadIn {
    #[serde(alias = "path")]
    file_path: String,
    /// 1-based first line.
    offset: Option<usize>,
    limit: Option<usize>,
    pages: Option<String>,
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "Read"
    }
    fn rule_name(&self) -> &'static str {
        "Read"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: format!(
                "Read a file from the local filesystem. file_path may be absolute or relative to the project root; Kor cannot leave the project. \
                 Returns cat -n style lines (`     N→line`), up to {MAX_LINES} lines. Use offset and limit for the rest. \
                 Images (png, jpeg, gif, webp) come back as base64. Jupyter notebooks come back as cells. \
                 PDF page extraction is not available."
            ),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "offset": {"type": "integer", "minimum": 1, "description": "1-based line to start at"},
                "limit": {"type": "integer", "minimum": 1, "description": "Number of lines to read"},
                "pages": {"type": "string", "description": "PDF page range such as 1-5, rendered with pdftoppm when it is installed"}
            }, "required": ["file_path"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        path_of(input)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: ReadIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "pdf" {
            return ToolOutput::err(
                "PDF files are not read in this build. Convert the PDF to text first.",
            );
        }
        let bytes = match read_bytes(&path).await {
            Ok(b) => b,
            Err(e) => {
                if e.contains("cannot find")
                    || e.contains("os error 2")
                    || e.contains("Not Found")
                    || e.contains("not found")
                {
                    return ToolOutput::err(format!(
                        "File does not exist. Note the current working directory is the project root: {}.",
                        crate::display_path(&ctx.root)
                    ));
                }
                return ToolOutput::err(format!("{}: {e}", a.file_path));
            }
        };
        if ext == "pdf" {
            return read_pdf(&path, a.pages.as_deref()).await;
        }
        if a.pages.is_some() {
            return ToolOutput::err(
                "pages is only valid for PDF files. Use formats like \"1-5\", \"3\", or \"10-20\".",
            );
        }
        if let Some(mime) = image_type(&ext) {
            if bytes.len() > 1_500_000 {
                return ToolOutput::err("image is larger than 1.5 MB");
            }
            return ToolOutput::ok(format!(
                "[image {mime}, {} bytes]\n{}",
                bytes.len(),
                base64(&bytes)
            ));
        }
        if ext == "ipynb" {
            return match notebook_text(&bytes) {
                Ok(t) => {
                    remember(
                        ctx,
                        &path,
                        ReadStamp {
                            mtime_ms: mtime_ms(&path),
                            partial: false,
                            from_read: true,
                            content: String::from_utf8_lossy(&bytes).into_owned(),
                            offset: 1,
                            limit: None,
                        },
                    );
                    ToolOutput::ok(t)
                }
                Err(e) => ToolOutput::err(e),
            };
        }
        if bytes.iter().take(8192).any(|&b| b == 0) {
            return ToolOutput::err(format!("{}: binary file", a.file_path));
        }
        let bytes = match ctx.editor.as_ref().filter(|e| e.can_read()) {
            Some(e) => e.read_text(&path).await.map_or(bytes, String::into_bytes),
            None => bytes,
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return ToolOutput::err(format!("{}: file is not valid UTF-8", a.file_path));
        };
        let start = a.offset.unwrap_or(1).max(1);
        let lines = split_lines(&text);
        let total = lines.len();
        if total == 0 {
            remember(
                ctx,
                &path,
                ReadStamp {
                    mtime_ms: mtime_ms(&path),
                    partial: false,
                    from_read: true,
                    content: text,
                    offset: 1,
                    limit: a.limit,
                },
            );
            return ToolOutput::ok(
                "<system-reminder>Warning: the file exists but the contents are empty.</system-reminder>",
            );
        }
        if start > total {
            return ToolOutput::ok(format!(
                "<system-reminder>Warning: the file exists but is shorter than the provided offset ({start}). The file has {total} lines.</system-reminder>"
            ));
        }
        let take = a.limit.unwrap_or(MAX_LINES).max(1);
        let slice: Vec<&str> = lines.iter().copied().skip(start - 1).take(take).collect();
        let partial = start != 1 || slice.len() < total - (start - 1);
        let unchanged = {
            let stamps = lock(&ctx.read_files);
            stamps.get(&path).is_some_and(|s| {
                s.from_read
                    && !s.partial
                    && !partial
                    && s.offset == start
                    && s.limit == a.limit
                    && s.mtime_ms == mtime_ms(&path)
            })
        };
        if unchanged {
            return ToolOutput::ok(FILE_UNCHANGED);
        }
        remember(
            ctx,
            &path,
            ReadStamp {
                mtime_ms: mtime_ms(&path),
                partial,
                from_read: true,
                content: text.clone(),
                offset: start,
                limit: a.limit,
            },
        );
        let mut out = add_line_numbers(&slice, start);
        let shown_end = start - 1 + slice.len();
        if shown_end < total {
            let _ = write!(
                out,
                "\n… {} more lines; use offset and limit to continue",
                total - shown_end
            );
        }
        ToolOutput::ok(out)
    }
}

#[derive(Debug)]
pub(crate) struct WriteFile;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteIn {
    #[serde(alias = "path")]
    file_path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "Write"
    }
    fn rule_name(&self) -> &'static str {
        "Write"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Write a file, creating it if needed. An existing non-empty file must have been Read first. Prefer Edit for small changes.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "content": {"type": "string"}
            }, "required": ["file_path", "content"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        path_of(input)
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<super::review::Proposal>> {
        let a: WriteIn = parse(input.clone()).ok()?;
        let path = writable(ctx, &a.file_path, &a.content)?;
        let before = tokio::fs::read_to_string(&path).await.ok();
        if let Some(b) = before.as_deref().filter(|b| !b.trim().is_empty()) {
            require_fresh_read(ctx, &path, b).ok()?;
        }
        Some(vec![super::review::proposal(ctx, &path, before, Some(a.content))])
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: WriteIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.content.len() > MAX_WRITE_BYTES {
            return ToolOutput::err(format!("content exceeds {MAX_WRITE_BYTES} bytes"));
        }
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if path.extension().is_some_and(|e| e == "ipynb") {
            return ToolOutput::err(
                "File is a Jupyter Notebook. Use the NotebookEdit tool to edit this file.",
            );
        }
        if let Some(team) = &ctx.team_memory
            && let Some(msg) = crate::secret_scan::block_team_write(&path, team, &a.content)
        {
            return ToolOutput::err(msg);
        }
        if let Ok(existing) = read_text(ctx, &path).await
            && !existing.trim().is_empty()
            && let Err(e) = require_fresh_read(ctx, &path, &existing)
        {
            return ToolOutput::err(e);
        }
        if let Some(dir) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(dir).await
        {
            return ToolOutput::err(e.to_string());
        }
        match write_text(ctx, &path, &a.content).await {
            Ok(()) => {
                remember(
                    ctx,
                    &path,
                    ReadStamp {
                        mtime_ms: mtime_ms(&path),
                        partial: false,
                        from_read: false,
                        content: a.content.clone(),
                        offset: 1,
                        limit: None,
                    },
                );
                ToolOutput::ok(format!(
                    "wrote {} ({} lines)",
                    rel(&ctx.root, &path),
                    a.content.lines().count()
                ))
            }
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Edit;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditIn {
    #[serde(alias = "path")]
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for Edit {
    fn name(&self) -> &'static str {
        "Edit"
    }
    fn rule_name(&self) -> &'static str {
        "Edit"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Replace exact text in a file. old_string must match once unless replace_all is true. The file must have been Read in full first. An empty old_string creates a file that does not exist yet. Do not use this on .ipynb files.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "old_string": {"type": "string"},
                "new_string": {"type": "string"},
                "replace_all": {"type": "boolean"}
            }, "required": ["file_path", "old_string", "new_string"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        path_of(input)
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<super::review::Proposal>> {
        let a: EditIn = parse(input.clone()).ok()?;
        let path = writable(ctx, &a.file_path, &a.new_string)?;
        let before = tokio::fs::read_to_string(&path).await.ok();
        let after = match before.as_deref() {
            None | Some("") if a.old_string.is_empty() => a.new_string,
            Some(f) if a.old_string.is_empty() && f.trim().is_empty() => a.new_string,
            Some(f) if !a.old_string.is_empty() => {
                require_fresh_read(ctx, &path, f).ok()?;
                apply_one(f, &a.old_string, &a.new_string, a.replace_all).ok()?
            }
            _ => return None,
        };
        Some(vec![super::review::proposal(ctx, &path, before, Some(after))])
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: EditIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if path.extension().is_some_and(|e| e == "ipynb") {
            return ToolOutput::err(
                "File is a Jupyter Notebook. Use the NotebookEdit tool to edit this file.",
            );
        }
        if let Some(team) = &ctx.team_memory
            && let Some(msg) = crate::secret_scan::block_team_write(&path, team, &a.new_string)
        {
            return ToolOutput::err(msg);
        }
        let existing = read_text(ctx, &path).await.ok();
        if existing.is_none() {
            if a.old_string.is_empty() {
                if let Some(dir) = path.parent()
                    && let Err(e) = tokio::fs::create_dir_all(dir).await
                {
                    return ToolOutput::err(e.to_string());
                }
                return match write_atomic(&path, a.new_string.as_bytes()).await {
                    Ok(()) => {
                        remember(
                            ctx,
                            &path,
                            ReadStamp {
                                mtime_ms: mtime_ms(&path),
                                partial: false,
                                from_read: false,
                                content: a.new_string.clone(),
                                offset: 1,
                                limit: None,
                            },
                        );
                        ToolOutput::ok(format!("created {}", rel(&ctx.root, &path)))
                    }
                    Err(e) => ToolOutput::err(e.to_string()),
                };
            }
            return ToolOutput::err(format!(
                "File does not exist. Note the current working directory is the project root: {}.",
                crate::display_path(&ctx.root)
            ));
        }
        let file = existing.unwrap_or_default();
        if a.old_string.is_empty() {
            if file.trim().is_empty() {
                return match write_atomic(&path, a.new_string.as_bytes()).await {
                    Ok(()) => ToolOutput::ok(format!("wrote {}", rel(&ctx.root, &path))),
                    Err(e) => ToolOutput::err(e.to_string()),
                };
            }
            return ToolOutput::err("Cannot create new file - file already exists.");
        }
        if let Err(e) = require_fresh_read(ctx, &path, &file) {
            return ToolOutput::err(e);
        }
        match apply_one(&file, &a.old_string, &a.new_string, a.replace_all) {
            Ok(new) => match write_text(ctx, &path, &new).await {
                Ok(()) => {
                    remember(
                        ctx,
                        &path,
                        ReadStamp {
                            mtime_ms: mtime_ms(&path),
                            partial: false,
                            from_read: false,
                            content: new,
                            offset: 1,
                            limit: None,
                        },
                    );
                    ToolOutput::ok(format!("edited {}", rel(&ctx.root, &path)))
                }
                Err(e) => ToolOutput::err(e.to_string()),
            },
            Err(e) => ToolOutput::err(e),
        }
    }
}

#[derive(Debug)]
pub(crate) struct List;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListIn {
    path: Option<String>,
}

#[async_trait]
impl Tool for List {
    fn name(&self) -> &'static str {
        "list"
    }
    fn rule_name(&self) -> &'static str {
        "LS"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "List a directory (default: project root). Directories end with `/`."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "additionalProperties": false
            }),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        Some(field(input, &["path"]).unwrap_or(".").to_owned())
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: ListIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, a.path.as_deref().unwrap_or(".")) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let mut rd = match tokio::fs::read_dir(&path).await {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let mut names = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            let mut n = e.file_name().to_string_lossy().into_owned();
            if e.file_type().await.is_ok_and(|t| t.is_dir()) {
                n.push('/');
            }
            names.push(n);
        }
        names.sort();
        let more = names.len().saturating_sub(MAX_LIST);
        names.truncate(MAX_LIST);
        let mut out = names.join("\n");
        if more > 0 {
            let _ = write!(out, "\n… {more} more");
        }
        ToolOutput::ok(if out.is_empty() {
            "(empty)".into()
        } else {
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_match_claude_cat_n() {
        let lines = ["hello", "world"];
        assert_eq!(add_line_numbers(&lines, 1), "     1→hello\n     2→world");
        assert_eq!(add_line_numbers(&["x"], 100_000), "100000→x");
    }

    #[test]
    fn edit_semantics() {
        assert_eq!(apply_one("a b c", "b", "B", false).unwrap(), "a B c");
        assert!(
            apply_one("a a", "a", "b", false)
                .unwrap_err()
                .contains("2 matches")
        );
        assert_eq!(apply_one("a a", "a", "b", true).unwrap(), "b b");
        assert!(apply_one("x", "y", "z", false).is_err());
        assert!(
            apply_one("x", "x", "x", false)
                .unwrap_err()
                .contains("exactly the same")
        );
    }

    #[test]
    fn edit_preserves_crlf_and_curly_quotes() {
        assert_eq!(
            apply_one("a\r\nb\r\n", "a\nb", "x\ny", false).unwrap(),
            "x\r\ny\r\n"
        );
        let file = "say \u{201c}hi\u{201d}";
        let out = apply_one(file, "say \"hi\"", "say \"yo\"", false).unwrap();
        assert!(out.contains('\u{201c}') && out.contains("yo"), "{out}");
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[tokio::test]
    async fn tools_work_inside_root_and_refuse_outside() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();

        let w = WriteFile
            .call(
                json!({"file_path": "src/a.txt", "content": "hello\nworld\n"}),
                &ctx,
            )
            .await;
        assert!(!w.is_error, "{}", w.content);
        let r = ReadFile.call(json!({"file_path": "src/a.txt"}), &ctx).await;
        assert_eq!(r.content, "     1→hello\n     2→world");
        let again = ReadFile.call(json!({"file_path": "src/a.txt"}), &ctx).await;
        assert!(
            again.content.contains("File unchanged"),
            "{}",
            again.content
        );
        let r = ReadFile
            .call(json!({"file_path": "src/a.txt", "offset": 2}), &ctx)
            .await;
        assert_eq!(r.content, "     2→world");

        let unread = Edit
            .call(
                json!({"file_path": "src/a.txt", "old_string": "world", "new_string": "kor"}),
                &ctx,
            )
            .await;
        assert!(
            unread.is_error && unread.content.contains("has not been read"),
            "{}",
            unread.content
        );
        // The offset read was partial, so a full read is required.
        let _ = ReadFile.call(json!({"file_path": "src/a.txt"}), &ctx).await;
        // mtime may match the first full read; force a content-visible read after the partial one.
        let ed = Edit
            .call(
                json!({"file_path": "src/a.txt", "old_string": "world", "new_string": "kor"}),
                &ctx,
            )
            .await;
        assert!(!ed.is_error, "{}", ed.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/a.txt")).unwrap(),
            "hello\nkor\n"
        );

        let l = List.call(json!({}), &ctx).await;
        assert_eq!(l.content, "src/");

        assert!(
            ReadFile
                .call(json!({"file_path": "../x"}), &ctx)
                .await
                .is_error
        );
        assert!(
            WriteFile
                .call(json!({"file_path": "../x", "content": ""}), &ctx)
                .await
                .is_error
        );
        assert!(
            ReadFile
                .call(json!({"file_path": "a", "bogus": 1}), &ctx)
                .await
                .is_error
        );
        assert!(
            ReadFile
                .call(json!({"file_path": "//server/share"}), &ctx)
                .await
                .is_error
        );

        let empty = WriteFile
            .call(json!({"file_path": "empty.txt", "content": ""}), &ctx)
            .await;
        assert!(!empty.is_error, "{}", empty.content);
        let empty = ReadFile.call(json!({"file_path": "empty.txt"}), &ctx).await;
        assert!(
            empty.content.contains("contents are empty"),
            "{}",
            empty.content
        );

        let nb = r#"{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","id":"c1","source":["print(1)\n"],"metadata":{},"outputs":[]}]}"#;
        std::fs::write(dir.path().join("n.ipynb"), nb).unwrap();
        let nb_read = ReadFile.call(json!({"file_path": "n.ipynb"}), &ctx).await;
        assert!(nb_read.content.contains("print(1)"), "{}", nb_read.content);
        let nb_edit = Edit
            .call(
                json!({"file_path": "n.ipynb", "old_string": "print", "new_string": "x"}),
                &ctx,
            )
            .await;
        assert!(
            nb_edit.content.contains("NotebookEdit"),
            "{}",
            nb_edit.content
        );
    }

    #[tokio::test]
    async fn team_memory_refuses_an_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let mem = dir.path().join("memory");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(mem.join("team")).unwrap();
        let mut ctx = ToolCtx::new(&root).unwrap();
        let mem_c = mem.canonicalize().unwrap();
        ctx.extra_roots.push(mem_c.clone());
        ctx.team_memory = Some(mem_c.join("team"));
        let key = format!("sk-ant-api03-{}AA", "a".repeat(93));
        let path = crate::without_verbatim(&mem_c.join("team").join("MEMORY.md"));
        let denied = WriteFile
            .call(
                json!({"file_path": path.display().to_string(), "content": key}),
                &ctx,
            )
            .await;
        assert!(denied.is_error, "{}", denied.content);
        assert!(denied.content.contains("secrets"), "{}", denied.content);
        assert!(!path.exists());

        let ok = WriteFile
            .call(
                json!({"file_path": path.display().to_string(), "content": "- [Role](user_role.md) — prefers small patches\n"}),
                &ctx,
            )
            .await;
        assert!(!ok.is_error, "{}", ok.content);
        assert!(path.is_file());
    }
}
