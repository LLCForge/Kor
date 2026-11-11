//! `kor quality`: an A–F grade per dimension (maintainability, test coverage, documentation,
//! security, performance) and overall, a prioritized improvement list as findings, and a
//! snapshot per commit in `<root>/.kor/quality/<commit>.json` so the trend is visible.
//!
//! Every metric carries its source. A dimension without data (no coverage report, no `kor perf`
//! run) says so and is left out of the overall grade instead of being guessed.

use std::path::Path;

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};

use super::{Grade, Project, debt, round1};
use crate::analysis::{Finding, Severity};

/// Functions above this cyclomatic complexity are "complex".
pub const COMPLEX: u32 = 10;
/// Files above this many code lines are "large".
pub const LARGE_FILE: u32 = 500;
/// Findings kept in the improvement list.
pub const MAX_FINDINGS: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    /// `functions`, `avg_complexity`, `complex_share`, `large_file_share`, `duplication`,
    /// `line_coverage`, `documented_public`, `readme`, `adr`, `critical`, …
    pub key: String,
    pub value: f64,
    /// `%`, `count`, `bool`, `cc`.
    pub unit: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    /// `maintainability`, `coverage`, `documentation`, `security`, `performance`.
    pub key: String,
    /// 0–100; `None` = no data.
    pub score: Option<f64>,
    pub grade: Option<Grade>,
    /// How the score is computed.
    pub formula: String,
    pub metrics: Vec<Metric>,
    /// Why there is no score, or a caveat (stale report, …).
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trend {
    /// Commit of the previous snapshot.
    pub since: String,
    pub since_at: i64,
    pub overall: Option<f64>,
    /// Dimension key → score change (points).
    pub dimensions: Vec<(String, f64)>,
    pub debt_hours: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
    pub commit: Option<String>,
    /// Uncommitted changes were part of the measurement.
    pub dirty: bool,
    pub taken_at: i64,
    pub overall: Option<f64>,
    pub grade: Option<Grade>,
    pub dimensions: Vec<Dimension>,
    pub debt_hours: f64,
    pub files: usize,
    /// Prioritized improvement list (worst first), at most [`MAX_FINDINGS`].
    pub findings: Vec<Finding>,
    /// All improvement findings before the cut.
    pub findings_total: usize,
    pub trend: Option<Trend>,
}

/// What is stored per commit (no findings, no author data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub commit: String,
    pub dirty: bool,
    pub taken_at: i64,
    pub overall: Option<f64>,
    pub dimensions: Vec<(String, Option<f64>)>,
    pub debt_hours: f64,
    pub coverage: Option<f64>,
}

fn metric(key: &str, value: f64, unit: &str, source: &str) -> Metric {
    Metric {
        key: key.into(),
        value: round1(value),
        unit: unit.into(),
        source: source.into(),
    }
}

fn dim(
    key: &str,
    score: Option<f64>,
    formula: &str,
    metrics: Vec<Metric>,
    note: Option<String>,
) -> Dimension {
    let score = score.map(|s| round1(s.clamp(0.0, 100.0)));
    Dimension {
        key: key.into(),
        score,
        grade: score.map(Grade::from_score),
        formula: formula.into(),
        metrics,
        note,
    }
}

#[allow(clippy::cast_precision_loss)]
fn share(part: usize, all: usize) -> f64 {
    if all == 0 {
        0.0
    } else {
        part as f64 / all as f64
    }
}

#[allow(clippy::cast_precision_loss)] // function counts
fn maintainability(p: &Project) -> Dimension {
    let code: Vec<_> = p.files.iter().filter(|f| !f.is_test).collect();
    let src = format!("tree-sitter: {} source files", code.len());
    if code.is_empty() {
        return dim(
            "maintainability",
            None,
            "",
            Vec::new(),
            Some("no-source".into()),
        );
    }
    let fns: Vec<_> = code.iter().flat_map(|f| &f.functions).collect();
    let complex = fns.iter().filter(|f| f.complexity > COMPLEX).count();
    let large = code.iter().filter(|f| f.code_lines > LARGE_FILE).count();
    #[allow(clippy::cast_precision_loss)]
    let avg = if fns.is_empty() {
        0.0
    } else {
        fns.iter().map(|f| f64::from(f.complexity)).sum::<f64>() / fns.len() as f64
    };
    let (cs, ls, dr) = (
        share(complex, fns.len()),
        share(large, code.len()),
        p.dup.ratio(),
    );
    let c = 1.0 - (4.0 * cs).min(1.0);
    let s = 1.0 - (4.0 * ls).min(1.0);
    let d = 1.0 - (5.0 * dr).min(1.0);
    let dup_src = format!(
        "{}-line windows repeated across {} files",
        super::code::DUP_WINDOW,
        p.dup.per_file.len()
    );
    dim(
        "maintainability",
        Some(100.0 * (0.5 * c + 0.25 * s + 0.25 * d)),
        "100 × (0.5 × (1 − min(1, 4 × complex share)) + 0.25 × (1 − min(1, 4 × large-file share)) + 0.25 × (1 − min(1, 5 × duplicated share))); complex = CC > 10, large = > 500 code lines",
        vec![
            metric("functions", fns.len() as f64, "count", &src),
            metric("avg_complexity", avg, "cc", &src),
            metric("complex_share", 100.0 * cs, "%", &src),
            metric("large_file_share", 100.0 * ls, "%", &src),
            metric("duplication", 100.0 * dr, "%", &dup_src),
        ],
        None,
    )
}

fn coverage(p: &Project) -> Dimension {
    let Some(c) = &p.coverage else {
        return dim("coverage", None, "", Vec::new(), Some("no-coverage".into()));
    };
    let pct = c.total.percent().unwrap_or(0.0);
    let head_time = p.commits.first().map_or(0, |x| x.time);
    let note = (c.modified < head_time).then(|| "coverage-stale".to_owned());
    #[allow(clippy::cast_precision_loss)]
    let metrics = vec![
        metric("line_coverage", pct, "%", &c.source()),
        metric(
            "covered_lines",
            c.total.covered as f64,
            "count",
            &c.source(),
        ),
        metric(
            "coverable_lines",
            c.total.total as f64,
            "count",
            &c.source(),
        ),
    ];
    dim("coverage", Some(pct), "line coverage %", metrics, note)
}

fn documentation(root: &Path, p: &Project) -> Dimension {
    let code: Vec<_> = p.files.iter().filter(|f| !f.is_test).collect();
    let public: u32 = code.iter().map(|f| f.public_items).sum();
    let documented: u32 = code.iter().map(|f| f.documented_public).sum();
    let readme = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("readme")
        })
        .map(|e| e.file_name().to_string_lossy().into_owned());
    let adr = [
        "docs/decisions",
        "docs/adr",
        "doc/adr",
        "adr",
        "docs/architecture/decisions",
    ]
    .into_iter()
    .find(|d| root.join(d).is_dir());
    let src = format!("tree-sitter: {public} public items");
    let ratio = (public > 0).then(|| f64::from(documented) / f64::from(public));
    let r = f64::from(u8::from(readme.is_some()));
    let a = f64::from(u8::from(adr.is_some()));
    let (score, formula) = match ratio {
        Some(q) => (
            80.0 * q + 10.0 * r + 10.0 * a,
            "80 × documented public share + 10 × README + 10 × ADR folder",
        ),
        None => (
            50.0 * (r + a),
            "50 × README + 50 × ADR folder (no public items)",
        ),
    };
    let mut metrics = vec![
        metric("public_items", f64::from(public), "count", &src),
        metric("documented_public", 100.0 * ratio.unwrap_or(0.0), "%", &src),
    ];
    metrics.push(metric(
        "readme",
        r,
        "bool",
        readme.as_deref().unwrap_or("-"),
    ));
    metrics.push(metric("adr", a, "bool", adr.unwrap_or("-")));
    dim("documentation", Some(score), formula, metrics, None)
}

fn security(root: &Path, l: &Localizer) -> Dimension {
    let report = crate::analysis::security::scan(
        root,
        &crate::analysis::security::Options { no_external: true },
        l,
    );
    let c = report.counts();
    let src = "kor audit --security (Kor rules; gitleaks/semgrep not run here)";
    #[allow(clippy::cast_precision_loss)]
    let score = 100.0 - 25.0 * c[0] as f64 - 10.0 * c[1] as f64 - 3.0 * c[2] as f64 - c[3] as f64;
    #[allow(clippy::cast_precision_loss)]
    let metrics = ["critical", "high", "medium", "low"]
        .iter()
        .zip(c)
        .map(|(k, n)| metric(k, n as f64, "count", src))
        .collect();
    dim(
        "security",
        Some(score),
        "100 − 25 × critical − 10 × high − 3 × medium − 1 × low",
        metrics,
        None,
    )
}

/// From the newest `kor perf` run: its measurements, and a score only when it was compared
/// with a baseline (a wall-time regression costs 2 points per percent).
fn performance(root: &Path) -> Dimension {
    let Some(newest) = crate::perf::list(root).into_iter().next() else {
        return dim("performance", None, "", Vec::new(), Some("no-perf".into()));
    };
    let Ok(r) = crate::perf::load_report(root, &newest.id) else {
        return dim(
            "performance",
            None,
            "",
            Vec::new(),
            Some("perf-unreadable".into()),
        );
    };
    let src = format!("kor perf {} ({}, {})", r.id, r.profiler, r.created);
    let mut metrics = Vec::new();
    if let Some(w) = r.wall_ms {
        metrics.push(metric("wall_ms", w, "ms", &src));
    }
    if let Some(h) = r.hottest.as_ref().or(r.top.first()) {
        metrics.push(metric(
            "hottest_share",
            h.self_pct,
            "%",
            &format!("{src}: {}", h.name),
        ));
    }
    let regression = r
        .compare
        .iter()
        .find(|c| c.metric == "wall_ms")
        .and_then(|c| c.change_pct);
    match (regression, &r.baseline) {
        (Some(change), Some(base)) => {
            metrics.push(metric(
                "wall_change",
                change,
                "%",
                &format!("{src} vs {base}"),
            ));
            dim(
                "performance",
                Some(100.0 - 2.0 * change.max(0.0)),
                "100 − 2 × wall-time regression % against the baseline run",
                metrics,
                None,
            )
        }
        _ => dim(
            "performance",
            None,
            "",
            metrics,
            Some("perf-no-baseline".into()),
        ),
    }
}

/// Improvement list: complex functions, large files, duplicated blocks, poorly covered files,
/// undocumented public items.
fn findings(p: &Project, l: &Localizer) -> Vec<Finding> {
    let mut out = Vec::new();
    let f = |rule: &str, sev: Severity, file: &str, line: u32, message: String| Finding {
        rule: rule.into(),
        source: "kor".into(),
        severity: sev,
        file: file.into(),
        line,
        message,
        ..Finding::default()
    };
    for file in p.files.iter().filter(|x| !x.is_test) {
        for func in file.functions.iter().filter(|x| x.complexity > COMPLEX) {
            let sev = match func.complexity {
                c if c > 25 => Severity::High,
                c if c > 15 => Severity::Medium,
                _ => Severity::Low,
            };
            out.push(f(
                "kor.quality.complexity",
                sev,
                &file.path,
                func.line,
                l.t_args(
                    "metrics-f-complexity",
                    &args!("name" => func.name.clone(), "cc" => func.complexity, "limit" => COMPLEX, "line" => func.line, "end" => func.end),
                ),
            ));
        }
        if file.code_lines > LARGE_FILE {
            let sev = if file.code_lines > 2 * LARGE_FILE {
                Severity::Medium
            } else {
                Severity::Low
            };
            out.push(f(
                "kor.quality.file-size",
                sev,
                &file.path,
                1,
                l.t_args(
                    "metrics-f-file-size",
                    &args!("lines" => file.code_lines, "limit" => LARGE_FILE),
                ),
            ));
        }
        if let Some(c) = &p.coverage
            && let Some(lines) = c.file(&file.path)
            && lines.total >= 20
            && let Some(pct) = lines.percent()
            && pct < 50.0
        {
            out.push(f(
                "kor.quality.coverage",
                Severity::Medium,
                &file.path,
                1,
                l.t_args(
                    "metrics-f-coverage",
                    &args!("pct" => format!("{pct:.0}"), "covered" => lines.covered, "total" => lines.total, "source" => c.source()),
                ),
            ));
        }
        for (name, line) in file.undocumented.iter().take(3) {
            out.push(f(
                "kor.quality.undocumented",
                Severity::Info,
                &file.path,
                *line,
                l.t_args("metrics-f-undocumented", &args!("name" => name.clone())),
            ));
        }
    }
    for b in &p.dup.blocks {
        out.push(f(
            "kor.quality.duplication",
            Severity::Low,
            &b.file,
            b.line,
            l.t_args(
                "metrics-f-duplication",
                &args!("line" => b.line, "end" => b.end, "other" => b.other.clone(), "other_line" => b.other_line),
            ),
        ));
    }
    // Worst first; within a severity the most complex / largest first (the message order is
    // stable because rules are grouped).
    out.sort_by(|a, b| {
        (a.severity, &a.rule, &a.file, a.line).cmp(&(b.severity, &b.rule, &b.file, b.line))
    });
    out
}

/// Measure the project and store the snapshot of the current commit.
pub fn run(root: &Path, l: &Localizer) -> QualityReport {
    let p = Project::gather(root);
    let mut r = measure(root, &p, l);
    r.trend = trend(root, &r);
    if let Some(commit) = &r.commit {
        let _ = save(root, &snapshot(&r, commit));
    }
    r
}

/// The report without storing anything.
pub fn measure(root: &Path, p: &Project, l: &Localizer) -> QualityReport {
    let dimensions = vec![
        maintainability(p),
        coverage(p),
        documentation(root, p),
        security(root, l),
        performance(root),
    ];
    let scored: Vec<f64> = dimensions.iter().filter_map(|d| d.score).collect();
    #[allow(clippy::cast_precision_loss)]
    let overall =
        (!scored.is_empty()).then(|| round1(scored.iter().sum::<f64>() / scored.len() as f64));
    let all = findings(p, l);
    let debt = debt::debt(root, p, debt::DEFAULT_THRESHOLD_HOURS);
    let dirty = p.is_repo
        && super::git::git(root, &["status", "--porcelain", "--untracked-files=no"])
            .is_ok_and(|s| !s.trim().is_empty());
    QualityReport {
        commit: p.head.clone(),
        dirty,
        taken_at: p.now,
        overall,
        grade: overall.map(Grade::from_score),
        dimensions,
        debt_hours: debt.total_hours,
        files: p.files.len(),
        findings_total: all.len(),
        findings: all.into_iter().take(MAX_FINDINGS).collect(),
        trend: None,
    }
}

fn snapshot(r: &QualityReport, commit: &str) -> Snapshot {
    Snapshot {
        commit: commit.to_owned(),
        dirty: r.dirty,
        taken_at: r.taken_at,
        overall: r.overall,
        dimensions: r
            .dimensions
            .iter()
            .map(|d| (d.key.clone(), d.score))
            .collect(),
        debt_hours: r.debt_hours,
        coverage: r
            .dimensions
            .iter()
            .find(|d| d.key == "coverage")
            .and_then(|d| d.score),
    }
}

fn dir(root: &Path) -> std::path::PathBuf {
    super::kor_dir(root, "quality")
}

fn save(root: &Path, s: &Snapshot) -> std::io::Result<()> {
    if s.commit.len() < 7 || !s.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(std::io::Error::other("invalid commit id"));
    }
    super::write_json(&dir(root).join(format!("{}.json", s.commit)), s)
}

/// Stored snapshots, oldest first.
pub fn history(root: &Path) -> Vec<Snapshot> {
    let mut out: Vec<Snapshot> = std::fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    out.sort_by_key(|s| s.taken_at);
    out
}

/// Change since the newest snapshot of another commit.
fn trend(root: &Path, r: &QualityReport) -> Option<Trend> {
    let prev = history(root)
        .into_iter()
        .rev()
        .find(|s| Some(&s.commit) != r.commit.as_ref())?;
    let dimensions = r
        .dimensions
        .iter()
        .filter_map(|d| {
            let before = prev.dimensions.iter().find(|(k, _)| *k == d.key)?.1?;
            Some((d.key.clone(), round1(d.score? - before)))
        })
        .collect();
    Some(Trend {
        since: prev.commit.clone(),
        since_at: prev.taken_at,
        overall: r.overall.zip(prev.overall).map(|(a, b)| round1(a - b)),
        dimensions,
        debt_hours: round1(r.debt_hours - prev.debt_hours),
    })
}

#[cfg(test)]
#[allow(clippy::format_push_string, clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::analysis::metrics::fixture;

    #[test]
    fn grades_snapshots_and_trend() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fixture::init(d);
        fixture::write(d, "README.md", "# x\n");
        fixture::write(
            d,
            "src/lib.rs",
            "/// Doc.\npub fn a() -> u32 { 1 }\npub fn b() -> u32 { 2 }\n",
        );
        fixture::commit(d, "feat: a", "Ayşe Dev", "ayse@example.com", 3);
        let l = Localizer::new(kor_i18n::Lang::En);
        let first = run(d, &l);
        let doc = first
            .dimensions
            .iter()
            .find(|x| x.key == "documentation")
            .unwrap();
        // 1 of 2 public items documented: 80 × 0.5 + README 10
        assert_eq!(doc.score, Some(50.0));
        let cov = first
            .dimensions
            .iter()
            .find(|x| x.key == "coverage")
            .unwrap();
        assert!(cov.score.is_none() && cov.note.is_some());
        let perf = first
            .dimensions
            .iter()
            .find(|x| x.key == "performance")
            .unwrap();
        assert!(perf.score.is_none());
        assert!(first.trend.is_none());
        assert_eq!(history(d).len(), 1);

        fixture::write(
            d,
            "src/lib.rs",
            "/// Doc.\npub fn a() -> u32 { 1 }\n/// Doc.\npub fn b() -> u32 { 2 }\n",
        );
        fixture::write(
            d,
            "lcov.info",
            "SF:src/lib.rs\nDA:2,1\nDA:4,0\nend_of_record\n",
        );
        fixture::commit(d, "docs: b", "Ayşe Dev", "ayse@example.com", 1);
        let second = run(d, &l);
        let t = second.trend.expect("trend against the first commit");
        assert_eq!(t.since, first.commit.unwrap());
        assert!(
            t.dimensions
                .iter()
                .any(|(k, v)| k == "documentation" && *v == 40.0)
        );
        let cov = second
            .dimensions
            .iter()
            .find(|x| x.key == "coverage")
            .unwrap();
        assert_eq!(cov.score, Some(50.0));
        assert_eq!(cov.metrics[0].source.split(' ').next(), Some("lcov.info"));
        assert_eq!(history(d).len(), 2);
    }

    #[test]
    fn complex_functions_become_findings() {
        let mut src = String::from("pub fn tangled(x: i32) -> i32 {\n");
        for i in 0..20 {
            src.push_str(&format!("    if x == {i} {{ return {i}; }}\n"));
        }
        src.push_str("    0\n}\n");
        let dir = tempfile::tempdir().unwrap();
        fixture::write(dir.path(), "src/a.rs", &src);
        let p = Project::gather(dir.path());
        let f = findings(&p, &Localizer::new(kor_i18n::Lang::En));
        let c = f
            .iter()
            .find(|x| x.rule == "kor.quality.complexity")
            .unwrap();
        assert_eq!(
            (c.file.to_str(), c.line, c.severity),
            (Some("src/a.rs"), 1, Severity::Medium)
        );
        assert!(c.message.contains("21"));
    }
}
