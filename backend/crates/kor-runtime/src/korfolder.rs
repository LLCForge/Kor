//! `~/Kor/` (Windows `%USERPROFILE%\Kor`): a readable view of Kor's stores (Master Build Prompt
//! §27.5, ADR 0052).
//!
//! Created only when the user asks (`kor folder init`, the desktop switch); `kor folder remove`
//! undoes exactly what Kor created. Storage stays where it is (`~/.kor` data dir, project
//! `.kor/`); nothing is copied:
//! - `Kayıtlar/<proje>` → link to `<proje>/.kor/sessions` (§15.9 recordings),
//!   `Görseller/<proje>` → link to `<proje>/kor-media` (+ generated `Görseller/Galeri.md` with
//!   size/date metadata), `Raporlar/<proje>/<tür>` → links to `.kor/perf`, `.kor/bench`, … ;
//! - generated Markdown views: `Oturumlar/<id>.md` (transcript + attachments, front matter
//!   `title`/`pinned`/`archived`), `Diffler/<session>.diff` (undo journals vs. current files);
//! - drop boxes: `Gelen/` (files join the next new session as context, then move to
//!   `Gelen/İşlendi/`) and `Görevler/` (a `.md` starts a task; the answer is written to
//!   `<ad>.sonuç.md`, the file moves to `Görevler/Tamamlandı/`).
//!
//! Two-way: editing a view's front matter changes the store (session title, pin, archive); when
//! both changed, the file wins. Every file Kor generated is recorded with its hash in
//! `<kor home>/korfolder.json`, so Kor never overwrites or deletes a file the user changed or
//! added. `notify` keeps it live in `kor daemon` and in the desktop app ([`watch`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub mod dav;
mod tasks;
mod views;
pub mod watch;

pub use tasks::{FileTask, TaskOutcome, TaskRunner, run_pending, tasks_pending};

pub const SESSIONS: &str = "Oturumlar";
pub const MEDIA: &str = "Görseller";
pub const REPORTS: &str = "Raporlar";
pub const DIFFS: &str = "Diffler";
pub const RECORDS: &str = "Kayıtlar";
pub const INBOX: &str = "Gelen";
pub const INBOX_DONE: &str = "İşlendi";
pub const TASKS: &str = "Görevler";
pub const TASKS_DONE: &str = "Tamamlandı";

/// Every top-level folder, in display order.
pub const FOLDERS: [&str; 7] = [SESSIONS, MEDIA, REPORTS, DIFFS, RECORDS, INBOX, TASKS];

/// Project folders that hold reports (linked under `Raporlar/<proje>/`).
const REPORT_DIRS: [&str; 5] = [
    ".kor/perf",
    ".kor/bench",
    ".kor/reports",
    ".kor/e2e",
    ".kor/deploy",
];

/// Largest inbox file inlined as text into a session.
const INBOX_TEXT_MAX: u64 = 32 * 1024;
const INBOX_TOTAL_MAX: usize = 128 * 1024;

/// Where the folder lives and where Kor keeps its data (`kor_home()`); both injectable for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `~/Kor`.
    pub root: PathBuf,
    /// Kor's data directory (`sessions/`, `undo/`, `desktop.json`, `korfolder.json`).
    pub home: PathBuf,
}

impl Layout {
    /// The user's `~/Kor` and Kor's data directory.
    pub fn current() -> Option<Self> {
        Some(Self {
            root: dirs::home_dir()?.join("Kor"),
            home: crate::kor_home()?,
        })
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.home.join("korfolder.json")
    }
    pub fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    /// Enabled = Kor created it and has not removed it.
    pub fn enabled(&self) -> bool {
        self.manifest_path().is_file()
    }
}

/// What Kor created, so removal and regeneration only touch Kor's own files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub root: PathBuf,
    /// Folders Kor created (relative to the root), parents first.
    pub dirs: Vec<String>,
    /// Links Kor created: relative path → target.
    pub links: BTreeMap<String, PathBuf>,
    /// Generated files: relative path → SHA-256 of what Kor last wrote (or accepted).
    pub generated: BTreeMap<String, String>,
    /// Projects whose recordings, media and reports are linked.
    pub projects: Vec<PathBuf>,
}

impl Manifest {
    pub fn load(l: &Layout) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(l.manifest_path()).ok()?).ok()
    }
    fn save(&self, l: &Layout) -> std::io::Result<()> {
        std::fs::create_dir_all(&l.home)?;
        let tmp = l.manifest_path().with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(tmp, l.manifest_path())
    }
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn rel(l: &Layout, p: &Path) -> String {
    p.strip_prefix(&l.root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One line of `kor folder status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub name: String,
    /// `link`, `view` or `dir`.
    pub kind: &'static str,
    pub path: PathBuf,
    pub target: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub path: PathBuf,
    pub items: Vec<Item>,
    pub projects: Vec<PathBuf>,
    /// Files waiting in `Gelen/` and `Görevler/`.
    pub inbox: usize,
    pub tasks: usize,
}

/// What a [`sync`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub written: Vec<PathBuf>,
    pub linked: Vec<PathBuf>,
    /// Views the user edited: their front matter was applied to the store, the file was kept.
    pub applied: Vec<PathBuf>,
    /// Views the user changed whose store entry is gone: kept untouched.
    pub kept: Vec<PathBuf>,
    pub notes: Vec<String>,
}

pub fn status(l: &Layout) -> Status {
    let m = Manifest::load(l);
    let mut items = Vec::new();
    if let Some(m) = &m {
        for name in FOLDERS {
            items.push(Item {
                name: name.into(),
                kind: if matches!(name, INBOX | TASKS) {
                    "dir"
                } else {
                    "view"
                },
                path: l.dir(name),
                target: None,
            });
        }
        for (r, t) in &m.links {
            items.push(Item {
                name: r.clone(),
                kind: "link",
                path: l.root.join(r),
                target: Some(t.clone()),
            });
        }
    }
    Status {
        enabled: m.is_some(),
        path: l.root.clone(),
        items,
        projects: m.map(|m| m.projects).unwrap_or_default(),
        inbox: inbox_files(l).len(),
        tasks: tasks_pending(l).len(),
    }
}

/// Create `~/Kor/` (consent given by the caller) and fill it. Idempotent; `project` is added to
/// the linked projects.
///
/// # Errors
/// If the folder cannot be created.
pub fn enable(l: &Layout, project: Option<&Path>) -> std::io::Result<SyncReport> {
    let mut m = Manifest::load(l).unwrap_or_else(|| Manifest {
        root: l.root.clone(),
        ..Manifest::default()
    });
    for name in FOLDERS.iter().map(|s| (*s).to_owned()).chain([
        format!("{INBOX}/{INBOX_DONE}"),
        format!("{TASKS}/{TASKS_DONE}"),
    ]) {
        let p = l.root.join(&name);
        if !p.exists() {
            if !l.root.exists() && !m.dirs.iter().any(String::is_empty) {
                m.dirs.push(String::new());
            }
            std::fs::create_dir_all(&p)?;
            m.dirs.push(name);
        }
    }
    if let Some(p) = project {
        add_project(&mut m, p);
    }
    m.save(l)?;
    sync(l)
}

fn add_project(m: &mut Manifest, p: &Path) {
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let p = PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\"));
    if !m.projects.contains(&p) {
        m.projects.push(p);
    }
}

/// Link `project` too (`kor folder sync` from inside a project, the desktop's open folder).
///
/// # Errors
/// When the manifest cannot be written.
pub fn add(l: &Layout, project: &Path) -> std::io::Result<bool> {
    let Some(mut m) = Manifest::load(l) else {
        return Ok(false);
    };
    let before = m.projects.len();
    add_project(&mut m, project);
    m.save(l)?;
    Ok(m.projects.len() != before)
}

/// What [`disable`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RemoveReport {
    pub removed: Vec<PathBuf>,
    /// Files the user added or changed: left in place (the folder stays).
    pub kept: Vec<PathBuf>,
}

/// Remove what Kor created: links (targets untouched), generated files the user did not change,
/// then the folders it created if they are empty. Anything else stays.
///
/// # Errors
/// If the manifest cannot be removed.
pub fn disable(l: &Layout) -> std::io::Result<RemoveReport> {
    let mut r = RemoveReport::default();
    let Some(m) = Manifest::load(l) else {
        return Ok(r);
    };
    for link in m.links.keys() {
        let p = l.root.join(link);
        if kor_platform::mount::remove_link(&p).is_ok() {
            r.removed.push(p);
        }
    }
    for (file, h) in &m.generated {
        let p = l.root.join(file);
        match std::fs::read(&p) {
            Ok(b) if hash(&b) == *h => {
                if std::fs::remove_file(&p).is_ok() {
                    r.removed.push(p);
                }
            }
            Ok(_) => r.kept.push(p),
            Err(_) => {}
        }
    }
    // Deepest first; only empty folders go (a user file keeps its folder).
    let mut dirs: Vec<PathBuf> = m.dirs.iter().map(|d| l.root.join(d)).collect();
    for p in walk_dirs(&l.root) {
        if !dirs.contains(&p) && is_under_created(&m, l, &p) {
            dirs.push(p);
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        if std::fs::remove_dir(&d).is_ok() {
            r.removed.push(d);
        } else if d.exists() {
            for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = f.path();
                if p.is_file() && !r.kept.contains(&p) {
                    r.kept.push(p);
                }
            }
        }
    }
    std::fs::remove_file(l.manifest_path())?;
    Ok(r)
}

/// Sub-folders Kor made while generating (e.g. `Raporlar/<proje>`).
fn is_under_created(m: &Manifest, l: &Layout, p: &Path) -> bool {
    let r = rel(l, p);
    m.links.keys().any(|k| k.starts_with(&format!("{r}/")))
}

fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir())
                && kor_platform::mount::link_target(&p).is_none()
            {
                out.push(p.clone());
                stack.push(p);
            }
        }
    }
    out
}

/// Bring links and generated views up to date (both ways). No-op when disabled.
///
/// # Errors
/// If the manifest cannot be written.
pub fn sync(l: &Layout) -> std::io::Result<SyncReport> {
    let mut r = SyncReport::default();
    let Some(mut m) = Manifest::load(l) else {
        return Ok(r);
    };
    for p in views::session_roots(l) {
        add_project(&mut m, &p);
    }
    link_projects(l, &mut m, &mut r);
    views::sessions(l, &mut m, &mut r);
    views::diffs(l, &mut m, &mut r);
    views::gallery(l, &mut m, &mut r);
    m.save(l)?;
    Ok(r)
}

fn project_name(m: &Manifest, p: &Path) -> String {
    let base = p
        .file_name()
        .map_or_else(|| "proje".into(), |n| n.to_string_lossy().into_owned());
    // Same folder name in two places: suffix with the parent's name.
    let clash = m
        .projects
        .iter()
        .filter(|o| o.file_name() == p.file_name())
        .count()
        > 1;
    match (clash, p.parent().and_then(Path::file_name)) {
        (true, Some(parent)) => format!("{base} ({})", parent.to_string_lossy()),
        _ => base,
    }
}

fn link_projects(l: &Layout, m: &mut Manifest, r: &mut SyncReport) {
    let projects = m.projects.clone();
    for p in &projects {
        let name = project_name(m, p);
        let mut want = vec![
            (format!("{RECORDS}/{name}"), p.join(kor_tools::record::DIR)),
            (
                format!("{MEDIA}/{name}"),
                p.join(kor_agent::media::MEDIA_DIR),
            ),
        ];
        for d in REPORT_DIRS {
            let kind = d.rsplit('/').next().unwrap_or(d);
            want.push((format!("{REPORTS}/{name}/{kind}"), p.join(d)));
        }
        for (link, target) in want {
            // Joined `a/b` keeps the `/` on Windows, which `mklink /J` reads as a switch.
            let target: PathBuf = target.components().collect();
            if !target.is_dir() || m.links.contains_key(&link) {
                continue;
            }
            let at: PathBuf = l.root.join(&link).components().collect();
            match kor_platform::mount::link_dir(&target, &at) {
                Ok(()) => {
                    m.links.insert(link, target);
                    r.linked.push(at);
                }
                Err(e) => r.notes.push(format!("{}: {e}", at.display())),
            }
        }
    }
    // Links whose target disappeared (project deleted) are dropped.
    let gone: Vec<String> = m
        .links
        .iter()
        .filter(|(_, t)| !t.is_dir())
        .map(|(k, _)| k.clone())
        .collect();
    for k in gone {
        let _ = kor_platform::mount::remove_link(&l.root.join(&k));
        m.links.remove(&k);
    }
}

/// Write a generated file unless the user changed it since Kor last wrote it. Returns `true`
/// when written.
pub(crate) fn write_generated(
    l: &Layout,
    m: &mut Manifest,
    r: &mut SyncReport,
    path: &Path,
    body: &str,
) -> bool {
    let key = rel(l, path);
    let new_hash = hash(body.as_bytes());
    match std::fs::read(path) {
        Ok(cur) => {
            let cur_hash = hash(&cur);
            if cur_hash == new_hash {
                m.generated.insert(key, new_hash);
                return false;
            }
            // Not Kor's, or changed by the user: the file wins.
            if m.generated.get(&key) != Some(&cur_hash) {
                r.kept.push(path.to_path_buf());
                return false;
            }
        }
        Err(_) => {}
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    match std::fs::write(path, body) {
        Ok(()) => {
            m.generated.insert(key, new_hash);
            r.written.push(path.to_path_buf());
            true
        }
        Err(e) => {
            r.notes.push(format!("{}: {e}", path.display()));
            false
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Gelen/ → context of the next new session
// ---------------------------------------------------------------------------------------------

/// Files waiting in `Gelen/` (not `İşlendi/`, no hidden or partial downloads).
pub fn inbox_files(l: &Layout) -> Vec<PathBuf> {
    if !l.enabled() {
        return Vec::new();
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(l.dir(INBOX))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_ready(p))
        .collect();
    v.sort();
    v
}

/// Skip hidden files and in-progress downloads.
fn is_ready(p: &Path) -> bool {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    !(name.starts_with('.')
        || name.starts_with("~$")
        || [".part", ".crdownload", ".tmp", ".download"]
            .iter()
            .any(|s| name.ends_with(s)))
}

/// Move a file into `dir` without overwriting: `ad.txt`, `ad (2).txt`, …
pub(crate) fn move_unique(from: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = from.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let to = crate::batch::unique_path(&dir.join(name), &[]);
    std::fs::rename(from, &to)?;
    Ok(to)
}

/// Take the waiting inbox files: they move to `Gelen/İşlendi/` and are returned there.
pub fn take_inbox(l: &Layout) -> Vec<PathBuf> {
    inbox_files(l)
        .into_iter()
        .filter_map(|p| move_unique(&p, &l.dir(INBOX).join(INBOX_DONE)).ok())
        .collect()
}

/// System prompt section for files taken from the inbox: text files inline (size-capped),
/// others by path. Paths outside the project are given here because the tools cannot read them.
pub fn inbox_block(files: &[PathBuf]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n# Files the user dropped into ~/Kor/Gelen\nTreat them as context for this conversation.\n",
    );
    for f in files {
        let size = std::fs::metadata(f).map_or(0, |m| m.len());
        let text = (size <= INBOX_TEXT_MAX)
            .then(|| std::fs::read(f).ok())
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .filter(|t| !t.contains('\0'));
        match text {
            Some(t) if out.len() + t.len() < INBOX_TOTAL_MAX => {
                out.push_str(&format!(
                    "\n<file path=\"{}\">\n{t}\n</file>\n",
                    f.display()
                ));
            }
            _ => out.push_str(&format!("\n- {} ({size} bytes)\n", f.display())),
        }
    }
    out
}

/// The inbox section for a new interactive session (empty when the folder is off or empty).
pub fn take_inbox_context() -> String {
    Layout::current().map_or_else(String::new, |l| inbox_block(&take_inbox(&l)))
}

#[cfg(test)]
mod tests;
