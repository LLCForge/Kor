//! Personal knowledge base extras (Master Build Prompt §27.15, ADR 0063): which notes were used,
//! the weekly digest, and staleness.
//!
//! - Uses: every time a record is shown to the agent (automatic recall) or to the user (`kor know`,
//!   `kor recall`), one line `{id, ts}` is appended to `<home>/knowledge-uses.jsonl`. Local only.
//! - Weekly digest: `<home>/knowledge/digest/<YYYY>-W<ww>.md` for the last complete ISO week: new
//!   notes, the five most referenced notes, and the notes that went stale. Written once per week by
//!   the scheduler (`kor daemon` or the desktop app) or on demand (`kor know digest`).
//! - Staleness: a note is stale when a file it names is gone or changed after the note was written,
//!   or when a symbol it names in backticks no longer appears in the files it names.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Datelike, TimeZone, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::kb::{self, Knowledge, Record};

/// Use log lines read at most (about a year of heavy use).
const MAX_USES: usize = 200_000;
/// Records looked at for a weekly digest.
const MAX_WEEK_RECORDS: usize = 5_000;
const TOP: usize = 5;
/// Bytes of a file read to look for a symbol.
const MAX_SCAN: u64 = 2 * 1024 * 1024;

/// `<home>/knowledge-uses.jsonl`.
pub fn uses_path(home: &Path) -> PathBuf {
    home.join("knowledge-uses.jsonl")
}

/// `<home>/knowledge/digest/`.
pub fn digest_dir(home: &Path) -> PathBuf {
    home.join("knowledge").join("digest")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Use {
    id: i64,
    ts: i64,
}

/// Note that `ids` were used now. Best effort for callers: an error only means the count is off.
///
/// # Errors
/// When the log cannot be written.
pub fn record_uses(home: &Path, ids: &[i64]) -> std::io::Result<()> {
    record_uses_at(home, ids, Utc::now().timestamp())
}

fn record_uses_at(home: &Path, ids: &[i64], ts: i64) -> std::io::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(home)?;
    let mut out = String::new();
    for id in ids.iter().filter(|i| **i > 0) {
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string(&Use { id: *id, ts }).unwrap_or_default()
        );
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(uses_path(home))?;
    f.write_all(out.as_bytes())
}

fn uses(home: &Path) -> Vec<Use> {
    let text = std::fs::read_to_string(uses_path(home)).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(MAX_USES);
    lines[skip..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Uses per record id within `[from, to)` (Unix seconds).
pub fn use_counts(home: &Path, from: i64, to: i64) -> BTreeMap<i64, usize> {
    let mut m = BTreeMap::new();
    for u in uses(home).into_iter().filter(|u| u.ts >= from && u.ts < to) {
        *m.entry(u.id).or_default() += 1;
    }
    m
}

// ---------------------------------------------------------------------------------------------
// Staleness
// ---------------------------------------------------------------------------------------------

/// Why a note may be out of date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// The project folder itself is gone.
    ProjectMissing { project: String },
    /// A file the note names no longer exists.
    FileMissing { file: String },
    /// A file the note names changed after the note was written.
    FileChanged { file: String, modified: i64 },
    /// A symbol the note names (in backticks) is in none of its files any more.
    SymbolGone { symbol: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stale {
    pub id: i64,
    pub task: String,
    pub reasons: Vec<Reason>,
}

static SYMBOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"`([A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.)[A-Za-z_][A-Za-z0-9_]*)*)(?:\(\))?`")
        .expect("valid regex")
});

/// Backticked identifiers in a note's text (`login`, `Db::connect`, `auth.verify()`).
fn symbols(r: &Record) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in std::iter::once(&r.summary)
        .chain(&r.errors)
        .chain(&r.decisions)
    {
        for c in SYMBOL.captures_iter(t) {
            let s = &c[1];
            // The last segment is what appears in the source (`connect` of `Db::connect`).
            let last = s.rsplit([':', '.']).next().unwrap_or(s);
            if last.len() >= 3 {
                out.insert(last.to_owned());
            }
        }
    }
    out
}

fn mtime(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}

/// Staleness of one record; `None` when it looks current (or names nothing that can be checked).
pub fn staleness(r: &Record) -> Option<Stale> {
    if r.files.is_empty() {
        return None;
    }
    let project = Path::new(&r.project);
    let mut reasons = Vec::new();
    if !r.project.is_empty() && project.is_absolute() && !project.is_dir() {
        reasons.push(Reason::ProjectMissing {
            project: r.project.clone(),
        });
        return Some(Stale {
            id: r.id,
            task: r.task.clone(),
            reasons,
        });
    }
    let mut existing = Vec::new();
    for f in &r.files {
        // Files are stored as written by the agent: project-relative, or absolute.
        let p = if Path::new(f).is_absolute() {
            PathBuf::from(f)
        } else {
            project.join(f)
        };
        match mtime(&p) {
            None if !p.exists() => reasons.push(Reason::FileMissing { file: f.clone() }),
            None => {}
            Some(m) => {
                // A second of slack: the note is written right after the edit.
                if m > r.ts + 1 {
                    reasons.push(Reason::FileChanged {
                        file: f.clone(),
                        modified: m,
                    });
                }
                existing.push(p);
            }
        }
    }
    if !existing.is_empty() {
        let texts: Vec<String> = existing
            .iter()
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= MAX_SCAN))
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();
        if !texts.is_empty() {
            for s in symbols(r) {
                if !texts.iter().any(|t| t.contains(&s)) {
                    reasons.push(Reason::SymbolGone { symbol: s });
                }
            }
        }
    }
    (!reasons.is_empty()).then(|| Stale {
        id: r.id,
        task: r.task.clone(),
        reasons,
    })
}

/// Stale notes among the newest `limit` (optionally of one project).
///
/// # Errors
/// On a database error.
pub fn stale(k: &Knowledge, project: Option<&str>, limit: usize) -> kb::Result<Vec<Stale>> {
    Ok(k.recent(project, limit)?
        .iter()
        .filter_map(staleness)
        .collect())
}

/// One line for a stale note (English; front ends localize the kinds).
pub fn reason_text(r: &Reason) -> String {
    match r {
        Reason::ProjectMissing { project } => format!("project {project} is gone"),
        Reason::FileMissing { file } => format!("{file} is gone"),
        Reason::FileChanged { file, modified } => {
            format!("{file} changed on {}", kb::date(*modified))
        }
        Reason::SymbolGone { symbol } => format!("`{symbol}` is no longer in its files"),
    }
}

// ---------------------------------------------------------------------------------------------
// Weekly digest
// ---------------------------------------------------------------------------------------------

/// A digest of one ISO week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Digest {
    /// `2026-W45`.
    pub week: String,
    /// Monday 00:00 UTC (Unix seconds).
    pub from: i64,
    pub to: i64,
    pub new: Vec<Record>,
    /// The most referenced notes of the week, with their use counts.
    pub top: Vec<(Record, usize)>,
    pub stale: Vec<Stale>,
}

/// Monday 00:00 UTC of the ISO week containing `ts`.
pub fn week_start(ts: i64) -> i64 {
    let d = Utc
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_default()
        .date_naive();
    let monday = d - chrono::Duration::days(i64::from(d.weekday().num_days_from_monday()));
    monday
        .and_hms_opt(0, 0, 0)
        .map_or(ts, |t| t.and_utc().timestamp())
}

fn week_label(from: i64) -> String {
    let d = Utc
        .timestamp_opt(from, 0)
        .single()
        .unwrap_or_default()
        .iso_week();
    format!("{}-W{:02}", d.year(), d.week())
}

/// The digest of the ISO week that starts at `from`.
///
/// # Errors
/// On a database error.
pub fn digest(k: &Knowledge, home: &Path, from: i64) -> kb::Result<Digest> {
    let to = from + 7 * 86_400;
    let recent = k.recent(None, MAX_WEEK_RECORDS)?;
    let new: Vec<Record> = recent
        .iter()
        .filter(|r| r.ts >= from && r.ts < to)
        .cloned()
        .collect();
    let counts = use_counts(home, from, to);
    let mut ranked: Vec<(i64, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    let mut top = Vec::new();
    for (id, n) in ranked {
        if let Some(r) = k.get(id)? {
            top.push((r, n));
        }
        if top.len() == TOP {
            break;
        }
    }
    let mut seen = BTreeSet::new();
    let stale = new
        .iter()
        .chain(top.iter().map(|(r, _)| r))
        .filter(|r| seen.insert(r.id))
        .filter_map(staleness)
        .collect();
    Ok(Digest {
        week: week_label(from),
        from,
        to,
        new,
        top,
        stale,
    })
}

/// The digest as Markdown (plain, Obsidian friendly).
pub fn markdown(d: &Digest) -> String {
    let mut o = format!(
        "---\nweek: {}\nfrom: {}\nto: {}\ntags: [kor, digest]\n---\n\n# Kor knowledge digest {}\n\n",
        d.week,
        kb::date(d.from),
        kb::date(d.to - 1),
        d.week
    );
    let _ = writeln!(o, "## Most referenced notes\n");
    if d.top.is_empty() {
        o.push_str("_No note was used this week._\n");
    }
    for (r, n) in &d.top {
        let _ = writeln!(
            o,
            "- **#{}** {} ({n}×) — {}",
            r.id,
            r.task.replace('\n', " "),
            first_line(&r.summary)
        );
    }
    let _ = writeln!(o, "\n## New notes ({})\n", d.new.len());
    for r in &d.new {
        let _ = writeln!(
            o,
            "- **#{}** {} · {} — {}",
            r.id,
            kb::date(r.ts),
            r.task.replace('\n', " "),
            first_line(&r.summary)
        );
    }
    if !d.stale.is_empty() {
        let _ = writeln!(o, "\n## Possibly outdated ({})\n", d.stale.len());
        for s in &d.stale {
            let why: Vec<String> = s.reasons.iter().map(reason_text).collect();
            let _ = writeln!(
                o,
                "- **#{}** {}: {}",
                s.id,
                s.task.replace('\n', " "),
                why.join("; ")
            );
        }
    }
    o
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

/// The file of the last complete week's digest when it is not written yet (cheap: no database).
pub fn due(home: &Path, now: i64) -> Option<PathBuf> {
    let from = week_start(now) - 7 * 86_400;
    let path = digest_dir(home).join(format!("{}.md", week_label(from)));
    (!path.exists()).then_some(path)
}

/// Write the digest of the last complete week if it is not written yet. Returns its path when
/// one was written now.
///
/// # Errors
/// On a database or file error.
pub fn write_due(k: &Knowledge, home: &Path, now: i64) -> kb::Result<Option<PathBuf>> {
    let from = week_start(now) - 7 * 86_400;
    let path = digest_dir(home).join(format!("{}.md", week_label(from)));
    if path.exists() {
        return Ok(None);
    }
    write_week(k, home, from).map(Some)
}

/// Write (or replace) the digest of the week starting at `from`.
///
/// # Errors
/// On a database or file error.
pub fn write_week(k: &Knowledge, home: &Path, from: i64) -> kb::Result<PathBuf> {
    let d = digest(k, home, from)?;
    let dir = digest_dir(home);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.md", d.week));
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, markdown(&d))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(project: &str, task: &str, files: &[&str], summary: &str, ts: i64) -> Record {
        Record {
            ts,
            project: project.into(),
            kind: "task".into(),
            task: task.into(),
            summary: summary.into(),
            files: files.iter().map(|s| (*s).to_owned()).collect(),
            ..Record::default()
        }
    }

    #[test]
    fn staleness_reasons() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/db.rs"), "pub fn connect() {}\n").unwrap();
        let project = root.to_string_lossy().replace('\\', "/");
        let future = Utc::now().timestamp() + 3600;
        // Written after the file: current.
        let ok = rec(
            &project,
            "fix db",
            &["src/db.rs"],
            "use `Db::connect`",
            future,
        );
        assert!(staleness(&ok).is_none());
        // Names a symbol that is gone and a missing file.
        let r = rec(
            &project,
            "fix",
            &["src/db.rs", "src/gone.rs"],
            "call `reconnect()`",
            future,
        );
        let s = staleness(&r).unwrap();
        assert!(s.reasons.contains(&Reason::FileMissing {
            file: "src/gone.rs".into()
        }));
        assert!(s.reasons.contains(&Reason::SymbolGone {
            symbol: "reconnect".into()
        }));
        // Written before the file changed.
        let old = rec(&project, "old", &["src/db.rs"], "", 1_000);
        assert!(matches!(
            staleness(&old).unwrap().reasons[0],
            Reason::FileChanged { .. }
        ));
        let gone = rec(
            &d.path().join("nope").to_string_lossy(),
            "x",
            &["a.rs"],
            "",
            1,
        );
        assert!(matches!(
            staleness(&gone).unwrap().reasons[0],
            Reason::ProjectMissing { .. }
        ));
        assert!(staleness(&rec(&project, "no files", &[], "", 1)).is_none());
    }

    #[test]
    fn weekly_digest_ranks_uses_and_is_written_once() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let k = Knowledge::open(&crate::knowledge_path(home)).unwrap();
        let from = week_start(1_762_300_000); // a Monday-aligned week in Nov 2025
        k.record_all(vec![
            rec(
                "p",
                "docker build cache",
                &[],
                "use --mount=type=cache",
                from + 100,
            ),
            rec("p", "flaky test", &[], "retry once", from + 200),
            rec("p", "old note", &[], "before the week", from - 100),
        ])
        .unwrap();
        k.flush().unwrap();
        let all = k.recent(None, 10).unwrap();
        let id = |t: &str| all.iter().find(|r| r.task == t).unwrap().id;
        record_uses_at(
            home,
            &[id("flaky test"), id("old note"), id("old note")],
            from + 300,
        )
        .unwrap();
        record_uses_at(home, &[id("old note")], from - 50).unwrap(); // outside the week
        let dg = digest(&k, home, from).unwrap();
        assert_eq!(dg.new.len(), 2);
        assert_eq!(dg.top[0].0.task, "old note");
        assert_eq!(dg.top[0].1, 2);
        assert_eq!(dg.top[1].0.task, "flaky test");
        let md = markdown(&dg);
        assert!(md.contains("# Kor knowledge digest") && md.contains("old note (2×)"));
        assert!(dg.week.contains("-W"));
        // write_due writes the previous week once.
        let now = from + 8 * 86_400;
        let p = write_due(&k, home, now).unwrap().unwrap();
        assert!(p.ends_with(format!("{}.md", dg.week)));
        assert!(write_due(&k, home, now).unwrap().is_none());
        assert_eq!(week_start(from + 3 * 86_400 + 5), from);
    }
}
