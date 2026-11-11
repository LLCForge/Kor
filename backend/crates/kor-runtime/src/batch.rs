//! Batch "Kor ile düzenle" (Master Build Prompt §27.6, ADR 0052): one instruction over many
//! files selected in the file manager or given to `kor files`.
//!
//! 1. The model gets the file list and the instruction and answers with a JSON operation list
//!    ([`prompt`], [`parse_ops`]). It never touches a file itself.
//! 2. Kor checks every operation ([`validate`]): sources must be selected files, targets stay in
//!    the selection's folders, there is no delete operation at all, clashing names are numbered
//!    (`ad (2).ext`). The result is the preview table ([`Plan::rows`]).
//! 3. After the user confirms, [`apply`] runs it with a `kor undo` journal (every touched path's
//!    first state is recorded before the change), so `kor undo` reverts the whole batch.
//!
//! Operations: rename/move, image convert/resize (new file), EXIF/metadata strip (lossless for
//! JPEG/PNG; new file or in place), text find-replace, encoding fix to UTF-8, CSV clean, PDF merge
//! (new file), audio/video convert with `ffmpeg` (new file; built from fields, never raw
//! arguments).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Most files in one batch.
pub const MAX_FILES: usize = 500;
/// Longest instruction.
pub const MAX_INSTRUCTION: usize = 4 * 1024;
/// Text operations read at most this much.
const MAX_TEXT: u64 = 32 * 1024 * 1024;

/// One operation as the model proposes it (paths are file names / relative paths).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Rename or move `from` to `to` (relative to `from`'s folder).
    Rename { from: String, to: String },
    /// Write a converted/resized copy of an image.
    ConvertImage {
        from: String,
        to: String,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
    },
    /// Remove EXIF/XMP/text metadata; in place when `to` is missing.
    StripMetadata {
        from: String,
        #[serde(default)]
        to: Option<String>,
    },
    /// Find and replace in a UTF-8 text file (in place).
    Replace {
        file: String,
        find: String,
        replace: String,
        #[serde(default)]
        regex: bool,
    },
    /// Re-encode a text file from `encoding` (e.g. `windows-1254`, `latin1`, `utf-16le`) to UTF-8.
    Reencode { file: String, encoding: String },
    /// Clean a CSV file (in place, or into `to`).
    CsvClean {
        file: String,
        #[serde(default)]
        to: Option<String>,
        #[serde(default = "yes")]
        trim: bool,
        #[serde(default = "yes")]
        drop_empty: bool,
        #[serde(default)]
        dedupe: bool,
        /// Input delimiter (`,` `;` `\t` `|`); output is always `,`.
        #[serde(default)]
        delimiter: Option<String>,
    },
    /// Merge PDFs, in the given order, into a new file.
    PdfMerge { files: Vec<String>, to: String },
    /// Convert audio/video with `ffmpeg` into a new file.
    Media {
        from: String,
        to: String,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        audio_bitrate_k: Option<u32>,
        #[serde(default)]
        video_bitrate_k: Option<u32>,
        #[serde(default)]
        start_s: Option<f64>,
        #[serde(default)]
        duration_s: Option<f64>,
    },
}

fn yes() -> bool {
    true
}

/// A checked operation with absolute paths.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub op: Op,
    /// Inputs (selected files).
    pub sources: Vec<PathBuf>,
    /// Output path (new file, rename target, or the file changed in place).
    pub target: PathBuf,
    /// Output is a new file (never replaces anything).
    pub creates: bool,
}

/// One row of the preview table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    /// Operation code (`rename`, `convert_image`, …); front ends translate it.
    pub op: String,
    pub from: String,
    pub to: String,
    pub detail: String,
}

/// A validated plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub rows: Vec<Row>,
    /// Operations dropped by validation, with the reason.
    pub warnings: Vec<String>,
}

/// Asks the model (the real one uses the configured provider; tests script it).
#[async_trait]
pub trait Planner: Send + Sync {
    async fn ask(&self, prompt: String) -> Result<String, String>;
}

/// Asks `model` on `provider` once.
pub struct ProviderPlanner {
    pub provider: std::sync::Arc<dyn kor_providers::Provider>,
    pub model: String,
}

impl std::fmt::Debug for ProviderPlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderPlanner")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Planner for ProviderPlanner {
    async fn ask(&self, prompt: String) -> Result<String, String> {
        kor_agent::gitflow::ask(&self.provider, &self.model, prompt, 4096)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Expand globs and paths (relative to `cwd`) into existing files, sorted, deduplicated.
///
/// # Errors
/// A pattern that is not a valid glob, or too many files.
pub fn expand(patterns: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = BTreeSet::new();
    for pat in patterns {
        let p = Path::new(pat);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        };
        if abs.is_file() {
            out.insert(abs);
            continue;
        }
        if !pat.contains(['*', '?', '[', '{']) {
            return Err(format!("not found: {pat}"));
        }
        // Glob relative to the folder before the first wildcard.
        let norm = pat.replace('\\', "/");
        let cut = norm.find(['*', '?', '[', '{']).unwrap_or(0);
        let base_rel = norm[..cut].rsplit_once('/').map_or("", |(b, _)| b);
        let base = if Path::new(base_rel).is_absolute() {
            PathBuf::from(base_rel)
        } else {
            cwd.join(base_rel)
        };
        let glob_rel = norm[cut..].to_owned();
        let tail = norm[..cut]
            .rsplit_once('/')
            .map_or(&norm[..cut], |(_, t)| t);
        let m = globset::Glob::new(&format!("{tail}{glob_rel}"))
            .map_err(|e| format!("{pat}: {e}"))?
            .compile_matcher();
        let deep = glob_rel.contains("**") || glob_rel.contains('/');
        for e in ignore::WalkBuilder::new(&base)
            .max_depth(if deep { None } else { Some(1) })
            .hidden(true)
            .git_ignore(false)
            .build()
            .flatten()
        {
            let f = e.path();
            if f.is_file() && f.strip_prefix(&base).is_ok_and(|r| m.is_match(r)) {
                out.insert(f.to_path_buf());
            }
        }
    }
    if out.len() > MAX_FILES {
        return Err(format!(
            "too many files ({}; at most {MAX_FILES})",
            out.len()
        ));
    }
    Ok(out.into_iter().collect())
}

/// The prompt that asks the model for the operation list.
pub fn prompt(files: &[PathBuf], instruction: &str) -> String {
    let mut list = String::new();
    for f in files {
        let size = std::fs::metadata(f).map_or(0, |m| m.len());
        let _ = writeln!(list, "- {} ({size} bytes)", f.display());
    }
    format!(
        "You plan a batch file operation. Do NOT perform it; answer ONLY with a JSON array of operations.\n\
         Allowed operations (paths: use the exact source paths below; `to` is a file name or a path relative to the \
         source's folder; never use `..`):\n\
         {{\"op\":\"rename\",\"from\":SRC,\"to\":NEW}}\n\
         {{\"op\":\"convert_image\",\"from\":SRC,\"to\":NEW_WITH_EXT,\"width\":N?,\"height\":N?}} (png, jpg, webp, gif, bmp, tiff)\n\
         {{\"op\":\"strip_metadata\",\"from\":SRC,\"to\":NEW?}}\n\
         {{\"op\":\"replace\",\"file\":SRC,\"find\":TEXT,\"replace\":TEXT,\"regex\":false}}\n\
         {{\"op\":\"reencode\",\"file\":SRC,\"encoding\":\"windows-1254\"}}\n\
         {{\"op\":\"csv_clean\",\"file\":SRC,\"to\":NEW?,\"trim\":true,\"drop_empty\":true,\"dedupe\":false,\"delimiter\":\";\"}}\n\
         {{\"op\":\"pdf_merge\",\"files\":[SRC,...],\"to\":NEW}}\n\
         {{\"op\":\"media\",\"from\":SRC,\"to\":NEW_WITH_EXT,\"width\":N?,\"height\":N?,\"audio_bitrate_k\":N?,\
         \"video_bitrate_k\":N?,\"start_s\":N?,\"duration_s\":N?}} (ffmpeg)\n\
         There is no delete operation. If the instruction cannot be done with these, answer [].\n\n\
         Files:\n{list}\nInstruction: {instruction}\n"
    )
}

/// The JSON array in a model answer (fenced or bare).
///
/// # Errors
/// No JSON array of known operations.
pub fn parse_ops(answer: &str) -> Result<Vec<Op>, String> {
    let start = answer
        .find('[')
        .ok_or("the model did not answer with a JSON list")?;
    let end = answer
        .rfind(']')
        .filter(|e| *e > start)
        .ok_or("the model did not answer with a JSON list")?;
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(&answer[start..=end]).map_err(|e| format!("invalid JSON: {e}"))?;
    let mut ops = Vec::new();
    for v in raw {
        ops.push(serde_json::from_value(v.clone()).map_err(|e| format!("{v}: {e}"))?);
    }
    Ok(ops)
}

/// `path`, or `name (2).ext`, `name (3).ext`, … when it exists or is already `taken`.
pub fn unique_path(path: &Path, taken: &[PathBuf]) -> PathBuf {
    let free = |p: &Path| p.symlink_metadata().is_err() && !taken.iter().any(|t| same(t, p));
    if free(path) {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    // Bounded: a folder with ten thousand copies keeps the original name rather than spin.
    (2..10_000)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| free(p))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Paths compare case-insensitively on case-insensitive file systems (Windows, macOS).
fn same(a: &Path, b: &Path) -> bool {
    if kor_platform::os() == kor_platform::Os::Linux {
        a == b
    } else {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
}

/// Resolve `to` against `dir` and keep it inside the allowed folders.
fn target(dir: &Path, to: &str, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let t = Path::new(to.trim());
    if to.trim().is_empty() {
        return Err("empty target".into());
    }
    if t.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("`{to}`: only names or sub-folders are allowed"));
    }
    let p = dir.join(t);
    if !allowed.iter().any(|a| p.starts_with(a)) {
        return Err(format!("`{to}` leaves the selected folders"));
    }
    Ok(p)
}

fn ext_lower(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

const IMAGE_OUT: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff"];

/// Check the model's operations against the selection. Invalid ones become warnings.
#[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
pub fn validate(files: &[PathBuf], ops: Vec<Op>) -> Plan {
    let allowed: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = files
            .iter()
            .filter_map(|f| f.parent().map(Path::to_path_buf))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let find = |name: &str| -> Result<PathBuf, String> {
        let p = Path::new(name);
        files
            .iter()
            .find(|f| {
                same(f, p)
                    || (!p.is_absolute()
                        && f.file_name() == p.file_name()
                        && p.components().count() == 1)
            })
            .cloned()
            .ok_or_else(|| format!("`{name}` is not one of the selected files"))
    };
    let mut plan = Plan {
        steps: Vec::new(),
        rows: Vec::new(),
        warnings: Vec::new(),
    };
    let mut taken: Vec<PathBuf> = Vec::new();
    // A renamed source cannot be used by a later step.
    let mut moved: Vec<PathBuf> = Vec::new();
    for op in ops {
        let checked = (|| -> Result<Step, String> {
            match &op {
                Op::Rename { from, to } => {
                    let src = find(from)?;
                    let t = target(src.parent().unwrap_or(Path::new("")), to, &allowed)?;
                    if same(&t, &src) {
                        return Err(format!("`{from}`: same name"));
                    }
                    Ok(Step {
                        sources: vec![src],
                        target: t,
                        creates: true,
                        op: op.clone(),
                    })
                }
                Op::ConvertImage {
                    from,
                    to,
                    width,
                    height,
                } => {
                    let src = find(from)?;
                    let t = target(src.parent().unwrap_or(Path::new("")), to, &allowed)?;
                    if !IMAGE_OUT.contains(&ext_lower(&t).as_str()) {
                        return Err(format!("`{to}`: unsupported image type"));
                    }
                    if width.is_some_and(|w| w == 0 || w > 20_000)
                        || height.is_some_and(|h| h == 0 || h > 20_000)
                    {
                        return Err("size must be 1-20000 px".into());
                    }
                    Ok(Step {
                        sources: vec![src],
                        target: t,
                        creates: true,
                        op: op.clone(),
                    })
                }
                Op::StripMetadata { from, to } => {
                    let src = find(from)?;
                    match to {
                        Some(to) => {
                            let t = target(src.parent().unwrap_or(Path::new("")), to, &allowed)?;
                            Ok(Step {
                                sources: vec![src],
                                target: t,
                                creates: true,
                                op: op.clone(),
                            })
                        }
                        None => Ok(Step {
                            sources: vec![src.clone()],
                            target: src,
                            creates: false,
                            op: op.clone(),
                        }),
                    }
                }
                Op::Replace {
                    file,
                    find: needle,
                    regex,
                    ..
                } => {
                    let src = find(file)?;
                    if needle.is_empty() {
                        return Err("empty search text".into());
                    }
                    if *regex {
                        regex::Regex::new(needle).map_err(|e| e.to_string())?;
                    }
                    Ok(Step {
                        sources: vec![src.clone()],
                        target: src,
                        creates: false,
                        op: op.clone(),
                    })
                }
                Op::Reencode { file, encoding } => {
                    let src = find(file)?;
                    encoding_rs::Encoding::for_label(encoding.as_bytes())
                        .ok_or_else(|| format!("unknown encoding `{encoding}`"))?;
                    Ok(Step {
                        sources: vec![src.clone()],
                        target: src,
                        creates: false,
                        op: op.clone(),
                    })
                }
                Op::CsvClean {
                    file,
                    to,
                    delimiter,
                    ..
                } => {
                    let src = find(file)?;
                    if let Some(d) = delimiter
                        && csv_delim(d).is_none()
                    {
                        return Err(format!("delimiter `{d}`: use , ; tab or |"));
                    }
                    match to {
                        Some(to) => {
                            let t = target(src.parent().unwrap_or(Path::new("")), to, &allowed)?;
                            Ok(Step {
                                sources: vec![src],
                                target: t,
                                creates: true,
                                op: op.clone(),
                            })
                        }
                        None => Ok(Step {
                            sources: vec![src.clone()],
                            target: src,
                            creates: false,
                            op: op.clone(),
                        }),
                    }
                }
                Op::PdfMerge { files: list, to } => {
                    if list.len() < 2 {
                        return Err("merge needs at least two PDFs".into());
                    }
                    let srcs = list
                        .iter()
                        .map(|f| find(f))
                        .collect::<Result<Vec<_>, _>>()?;
                    if srcs.iter().any(|s| ext_lower(s) != "pdf") {
                        return Err("only PDF files can be merged".into());
                    }
                    let t = target(srcs[0].parent().unwrap_or(Path::new("")), to, &allowed)?;
                    if ext_lower(&t) != "pdf" {
                        return Err(format!("`{to}` must end in .pdf"));
                    }
                    Ok(Step {
                        sources: srcs,
                        target: t,
                        creates: true,
                        op: op.clone(),
                    })
                }
                Op::Media {
                    from,
                    to,
                    start_s,
                    duration_s,
                    ..
                } => {
                    let src = find(from)?;
                    let t = target(src.parent().unwrap_or(Path::new("")), to, &allowed)?;
                    if ext_lower(&t).is_empty() {
                        return Err(format!("`{to}` needs a file extension"));
                    }
                    if start_s.is_some_and(|s| !s.is_finite() || s < 0.0)
                        || duration_s.is_some_and(|s| !s.is_finite() || s <= 0.0)
                    {
                        return Err("invalid time range".into());
                    }
                    Ok(Step {
                        sources: vec![src],
                        target: t,
                        creates: true,
                        op: op.clone(),
                    })
                }
            }
        })();
        let checked = checked.and_then(|st| {
            if st.sources.iter().any(|x| moved.iter().any(|m| same(m, x))) {
                Err(format!(
                    "{}: the file is already renamed by an earlier step",
                    op_code(&op)
                ))
            } else {
                Ok(st)
            }
        });
        match checked {
            Ok(mut step) => {
                if step.creates {
                    step.target = unique_path(&step.target, &taken);
                    taken.push(step.target.clone());
                }
                if matches!(step.op, Op::Rename { .. }) {
                    moved.extend(step.sources.iter().cloned());
                }
                plan.rows.push(row(&step));
                plan.steps.push(step);
            }
            Err(e) => plan.warnings.push(e),
        }
    }
    plan
}

fn op_code(op: &Op) -> &'static str {
    match op {
        Op::Rename { .. } => "rename",
        Op::ConvertImage { .. } => "convert_image",
        Op::StripMetadata { .. } => "strip_metadata",
        Op::Replace { .. } => "replace",
        Op::Reencode { .. } => "reencode",
        Op::CsvClean { .. } => "csv_clean",
        Op::PdfMerge { .. } => "pdf_merge",
        Op::Media { .. } => "media",
    }
}

fn row(s: &Step) -> Row {
    let from = s
        .sources
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let detail = match &s.op {
        Op::ConvertImage { width, height, .. } | Op::Media { width, height, .. } => {
            match (width, height) {
                (None, None) => String::new(),
                (w, h) => format!(
                    "{}x{}",
                    w.map_or("?".into(), |v| v.to_string()),
                    h.map_or("?".into(), |v| v.to_string())
                ),
            }
        }
        Op::Replace {
            find,
            replace,
            regex,
            ..
        } => {
            format!("{}{find} → {replace}", if *regex { "regex " } else { "" })
        }
        Op::Reencode { encoding, .. } => format!("{encoding} → UTF-8"),
        Op::CsvClean {
            trim,
            drop_empty,
            dedupe,
            ..
        } => [
            ("trim", *trim),
            ("drop_empty", *drop_empty),
            ("dedupe", *dedupe),
        ]
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(", "),
        _ => String::new(),
    };
    Row {
        op: op_code(&s.op).into(),
        from,
        to: s.target.display().to_string(),
        detail,
    }
}

/// What [`apply`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub done: Vec<Row>,
    pub failed: Vec<(Row, String)>,
    /// `kor undo --session <id>` reverts the batch.
    pub undo: Option<String>,
}

/// Run a validated plan. Every path is recorded in the undo journal under `undo_base` before it
/// changes. A failing step does not stop the others.
pub fn apply(plan: &Plan, undo_base: Option<&Path>) -> Applied {
    let mut out = Applied::default();
    let roots: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = plan
            .steps
            .iter()
            .flat_map(|s| s.sources.iter().chain([&s.target]))
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let id = format!("batch-{}", kor_agent::session::new_id());
    let mut journal = undo_base.and_then(|b| kor_tools::undo::Journal::open(b, &id, &roots).ok());
    for s in &plan.steps {
        let r = row(s);
        let recorded = journal.as_mut().map_or(Ok(()), |j| {
            s.sources
                .iter()
                .filter(|p| matches!(s.op, Op::Rename { .. }) || **p == s.target)
                .chain([&s.target])
                .try_for_each(|p| j.record(Path::new(""), &p.to_string_lossy()))
        });
        let result = recorded
            .map_err(|e| format!("undo journal: {e}"))
            .and_then(|()| run_step(s));
        match result {
            Ok(()) => out.done.push(r),
            Err(e) => out.failed.push((r, e)),
        }
    }
    if journal.is_some() && !out.done.is_empty() {
        out.undo = Some(id);
    }
    out
}

fn read_text(p: &Path) -> Result<String, String> {
    if std::fs::metadata(p).map_err(|e| e.to_string())?.len() > MAX_TEXT {
        return Err("file is too large".into());
    }
    let b = std::fs::read(p).map_err(|e| e.to_string())?;
    String::from_utf8(b).map_err(|_| "not UTF-8 text (use reencode first)".into())
}

/// Create a new file without ever replacing one.
fn write_new(p: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .map_err(|e| format!("{}: {e}", p.display()))?;
    std::io::Write::write_all(&mut f, bytes).map_err(|e| e.to_string())
}

fn write_out(s: &Step, bytes: &[u8]) -> Result<(), String> {
    if s.creates {
        write_new(&s.target, bytes)
    } else {
        std::fs::write(&s.target, bytes).map_err(|e| e.to_string())
    }
}

#[allow(clippy::items_after_statements, clippy::too_many_lines)] // helper kept next to its only use; one linear sequence; splitting it would scatter the steps
fn run_step(s: &Step) -> Result<(), String> {
    let src = &s.sources[0];
    match &s.op {
        Op::Rename { .. } => {
            if s.target.exists() {
                return Err(format!("{} exists", s.target.display()));
            }
            if let Some(d) = s.target.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::rename(src, &s.target).map_err(|e| e.to_string())
        }
        Op::ConvertImage { width, height, .. } => {
            let img = image::open(src).map_err(|e| e.to_string())?;
            let img = match (width, height) {
                (None, None) => img,
                (w, h) => {
                    let (w0, h0) = (img.width(), img.height());
                    let (w, h) = match (w, h) {
                        (Some(w), Some(h)) => (*w, *h),
                        (Some(w), None) => (*w, scale(h0, *w, w0)),
                        (None, Some(h)) => (scale(w0, *h, h0), *h),
                        (None, None) => (w0, h0),
                    };
                    img.resize(w, h, image::imageops::FilterType::Lanczos3)
                }
            };
            let fmt = image::ImageFormat::from_path(&s.target).map_err(|e| e.to_string())?;
            let img = if matches!(fmt, image::ImageFormat::Jpeg) {
                image::DynamicImage::ImageRgb8(img.to_rgb8())
            } else {
                img
            };
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, fmt).map_err(|e| e.to_string())?;
            write_out(s, buf.get_ref())
        }
        Op::StripMetadata { .. } => {
            let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
            let clean = match ext_lower(src).as_str() {
                "jpg" | "jpeg" => strip_jpeg(&bytes)?,
                "png" => strip_png(&bytes)?,
                _ => {
                    // Other formats: re-encoding drops metadata.
                    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                    let fmt = image::ImageFormat::from_path(src).map_err(|e| e.to_string())?;
                    let mut buf = std::io::Cursor::new(Vec::new());
                    img.write_to(&mut buf, fmt).map_err(|e| e.to_string())?;
                    buf.into_inner()
                }
            };
            write_out(s, &clean)
        }
        Op::Replace {
            find,
            replace,
            regex,
            ..
        } => {
            let text = read_text(src)?;
            let new = if *regex {
                regex::Regex::new(find)
                    .map_err(|e| e.to_string())?
                    .replace_all(&text, replace.as_str())
                    .into_owned()
            } else {
                text.replace(find.as_str(), replace)
            };
            write_out(s, new.as_bytes())
        }
        Op::Reencode { encoding, .. } => {
            let enc =
                encoding_rs::Encoding::for_label(encoding.as_bytes()).ok_or("unknown encoding")?;
            let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
            let (text, _, bad) = enc.decode(&bytes);
            if bad {
                return Err(format!("not valid {}", enc.name()));
            }
            write_out(s, text.as_bytes())
        }
        Op::CsvClean {
            trim,
            drop_empty,
            dedupe,
            delimiter,
            ..
        } => {
            let text = read_text(src)?;
            let delim = delimiter
                .as_deref()
                .and_then(csv_delim)
                .unwrap_or_else(|| sniff_delim(&text));
            write_out(
                s,
                csv_clean(&text, delim, *trim, *drop_empty, *dedupe).as_bytes(),
            )
        }
        Op::PdfMerge { .. } => {
            let bytes = pdf_merge(&s.sources)?;
            write_out(s, &bytes)
        }
        Op::Media {
            width,
            height,
            audio_bitrate_k,
            video_bitrate_k,
            start_s,
            duration_s,
            ..
        } => {
            let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-n".into()];
            if let Some(v) = start_s {
                args.extend(["-ss".into(), format!("{v}")]);
            }
            args.extend(["-i".into(), src.to_string_lossy().into_owned()]);
            if let Some(v) = duration_s {
                args.extend(["-t".into(), format!("{v}")]);
            }
            match (width, height) {
                (None, None) => {}
                (w, h) => args.extend([
                    "-vf".into(),
                    format!(
                        "scale={}:{}",
                        w.map_or(-2, i64::from),
                        h.map_or(-2, i64::from)
                    ),
                ]),
            }
            if let Some(v) = audio_bitrate_k {
                args.extend(["-b:a".into(), format!("{v}k")]);
            }
            if let Some(v) = video_bitrate_k {
                args.extend(["-b:v".into(), format!("{v}k")]);
            }
            args.push(s.target.to_string_lossy().into_owned());
            use kor_platform::NoWindow as _;
            let out = std::process::Command::new(kor_platform::exe("ffmpeg"))
                .args(&args)
                .stdin(std::process::Stdio::null())
                .no_window()
                .output()
                .map_err(|e| format!("ffmpeg: {e} (install ffmpeg and put it on PATH)"))?;
            if out.status.success() {
                Ok(())
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                Err(format!(
                    "ffmpeg: {}",
                    err.lines().last().unwrap_or("failed")
                ))
            }
        }
    }
}

fn scale(other: u32, new: u32, old: u32) -> u32 {
    let v = u64::from(other) * u64::from(new) / u64::from(old.max(1));
    u32::try_from(v.max(1)).unwrap_or(u32::MAX)
}

/// JPEG without APP1 (EXIF/XMP), APP13 (IPTC) and COM segments; image data untouched.
fn strip_jpeg(b: &[u8]) -> Result<Vec<u8>, String> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG".into());
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return Err("damaged JPEG".into());
        }
        let marker = b[i + 1];
        if marker == 0xDA {
            out.extend_from_slice(&b[i..]); // start of scan: the rest is image data
            return Ok(out);
        }
        let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        let end = i + 2 + len;
        if len < 2 || end > b.len() {
            return Err("damaged JPEG".into());
        }
        if !matches!(marker, 0xE1 | 0xED | 0xFE) {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    Err("damaged JPEG".into())
}

/// PNG without `eXIf`, `tEXt`, `zTXt`, `iTXt`, `tIME` chunks.
fn strip_png(b: &[u8]) -> Result<Vec<u8>, String> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !b.starts_with(&SIG) {
        return Err("not a PNG".into());
    }
    let mut out = SIG.to_vec();
    let mut i = 8;
    while i + 12 <= b.len() {
        let len = usize::try_from(u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]))
            .map_err(|e| e.to_string())?;
        let end = i + 12 + len;
        if end > b.len() {
            return Err("damaged PNG".into());
        }
        let kind = &b[i + 4..i + 8];
        if ![b"eXIf", b"tEXt", b"zTXt", b"iTXt", b"tIME"]
            .iter()
            .any(|k| kind == *k)
        {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            return Ok(out);
        }
    }
    Err("damaged PNG".into())
}

fn csv_delim(s: &str) -> Option<char> {
    match s {
        "," => Some(','),
        ";" => Some(';'),
        "\t" | "\\t" | "tab" => Some('\t'),
        "|" => Some('|'),
        _ => None,
    }
}

fn sniff_delim(text: &str) -> char {
    let head = text.lines().next().unwrap_or("");
    [',', ';', '\t', '|']
        .into_iter()
        .max_by_key(|d| head.matches(*d).count())
        .unwrap_or(',')
}

/// RFC 4180 records (quoted fields, doubled quotes, newlines in quotes).
fn csv_parse(text: &str, d: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == d {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_field(f: &str) -> String {
    if f.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", f.replace('"', "\"\""))
    } else {
        f.to_owned()
    }
}

/// Clean CSV text; output uses `,` and `\n`.
pub fn csv_clean(text: &str, delim: char, trim: bool, drop_empty: bool, dedupe: bool) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out = String::new();
    for mut r in csv_parse(text, delim) {
        if trim {
            for f in &mut r {
                *f = f.trim().to_owned();
            }
        }
        if drop_empty && r.iter().all(String::is_empty) {
            continue;
        }
        let line = r.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",");
        if dedupe && !seen.insert(line.clone()) {
            continue;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Merge PDFs in order (pages of each appended to one document).
fn pdf_merge(files: &[PathBuf]) -> Result<Vec<u8>, String> {
    use lopdf::{Document, Object, ObjectId};
    let mut max_id = 1;
    let mut pages: Vec<(ObjectId, Object)> = Vec::new();
    let mut objects = std::collections::BTreeMap::new();
    for f in files {
        let mut doc = Document::load(f).map_err(|e| format!("{}: {e}", f.display()))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        for (_, id) in doc.get_pages() {
            let page = doc.get_object(id).map_err(|e| e.to_string())?.clone();
            pages.push((id, page));
        }
        objects.extend(doc.objects);
    }
    let mut out = Document::with_version("1.5");
    let mut catalog: Option<(ObjectId, Object)> = None;
    let mut root_pages: Option<(ObjectId, Object)> = None;
    for (id, obj) in &objects {
        match obj.type_name().unwrap_or(b"") {
            b"Catalog" => {
                if catalog.is_none() {
                    catalog = Some((*id, obj.clone()));
                }
            }
            b"Pages" => {
                if root_pages.is_none() {
                    root_pages = Some((*id, obj.clone()));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                out.objects.insert(*id, obj.clone());
            }
        }
    }
    let (pages_id, pages_obj) = root_pages.ok_or("no page tree found")?;
    let (catalog_id, catalog_obj) = catalog.ok_or("no catalog found")?;
    for (id, page) in &pages {
        if let Ok(d) = page.as_dict() {
            let mut d = d.clone();
            d.set("Parent", pages_id);
            out.objects.insert(*id, Object::Dictionary(d));
        }
    }
    let mut pd = pages_obj.as_dict().map_err(|e| e.to_string())?.clone();
    pd.set("Count", i64::try_from(pages.len()).unwrap_or(i64::MAX));
    pd.set(
        "Kids",
        pages
            .iter()
            .map(|(id, _)| Object::Reference(*id))
            .collect::<Vec<_>>(),
    );
    out.objects.insert(pages_id, Object::Dictionary(pd));
    let mut cd = catalog_obj.as_dict().map_err(|e| e.to_string())?.clone();
    cd.set("Pages", pages_id);
    cd.remove(b"Outlines");
    out.objects.insert(catalog_id, Object::Dictionary(cd));
    out.trailer.set("Root", catalog_id);
    out.max_id = u32::try_from(out.objects.len()).unwrap_or(u32::MAX);
    out.renumber_objects();
    out.compress();
    let mut buf = Vec::new();
    out.save_to(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Ask `planner` for the operations and validate them.
///
/// # Errors
/// Empty instruction or selection, or an unusable answer.
pub async fn plan(
    planner: &dyn Planner,
    files: &[PathBuf],
    instruction: &str,
) -> Result<Plan, String> {
    let instruction = instruction.trim();
    if instruction.is_empty() || instruction.len() > MAX_INSTRUCTION {
        return Err(format!(
            "instruction must be 1-{MAX_INSTRUCTION} characters"
        ));
    }
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(format!("select 1-{MAX_FILES} files"));
    }
    let answer = planner.ask(prompt(files, instruction)).await?;
    Ok(validate(files, parse_ops(&answer)?))
}

#[cfg(test)]
mod tests;
