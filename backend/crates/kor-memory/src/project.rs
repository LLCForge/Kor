//! Per-project memory (§6.1): plain Markdown the user may edit. Kor appends entries
//! (`decisions.md`, `sessions/`) or replaces only the sections it marked itself; a marked section
//! the user changed is left alone.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const LINKS: &str = "links.json";
const MAX_LINKS: usize = 64;

/// The four Markdown documents of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    Architecture,
    Decisions,
    Patterns,
    Hotspots,
}

impl Doc {
    pub const ALL: [Self; 4] = [
        Self::Architecture,
        Self::Decisions,
        Self::Patterns,
        Self::Hotspots,
    ];

    /// The document whose file name is `name` (`architecture.md`, ...).
    pub fn from_file(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.file() == name)
    }

    pub fn file(self) -> &'static str {
        match self {
            Self::Architecture => "architecture.md",
            Self::Decisions => "decisions.md",
            Self::Patterns => "patterns.md",
            Self::Hotspots => "hotspots.md",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Architecture => {
                "# Architecture\n\nKor's map of this project. Edit freely; Kor only rewrites its own marked sections.\n"
            }
            Self::Decisions => {
                "# Decisions\n\nOne `## YYYY-MM-DD — Title` entry per decision, then the reason. A `Refs:` line names files or symbols (`src/auth.rs`, `Auth::login`).\n"
            }
            Self::Patterns => "# Patterns\n\nPatterns and conventions used in this project.\n",
            Self::Hotspots => "# Hotspots\n\nMost changed or riskiest files.\n",
        }
    }
}

/// One ADR-style entry of `decisions.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub title: String,
    pub body: String,
    /// `YYYY-MM-DD`, empty when the entry has none.
    pub date: String,
    /// Files or symbols the decision is about (graph `DecidedBy` edges).
    pub refs: Vec<String>,
}

/// One row of `hotspots.md`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hotspot {
    pub path: String,
    pub score: f64,
    pub reason: String,
}

/// One entry of `sessions/<date>.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// `YYYY-MM-DD HH:MM`.
    pub date: String,
    pub task: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMemory {
    pub dir: PathBuf,
}

impl ProjectMemory {
    pub fn new(home: &Path, root: &Path) -> Self {
        Self {
            dir: crate::project_dir(home, root),
        }
    }

    pub fn path(&self, doc: Doc) -> PathBuf {
        self.dir.join(doc.file())
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.dir.join("sessions")
    }

    /// Create the folder and any missing document with its heading. Existing files are untouched.
    ///
    /// # Errors
    /// When the folder or a file cannot be written.
    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(self.sessions_dir())?;
        for doc in Doc::ALL {
            let p = self.path(doc);
            if !p.exists() {
                fs::write(p, doc.heading())?;
            }
        }
        Ok(())
    }

    fn read(&self, doc: Doc) -> String {
        fs::read_to_string(self.path(doc)).unwrap_or_else(|_| doc.heading().to_owned())
    }

    /// The text of `doc` (its heading when the file does not exist yet).
    pub fn text(&self, doc: Doc) -> String {
        self.read(doc)
    }

    /// Replace the whole document with the user's text, secrets masked. Kor's marked sections
    /// in it keep working: one the user changed is no longer rewritten ([`sections`]).
    /// Returns the text written.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write_doc(&self, doc: Doc, text: &str) -> io::Result<String> {
        let text = crate::mask(text);
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(doc), &text)?;
        Ok(text)
    }

    /// Days that have session summaries (`YYYY-MM-DD` or `undated`), newest first.
    pub fn session_days(&self) -> Vec<String> {
        let mut days: Vec<String> = fs::read_dir(self.sessions_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".md")
                    .filter(|d| valid_day(d))
                    .map(str::to_owned)
            })
            .collect();
        days.sort_unstable_by(|a, b| b.cmp(a));
        days
    }

    /// One day's session summaries; `None` for an invalid or missing day.
    pub fn session(&self, day: &str) -> Option<String> {
        if !valid_day(day) {
            return None;
        }
        fs::read_to_string(self.sessions_dir().join(format!("{day}.md"))).ok()
    }

    /// Replace Kor's section `name` of `doc` with `body`, or append it. `Ok(false)` when the user
    /// edited that section since Kor wrote it: it is kept as it is.
    ///
    /// # Errors
    /// When the name is not a plain word or the file cannot be written.
    pub fn set_section(&self, doc: Doc, name: &str, body: &str) -> io::Result<bool> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "section name must be letters, digits, - or _",
            ));
        }
        let text = self.read(doc);
        let body = body.trim();
        let begin = format!("<!-- kor:begin {name} ");
        let end = format!("<!-- kor:end {name} -->");
        let block = format!("{begin}hash={} -->\n{body}\n{end}", crate::fnv(body));
        let updated = match (text.find(&begin), text.find(&end)) {
            (Some(b), Some(e)) if b < e => {
                let head_end = text[b..].find("-->").map_or(b, |i| b + i);
                let recorded = text[b..head_end]
                    .split_whitespace()
                    .find_map(|w| w.strip_prefix("hash="))
                    .unwrap_or_default();
                let inner_start = (head_end + 3).min(e);
                if crate::fnv(text[inner_start..e].trim()) != recorded {
                    return Ok(false);
                }
                format!("{}{block}{}", &text[..b], &text[e + end.len()..])
            }
            _ => {
                let mut t = text;
                if !t.ends_with('\n') {
                    t.push('\n');
                }
                format!("{t}\n{block}\n")
            }
        };
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(doc), &updated)?;
        Ok(true)
    }

    /// Append one decision to `decisions.md`.
    ///
    /// # Errors
    /// When the title is empty or the file cannot be written.
    pub fn add_decision(&self, d: &Decision) -> io::Result<()> {
        let title = one_line(&d.title);
        if title.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "decision title is empty",
            ));
        }
        let mut entry = String::from("\n## ");
        if !d.date.trim().is_empty() {
            let _ = write!(entry, "{} — ", one_line(&d.date));
        }
        let _ = writeln!(entry, "{title}\n");
        let body = crate::mask(d.body.trim());
        if !body.is_empty() {
            let _ = writeln!(entry, "{body}\n");
        }
        if !d.refs.is_empty() {
            let refs: Vec<String> = d
                .refs
                .iter()
                .map(|r| format!("`{}`", one_line(r)))
                .collect();
            let _ = writeln!(entry, "Refs: {}", refs.join(", "));
        }
        self.append(Doc::Decisions, &entry)
    }

    fn append(&self, doc: Doc, entry: &str) -> io::Result<()> {
        let mut text = self.read(doc);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(entry);
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(doc), &text)
    }

    /// Every `## ` entry of `decisions.md`, in file order (hand-written ones included).
    pub fn decisions(&self) -> Vec<Decision> {
        fs::read_to_string(self.path(Doc::Decisions))
            .map(|t| parse_decisions(&t))
            .unwrap_or_default()
    }

    /// Rewrite the hotspot table (Kor's `hotspots` section of `hotspots.md`).
    ///
    /// # Errors
    /// See [`Self::set_section`].
    pub fn write_hotspots(&self, rows: &[Hotspot]) -> io::Result<bool> {
        let mut t = String::from("| File | Score | Why |\n| --- | ---: | --- |\n");
        for r in rows {
            let _ = writeln!(
                t,
                "| `{}` | {:.2} | {} |",
                cell(&r.path),
                r.score,
                cell(&r.reason)
            );
        }
        self.set_section(Doc::Hotspots, "hotspots", &t)
    }

    /// Append a session summary to `sessions/<date>.md`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn add_session(&self, s: &SessionSummary) -> io::Result<()> {
        let day: String = s.date.chars().take(10).collect();
        let day = if day.len() == 10 && day.chars().all(|c| c.is_ascii_digit() || c == '-') {
            day
        } else {
            "undated".to_owned()
        };
        let dir = self.sessions_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{day}.md"));
        let mut text = fs::read_to_string(&path).unwrap_or_else(|_| format!("# Sessions {day}\n"));
        let _ = write!(
            text,
            "\n## {} — {}\n\n{}\n",
            one_line(&s.date),
            one_line(&s.task),
            crate::mask(s.result.trim())
        );
        write_atomic(&path, &text)
    }

    /// Projects linked with `kor context add`, in the order they were added.
    pub fn linked_projects(&self) -> Vec<PathBuf> {
        fs::read_to_string(self.dir.join(LINKS))
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
            .unwrap_or_default()
    }

    /// Link another project folder. Returns its canonical path; `Ok(None)` when already linked.
    ///
    /// # Errors
    /// When `other` is not an existing folder, is this project, or the list is full.
    pub fn link(&self, root: &Path, other: &Path) -> io::Result<Option<PathBuf>> {
        let other = crate::canonical(other)?;
        if !other.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a folder"));
        }
        if crate::canonical(root).is_ok_and(|r| r == other) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "this is the current project",
            ));
        }
        let mut links = self.linked_projects();
        if links.contains(&other) {
            return Ok(None);
        }
        if links.len() >= MAX_LINKS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("at most {MAX_LINKS} linked projects"),
            ));
        }
        links.push(other.clone());
        self.save_links(&links)?;
        Ok(Some(other))
    }

    /// Unlink a project (by its path as given or canonical). `Ok(false)` when it was not linked.
    ///
    /// # Errors
    /// When the list cannot be written.
    pub fn unlink(&self, other: &Path) -> io::Result<bool> {
        let canon = crate::canonical(other).ok();
        let mut links = self.linked_projects();
        let before = links.len();
        links.retain(|l| l != other && Some(l) != canon.as_ref());
        if links.len() == before {
            return Ok(false);
        }
        self.save_links(&links)?;
        Ok(true)
    }

    fn save_links(&self, links: &[PathBuf]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let text = serde_json::to_string_pretty(links).map_err(io::Error::other)?;
        write_atomic(&self.dir.join(LINKS), &text)
    }
}

/// Parse `## [date — ]title` entries with an optional `Refs:` line.
pub fn parse_decisions(text: &str) -> Vec<Decision> {
    let mut out: Vec<Decision> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let flush = |out: &mut Vec<Decision>, body: &mut Vec<&str>| {
        if let Some(d) = out.last_mut() {
            body.join("\n").trim().clone_into(&mut d.body);
        }
        body.clear();
    };
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            flush(&mut out, &mut body);
            let head = head.trim();
            let (date, title) = match head.get(..10) {
                Some(d)
                    if d.chars().all(|c| c.is_ascii_digit() || c == '-')
                        && d.matches('-').count() == 2 =>
                {
                    let rest = head[10..].trim_start();
                    let rest = rest
                        .strip_prefix('—')
                        .or_else(|| rest.strip_prefix('-'))
                        .or_else(|| rest.strip_prefix(':'))
                        .unwrap_or(rest);
                    (d.to_owned(), rest.trim().to_owned())
                }
                _ => (String::new(), head.to_owned()),
            };
            out.push(Decision {
                title,
                date,
                ..Decision::default()
            });
        } else if out.is_empty() {
            // Text before the first entry is the file's heading.
        } else if let Some(refs) = line
            .trim()
            .strip_prefix("Refs:")
            .or_else(|| line.trim().strip_prefix("refs:"))
        {
            if let Some(d) = out.last_mut() {
                d.refs.extend(
                    refs.split(',')
                        .map(|r| r.trim().trim_matches('`').trim().to_owned())
                        .filter(|r| !r.is_empty()),
                );
            }
        } else {
            body.push(line);
        }
    }
    flush(&mut out, &mut body);
    out
}

/// A section Kor wrote into a document (`<!-- kor:begin NAME hash=… -->` … `<!-- kor:end NAME -->`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub name: String,
    /// 1-based lines of the begin and end markers.
    pub start: usize,
    pub end: usize,
    /// The text inside changed since Kor wrote it: Kor keeps it and no longer rewrites it.
    pub edited: bool,
}

/// Kor's marked sections of `text`, matched the way [`ProjectMemory::set_section`] matches them.
pub fn sections(text: &str) -> Vec<Section> {
    const BEGIN: &str = "<!-- kor:begin ";
    let line_of = |at: usize| text[..at].matches('\n').count() + 1;
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(BEGIN) {
        let b = from + i;
        let Some(h) = text[b..].find("-->").map(|h| b + h) else {
            break;
        };
        let mut words = text[b + BEGIN.len()..h].split_whitespace();
        let name = words.next().unwrap_or_default().to_owned();
        let hash = words
            .find_map(|w| w.strip_prefix("hash="))
            .unwrap_or_default();
        let end = format!("<!-- kor:end {name} -->");
        let Some(e) = text[h..].find(&end).map(|e| h + e) else {
            from = h;
            continue;
        };
        let edited = crate::fnv(text[h + 3..e].trim()) != hash;
        out.push(Section {
            name,
            start: line_of(b),
            end: line_of(e),
            edited,
        });
        from = e + end.len();
    }
    out
}

fn valid_day(d: &str) -> bool {
    d == "undated" || (d.len() == 10 && d.chars().all(|c| c.is_ascii_digit() || c == '-'))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cell(s: &str) -> String {
    one_line(s).replace('|', "\\|")
}

/// Write via a sibling temp file and rename, so a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("kor-tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> (tempfile::TempDir, ProjectMemory) {
        let dir = tempfile::tempdir().unwrap();
        let m = ProjectMemory::new(dir.path(), Path::new("/work/p"));
        m.ensure().unwrap();
        (dir, m)
    }

    #[test]
    fn decisions_round_trip_with_hand_written_entries() {
        let (_d, m) = mem();
        m.add_decision(&Decision {
            title: "Use SQLite".into(),
            body: "One file, FTS5 built in.\npassword=hunter22".into(),
            date: "2026-11-10".into(),
            refs: vec![
                "crates/kor-memory/src/kb.rs".into(),
                "Knowledge::open".into(),
            ],
        })
        .unwrap();
        let path = m.path(Doc::Decisions);
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("\n## Why Go, not Rust\nHand-written by the user.\nrefs: `main.go`\n");
        fs::write(&path, text).unwrap();
        let ds = m.decisions();
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].date, "2026-11-10");
        assert_eq!(ds[0].title, "Use SQLite");
        assert_eq!(
            ds[0].refs,
            ["crates/kor-memory/src/kb.rs", "Knowledge::open"]
        );
        assert!(ds[0].body.starts_with("One file") && !ds[0].body.contains("hunter22"));
        assert_eq!(ds[1].title, "Why Go, not Rust");
        assert_eq!(ds[1].date, "");
        assert_eq!(ds[1].refs, ["main.go"]);
    }

    #[test]
    fn sections_are_replaced_but_user_edits_survive() {
        let (_d, m) = mem();
        let path = m.path(Doc::Hotspots);
        assert!(
            m.write_hotspots(&[Hotspot {
                path: "a.rs".into(),
                score: 3.0,
                reason: "churn".into()
            }])
            .unwrap()
        );
        // The user writes above and below Kor's section.
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{text}\nMy own note.\n")).unwrap();
        assert!(
            m.write_hotspots(&[Hotspot {
                path: "b.rs".into(),
                score: 1.5,
                reason: "x|y".into()
            }])
            .unwrap()
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("`b.rs` | 1.50 | x\\|y") && !text.contains("a.rs"),
            "{text}"
        );
        assert!(text.contains("My own note.") && text.contains("# Hotspots"));
        // Now the user edits inside Kor's section: Kor keeps it.
        fs::write(&path, text.replace("b.rs", "mine.rs")).unwrap();
        assert!(!m.write_hotspots(&[]).unwrap());
        assert!(fs::read_to_string(&path).unwrap().contains("mine.rs"));
        assert!(m.set_section(Doc::Architecture, "bad name", "x").is_err());
    }

    #[test]
    fn marked_sections_and_whole_document_writes() {
        let (_d, m) = mem();
        assert!(m.set_section(Doc::Architecture, "map", "a -> b").unwrap());
        let text = m.text(Doc::Architecture);
        let s = sections(&text);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].name.as_str(), s[0].edited), ("map", false));
        assert_eq!(s[0].end, s[0].start + 2);
        // The user saves the whole file with a change inside Kor's section.
        let saved = m
            .write_doc(
                Doc::Architecture,
                &text.replace("a -> b", "a -> c\ntoken=abcdef123"),
            )
            .unwrap();
        assert!(!saved.contains("abcdef123"));
        assert!(sections(&m.text(Doc::Architecture))[0].edited);
        assert!(!m.set_section(Doc::Architecture, "map", "x").unwrap());
        assert_eq!(Doc::from_file("patterns.md"), Some(Doc::Patterns));
        assert_eq!(Doc::from_file("../x.md"), None);
        assert!(sections("<!-- kor:begin x hash=1 -->\nno end").is_empty());
    }

    #[test]
    fn session_days_are_listed_newest_first_and_validated() {
        let (_d, m) = mem();
        for date in ["2026-11-09 10:00", "2026-11-10 09:00"] {
            m.add_session(&SessionSummary {
                date: date.into(),
                task: "t".into(),
                result: "r".into(),
            })
            .unwrap();
        }
        assert_eq!(m.session_days(), ["2026-11-10", "2026-11-09"]);
        assert!(
            m.session("2026-11-10")
                .unwrap()
                .contains("## 2026-11-10 09:00")
        );
        assert_eq!(m.session("../../secrets"), None);
    }

    #[test]
    fn sessions_and_links() {
        let (d, m) = mem();
        m.add_session(&SessionSummary {
            date: "2026-11-10 14:05".into(),
            task: "Fix login".into(),
            result: "ok: tests pass".into(),
        })
        .unwrap();
        let day = fs::read_to_string(m.sessions_dir().join("2026-11-10.md")).unwrap();
        assert!(day.contains("## 2026-11-10 14:05 — Fix login") && day.contains("tests pass"));

        let other = d.path().join("other");
        fs::create_dir_all(&other).unwrap();
        let root = Path::new("/work/p");
        let added = m.link(root, &other).unwrap().unwrap();
        assert_eq!(m.link(root, &other).unwrap(), None);
        assert_eq!(m.linked_projects(), std::slice::from_ref(&added));
        assert!(m.link(root, &d.path().join("missing")).is_err());
        assert!(m.unlink(&other).unwrap());
        assert!(!m.unlink(&other).unwrap());
        assert!(m.linked_projects().is_empty());
    }
}
