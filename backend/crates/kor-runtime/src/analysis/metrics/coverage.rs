//! Coverage reports the project already produced: lcov (`lcov.info`), Cobertura XML (also
//! coverage.py's `coverage.xml` and `cargo tarpaulin --out xml`), coverage.py JSON, Go
//! `-coverprofile`, tarpaulin JSON and `llvm-cov export` JSON. Kor never runs a test or coverage
//! tool to get them (that would execute project code without asking); with no report the
//! coverage dimension says "no data".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reports larger than this are not read.
const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

/// File names looked for (case-insensitive), at the root and up to three directories down.
const NAMES: [&str; 13] = [
    "lcov.info",
    "coverage.lcov",
    "cobertura.xml",
    "cobertura-coverage.xml",
    "coverage.xml",
    "coverage.json",
    "coverage-final.json",
    "coverage.out",
    "cover.out",
    "coverage.txt",
    "c.out",
    "tarpaulin-report.json",
    "llvm-cov.json",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportKind {
    Lcov,
    Cobertura,
    CoveragePy,
    GoCover,
    Tarpaulin,
    LlvmCov,
}

impl ReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lcov => "lcov",
            Self::Cobertura => "cobertura",
            Self::CoveragePy => "coverage.py",
            Self::GoCover => "go cover",
            Self::Tarpaulin => "tarpaulin",
            Self::LlvmCov => "llvm-cov",
        }
    }
}

/// Covered / coverable lines (statements for Go).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub covered: u64,
    pub total: u64,
}

impl Lines {
    pub fn percent(self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)] // line counts
        (self.total > 0).then(|| self.covered as f64 * 100.0 / self.total as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub kind: ReportKind,
    /// Project-relative path of the report.
    pub report: String,
    /// Report file modification time (Unix seconds).
    pub modified: i64,
    pub total: Lines,
    /// Project-relative source path → lines.
    pub files: BTreeMap<String, Lines>,
}

impl Coverage {
    /// `lcov.info (lcov, 2026-11-10)` style source text.
    pub fn source(&self) -> String {
        let day = chrono::DateTime::from_timestamp(self.modified, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        format!("{} ({}, {day})", self.report, self.kind.as_str())
    }

    pub fn file(&self, path: &str) -> Option<Lines> {
        self.files.get(path).copied()
    }
}

/// Candidate report files, newest first.
pub fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(i64, PathBuf)> = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(false) // reports are usually git-ignored
        .max_depth(Some(4))
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir())
                && matches!(n.as_ref(), "node_modules" | ".git" | "vendor" | ".venv"))
        })
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            NAMES.contains(&n.as_str())
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            (m.len() <= MAX_REPORT_BYTES).then(|| (mtime(&m), e.into_path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

fn mtime(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// The newest readable coverage report of the project, if any.
pub fn load(root: &Path) -> Option<Coverage> {
    candidates(root).into_iter().find_map(|p| read(root, &p))
}

/// Parse one report; `None` when the format is not recognised or it covers nothing.
pub fn read(root: &Path, path: &Path) -> Option<Coverage> {
    let text = std::fs::read_to_string(path).ok()?;
    let (kind, files) = parse(root, &text)?;
    let mut total = Lines::default();
    for l in files.values() {
        total.covered += l.covered;
        total.total += l.total;
    }
    (total.total > 0).then(|| Coverage {
        kind,
        report: super::rel(root, path),
        modified: std::fs::metadata(path).map(|m| mtime(&m)).unwrap_or(0),
        total,
        files,
    })
}

/// Detect the format from the content and parse it.
pub fn parse(root: &Path, text: &str) -> Option<(ReportKind, BTreeMap<String, Lines>)> {
    let t = text.trim_start();
    if t.starts_with("mode:") {
        return Some((ReportKind::GoCover, go_cover(root, text)));
    }
    if t.starts_with('<') {
        return Some((ReportKind::Cobertura, cobertura(root, text)));
    }
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).ok()?;
        if v.get("type").and_then(Value::as_str) == Some("llvm.coverage.json.export") {
            return Some((ReportKind::LlvmCov, llvm_cov(root, &v)));
        }
        if v.get("files").is_some_and(Value::is_array) {
            return Some((ReportKind::Tarpaulin, tarpaulin(root, &v)));
        }
        if v.get("files").is_some_and(Value::is_object) {
            return Some((ReportKind::CoveragePy, coverage_py(root, &v)));
        }
        return None;
    }
    if text.contains("SF:") && text.contains("end_of_record") {
        return Some((ReportKind::Lcov, lcov(root, text)));
    }
    None
}

/// Report paths are absolute, relative to the report, or relative to the root: make them
/// project-relative with `/`.
fn norm(root: &Path, p: &str) -> String {
    let p = p.trim().replace('\\', "/");
    let root_s = root.to_string_lossy().replace('\\', "/");
    let root_s = root_s.trim_end_matches('/');
    let stripped = p
        .strip_prefix(root_s)
        .map(|r| r.trim_start_matches('/').to_owned());
    stripped.unwrap_or_else(|| p.trim_start_matches("./").to_owned())
}

fn lcov(root: &Path, text: &str) -> BTreeMap<String, Lines> {
    let mut out: BTreeMap<String, Lines> = BTreeMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("SF:") {
            cur = Some(norm(root, p));
        } else if let Some(d) = line.strip_prefix("DA:")
            && let Some(f) = &cur
        {
            let hits = d
                .split(',')
                .nth(1)
                .and_then(|h| h.trim().parse::<f64>().ok());
            let e = out.entry(f.clone()).or_default();
            e.total += 1;
            if hits.is_some_and(|h| h > 0.0) {
                e.covered += 1;
            }
        } else if line.starts_with("end_of_record") {
            cur = None;
        }
    }
    out
}

static CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<class\b[^>]*\bfilename="([^"]+)"[^>]*>"#).expect("valid regex")
});
/// `name="value"` inside one tag's text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let tag = tag.split('>').next().unwrap_or(tag);
    tag.split(&format!(" {name}=\"")).nth(1)?.split('"').next()
}

static SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<source>([^<]+)</source>").expect("valid regex"));

/// Cobertura: `<class filename=...>` blocks with `<line number hits>`; `filename` is relative
/// to one of the `<source>` directories.
fn cobertura(root: &Path, text: &str) -> BTreeMap<String, Lines> {
    let sources: Vec<String> = SOURCE
        .captures_iter(text)
        .map(|c| norm(root, &c[1]))
        .collect();
    let mut out: BTreeMap<String, Lines> = BTreeMap::new();
    let starts: Vec<(usize, String)> = CLASS
        .captures_iter(text)
        .filter_map(|c| Some((c.get(0)?.end(), c[1].to_owned())))
        .collect();
    for (i, (start, file)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(text.len(), |n| n.0);
        let body = &text[*start..end];
        let body = body.split("</class>").next().unwrap_or(body);
        let mut path = norm(root, file);
        if !root.join(&path).exists()
            && let Some(s) = sources
                .iter()
                .find(|s| !s.is_empty() && root.join(s).join(&path).exists())
        {
            path = format!("{}/{path}", s.trim_end_matches('/'));
        }
        let e = out.entry(path).or_default();
        // A class can repeat a line (methods block + lines block); count each number once.
        let mut seen = std::collections::BTreeMap::new();
        for l in body.split("<line").skip(1) {
            let num = attr(l, "number").and_then(|n| n.parse::<u64>().ok());
            let hits = attr(l, "hits")
                .and_then(|h| h.parse::<u64>().ok())
                .unwrap_or(0);
            if let Some(n) = num {
                let h: &mut u64 = seen.entry(n).or_default();
                *h = (*h).max(hits);
            }
        }
        e.total += seen.len() as u64;
        e.covered += seen.values().filter(|h| **h > 0).count() as u64;
    }
    out
}

fn coverage_py(root: &Path, v: &Value) -> BTreeMap<String, Lines> {
    let mut out = BTreeMap::new();
    if let Some(files) = v.get("files").and_then(Value::as_object) {
        for (path, f) in files {
            let s = f.get("summary");
            let get = |k: &str| s.and_then(|s| s.get(k)).and_then(Value::as_u64);
            let (covered, total) =
                if let (Some(c), Some(t)) = (get("covered_lines"), get("num_statements")) {
                    (c, t)
                } else {
                    let n = |k: &str| f.get(k).and_then(Value::as_array).map_or(0, Vec::len) as u64;
                    (
                        n("executed_lines"),
                        n("executed_lines") + n("missing_lines"),
                    )
                };
            out.insert(norm(root, path), Lines { covered, total });
        }
    }
    out
}

fn tarpaulin(root: &Path, v: &Value) -> BTreeMap<String, Lines> {
    let mut out = BTreeMap::new();
    for f in v
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = match f.get("path") {
            Some(Value::Array(parts)) => {
                let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                let joined = parts.join("/");
                if parts.first() == Some(&"/") {
                    joined[1..].to_owned()
                } else {
                    joined
                }
            }
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        let covered = f.get("covered").and_then(Value::as_u64).unwrap_or(0);
        let total = f.get("coverable").and_then(Value::as_u64).unwrap_or(0);
        out.insert(norm(root, &path), Lines { covered, total });
    }
    out
}

fn llvm_cov(root: &Path, v: &Value) -> BTreeMap<String, Lines> {
    let mut out = BTreeMap::new();
    for d in v
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for f in d
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = f.get("filename").and_then(Value::as_str) else {
                continue;
            };
            let lines = f.pointer("/summary/lines");
            let get = |k: &str| {
                lines
                    .and_then(|l| l.get(k))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            out.insert(
                norm(root, name),
                Lines {
                    covered: get("covered"),
                    total: get("count"),
                },
            );
        }
    }
    out
}

/// `mode: set` then `pkg/path/file.go:12.5,14.2 3 1` (statements, count). Import paths are
/// mapped to files with the `module` line of `go.mod`.
fn go_cover(root: &Path, text: &str) -> BTreeMap<String, Lines> {
    let module = std::fs::read_to_string(root.join("go.mod"))
        .ok()
        .and_then(|m| {
            m.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("module ")
                    .map(|s| s.trim().to_owned())
            })
        })
        .unwrap_or_default();
    // A block can be listed several times (one per test binary): keep the highest count.
    let mut blocks: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
    for line in text.lines().skip(1) {
        let mut parts = line.rsplitn(3, ' ');
        let (Some(count), Some(stmts), Some(loc)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(count), Ok(stmts)) = (count.parse::<u64>(), stmts.parse::<u64>()) else {
            continue;
        };
        let Some((file, range)) = loc.rsplit_once(':') else {
            continue;
        };
        let rel = if module.is_empty() {
            file.to_owned()
        } else {
            file.strip_prefix(&module)
                .map_or_else(|| file.to_owned(), |r| r.trim_start_matches('/').to_owned())
        };
        let e = blocks.entry((rel, range.to_owned())).or_insert((stmts, 0));
        e.1 = e.1.max(count);
    }
    let mut out: BTreeMap<String, Lines> = BTreeMap::new();
    for ((file, _), (stmts, count)) in blocks {
        let e = out.entry(norm(root, &file)).or_default();
        e.total += stmts;
        if count > 0 {
            e.covered += stmts;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }

    #[test]
    fn lcov_counts_da_lines() {
        let t = "TN:\nSF:/proj/src/a.rs\nDA:1,1\nDA:2,0\nDA:3,5\nend_of_record\nSF:src/b.rs\nDA:1,0\nend_of_record\n";
        let (k, f) = parse(&root(), t).unwrap();
        assert_eq!(k, ReportKind::Lcov);
        assert_eq!(
            f["src/a.rs"],
            Lines {
                covered: 2,
                total: 3
            }
        );
        assert_eq!(
            f["src/b.rs"],
            Lines {
                covered: 0,
                total: 1
            }
        );
    }

    #[test]
    fn cobertura_and_coverage_py_xml() {
        let t = r#"<?xml version="1.0" ?><coverage><sources><source>/proj</source></sources><packages><package><classes>
<class name="a" filename="app/a.py" line-rate="0.5"><methods/><lines><line number="1" hits="1"/><line number="2" hits="0"/></lines></class>
<class name="b" filename="app/b.py"><lines><line number="1" hits="3" branch="false"/></lines></class>
</classes></package></packages></coverage>"#;
        let (k, f) = parse(&root(), t).unwrap();
        assert_eq!(k, ReportKind::Cobertura);
        assert_eq!(
            f["app/a.py"],
            Lines {
                covered: 1,
                total: 2
            }
        );
        assert_eq!(
            f["app/b.py"],
            Lines {
                covered: 1,
                total: 1
            }
        );
    }

    #[test]
    fn json_formats() {
        let py = r#"{"meta":{},"files":{"app/a.py":{"summary":{"covered_lines":3,"num_statements":4}}}}"#;
        let (k, f) = parse(&root(), py).unwrap();
        assert_eq!(k, ReportKind::CoveragePy);
        assert_eq!(
            f["app/a.py"],
            Lines {
                covered: 3,
                total: 4
            }
        );
        let tar = r#"{"files":[{"path":["/","proj","src","lib.rs"],"covered":7,"coverable":10}]}"#;
        let (k, f) = parse(&root(), tar).unwrap();
        assert_eq!(k, ReportKind::Tarpaulin);
        assert_eq!(
            f["src/lib.rs"],
            Lines {
                covered: 7,
                total: 10
            }
        );
        let llvm = r#"{"type":"llvm.coverage.json.export","version":"2.0.1","data":[{"files":[{"filename":"/proj/src/main.rs","summary":{"lines":{"count":20,"covered":15}}}]}]}"#;
        let (k, f) = parse(&root(), llvm).unwrap();
        assert_eq!(k, ReportKind::LlvmCov);
        assert_eq!(
            f["src/main.rs"],
            Lines {
                covered: 15,
                total: 20
            }
        );
    }

    #[test]
    fn go_cover_profile() {
        let t = "mode: set\nexample.com/m/pkg/a.go:3.10,5.2 2 1\nexample.com/m/pkg/a.go:6.10,8.2 3 0\nexample.com/m/pkg/a.go:6.10,8.2 3 1\nexample.com/m/b.go:1.1,2.2 1 0\n";
        let f = go_cover(&root(), t);
        // no go.mod at /proj: import paths stay as they are
        assert_eq!(
            f["example.com/m/pkg/a.go"],
            Lines {
                covered: 5,
                total: 5
            }
        );
        assert_eq!(
            f["example.com/m/b.go"],
            Lines {
                covered: 0,
                total: 1
            }
        );
        assert_eq!(
            Lines {
                covered: 1,
                total: 4
            }
            .percent(),
            Some(25.0)
        );
    }

    #[test]
    fn reports_are_found_and_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("coverage")).unwrap();
        std::fs::write(
            dir.path().join("coverage/lcov.info"),
            "SF:src/x.ts\nDA:1,1\nDA:2,1\nend_of_record\n",
        )
        .unwrap();
        let c = load(dir.path()).unwrap();
        assert_eq!(c.report, "coverage/lcov.info");
        assert_eq!(c.total.percent(), Some(100.0));
        assert!(c.source().starts_with("coverage/lcov.info (lcov, "));
    }
}
