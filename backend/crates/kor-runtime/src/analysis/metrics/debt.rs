//! `kor debt`: technical debt per file in estimated hours, from complexity, size, duplication,
//! churn and missing tests, with the formula stated next to every number (§9.1). Monthly growth
//! comes from earlier `kor quality` snapshots, else from the same estimate at the commit of 30
//! days ago (read from git, nothing checked out). All hours are estimates and printed with `≈`.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::code::{self, FileStats};
use super::coverage::Coverage;
use super::{Project, git, round1};

/// The formula, printed with every report.
pub const FORMULA: &str = "hours ≈ (0.25 × Σ max(0, CC − 10) + 0.5 × max(0, code lines − 400) / 100 + duplicated lines / 40) × (1 + min(commits in 180 days, 20) / 10) × test factor; test factor = 1 + 0.5 × uncovered share (coverage report), else 1.0 with tests / 1.5 without";
/// Churn window.
pub const CHURN_DAYS: i64 = 180;
pub const DEFAULT_THRESHOLD_HOURS: f64 = 400.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DebtItem {
    pub file: String,
    pub hours: f64,
    /// Σ max(0, CC − 10) over the file's functions.
    pub complexity_excess: u32,
    /// The most complex function: (name, line, complexity).
    pub worst: Option<(String, u32, u32)>,
    pub code_lines: u32,
    pub duplicated_lines: u32,
    pub churn: u32,
    pub test_factor: f64,
    /// Where the test factor came from.
    pub test: TestBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TestBasis {
    /// Line coverage of the file in the report (`source`).
    Coverage {
        percent: f64,
        source: String,
    },
    /// The report exists but does not list the file.
    NotInReport {
        source: String,
    },
    /// No report: a test file or test markers were found.
    TestsFound,
    NoTests,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Growth {
    /// Hours per 30 days (negative = shrinking).
    pub per_month: f64,
    pub basis: GrowthBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrowthBasis {
    /// Stored `kor quality` snapshots between two days.
    Snapshots {
        count: usize,
        from: String,
        to: String,
    },
    /// The same estimate at the commit `rev` of 30 days ago (both sides without coverage).
    Git { rev: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Debt {
    pub formula: String,
    pub total_hours: f64,
    /// Files with any debt, most first.
    pub items: Vec<DebtItem>,
    pub growth: Option<Growth>,
    pub threshold_hours: f64,
    /// Months until `threshold_hours` at the current growth; `Some(0.0)` when already above.
    pub months_to_threshold: Option<f64>,
    pub files_scanned: usize,
}

fn tested_stems(files: &[FileStats]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|f| f.is_test)
        .map(|f| {
            let name = f
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&f.path)
                .to_ascii_lowercase();
            let stem = name.split('.').next().unwrap_or(&name).to_owned();
            stem.trim_start_matches("test_")
                .trim_end_matches("_test")
                .trim_end_matches("_spec")
                .trim_end_matches("tests")
                .trim_end_matches("test")
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Debt of each non-test file. `coverage` is per-file line coverage when a report exists.
#[allow(clippy::cast_precision_loss)] // counts are small
pub fn items(
    files: &[FileStats],
    dup: &code::Duplication,
    churn: &HashMap<String, u32, impl std::hash::BuildHasher>,
    coverage: Option<&Coverage>,
) -> Vec<DebtItem> {
    let tested = tested_stems(files);
    let mut out: Vec<DebtItem> = files
        .iter()
        .filter(|f| !f.is_test)
        .filter_map(|f| {
            let excess: u32 = f
                .functions
                .iter()
                .map(|x| x.complexity.saturating_sub(10))
                .sum();
            let dup_lines = dup.per_file.get(&f.path).copied().unwrap_or(0);
            let base = 0.25 * f64::from(excess)
                + 0.5 * f64::from(f.code_lines.saturating_sub(400)) / 100.0
                + f64::from(dup_lines) / 40.0;
            if base <= 0.0 {
                return None;
            }
            let commits = churn.get(&f.path).copied().unwrap_or(0);
            let churn_factor = 1.0 + f64::from(commits.min(20)) / 10.0;
            let (test_factor, test) = if let Some(c) = coverage {
                match c.file(&f.path).and_then(super::coverage::Lines::percent) {
                    Some(p) => (
                        1.0 + 0.5 * (1.0 - p / 100.0),
                        TestBasis::Coverage {
                            percent: round1(p),
                            source: c.source(),
                        },
                    ),
                    None => (1.5, TestBasis::NotInReport { source: c.source() }),
                }
            } else {
                let name = f
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&f.path)
                    .to_ascii_lowercase();
                let stem = name.split('.').next().unwrap_or(&name);
                if f.has_tests || tested.contains(stem) {
                    (1.0, TestBasis::TestsFound)
                } else {
                    (1.5, TestBasis::NoTests)
                }
            };
            let worst = f
                .functions
                .iter()
                .max_by_key(|x| x.complexity)
                .filter(|x| x.complexity > 10)
                .map(|x| (x.name.clone(), x.line, x.complexity));
            Some(DebtItem {
                file: f.path.clone(),
                hours: round1(base * churn_factor * test_factor),
                complexity_excess: excess,
                worst,
                code_lines: f.code_lines,
                duplicated_lines: dup_lines,
                churn: commits,
                test_factor,
                test,
            })
        })
        .filter(|i| i.hours > 0.0)
        .collect();
    out.sort_by(|a, b| b.hours.total_cmp(&a.hours).then(a.file.cmp(&b.file)));
    out
}

/// Total hours of `files` without coverage data (used to compare two points in time alike).
fn total_without_coverage(
    files: &[FileStats],
    churn: &HashMap<String, u32, impl std::hash::BuildHasher>,
) -> f64 {
    let dup = code::duplication(files);
    items(files, &dup, churn, None)
        .iter()
        .map(|i| i.hours)
        .sum()
}

/// The estimate at the last commit before `ts`, from git objects (no checkout).
fn total_at(root: &Path, rev: &str, ts: i64, commits: &[git::Commit]) -> Option<f64> {
    let tree = kor_graph::git::tree(root, rev).ok()?;
    let wanted: Vec<(String, String)> = tree
        .into_iter()
        .filter(|(p, size, _)| *size <= code::MAX_FILE_BYTES && code::lang_of(p).is_some())
        .filter(|(p, _, _)| {
            !p.split('/')
                .any(|seg| matches!(seg, "node_modules" | "vendor" | "target" | "dist"))
        })
        .map(|(p, _, id)| (p, id))
        .collect();
    let ids: Vec<String> = wanted.iter().map(|(_, id)| id.clone()).collect();
    let blobs = kor_graph::git::blobs(root, &ids).ok()?;
    let mut parser = tree_sitter::Parser::new();
    let files: Vec<FileStats> = wanted
        .iter()
        .zip(blobs)
        .filter_map(|((p, _), bytes)| {
            let text = String::from_utf8(bytes).ok()?;
            Some(code::analyze_with(&mut parser, code::lang_of(p)?, p, &text))
        })
        .collect();
    let churn = git::churn(
        &commits
            .iter()
            .filter(|c| c.time <= ts)
            .cloned()
            .collect::<Vec<_>>(),
        ts - CHURN_DAYS * 86_400,
    );
    Some(total_without_coverage(&files, &churn))
}

/// Growth per 30 days from stored quality snapshots (≥ 14 days apart), else from git.
#[allow(clippy::cast_precision_loss)]
fn growth(root: &Path, p: &Project) -> Option<Growth> {
    let snaps = super::quality::history(root);
    let recent: Vec<_> = snaps
        .iter()
        .filter(|s| s.taken_at >= p.now - 180 * 86_400)
        .collect();
    if let (Some(first), Some(last)) = (recent.first(), recent.last())
        && last.taken_at - first.taken_at >= 14 * 86_400
    {
        let days = (last.taken_at - first.taken_at) as f64 / 86_400.0;
        let day = |t: i64| {
            chrono::DateTime::from_timestamp(t, 0)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default()
        };
        return Some(Growth {
            per_month: round1((last.debt_hours - first.debt_hours) / days * 30.0),
            basis: GrowthBasis::Snapshots {
                count: recent.len(),
                from: day(first.taken_at),
                to: day(last.taken_at),
            },
        });
    }
    if !p.is_repo {
        return None;
    }
    let then = p.now - 30 * 86_400;
    let rev = git::rev_before(root, then)?;
    if Some(&rev) == p.head.as_ref() {
        return None;
    }
    let past = total_at(root, &rev, then, &p.commits)?;
    let now_cmp = total_without_coverage(&p.files, &p.churn);
    Some(Growth {
        per_month: round1(now_cmp - past),
        basis: GrowthBasis::Git {
            rev: rev.chars().take(10).collect(),
        },
    })
}

/// The debt inventory of the project.
pub fn debt(root: &Path, p: &Project, threshold_hours: f64) -> Debt {
    let items = items(&p.files, &p.dup, &p.churn, p.coverage.as_ref());
    let total: f64 = round1(items.iter().map(|i| i.hours).sum());
    let growth = growth(root, p);
    let months_to_threshold = growth.as_ref().and_then(|g| {
        if total >= threshold_hours {
            Some(0.0)
        } else if g.per_month > 0.0 {
            Some(round1((threshold_hours - total) / g.per_month))
        } else {
            None
        }
    });
    Debt {
        formula: FORMULA.to_owned(),
        total_hours: total,
        items,
        growth,
        threshold_hours,
        months_to_threshold,
        files_scanned: p.files.len(),
    }
}

#[cfg(test)]
#[allow(clippy::format_push_string)]
mod tests {
    use super::*;
    use crate::analysis::metrics::fixture;

    fn complex_fn(name: &str, branches: usize) -> String {
        let mut s = format!("pub fn {name}(x: i32) -> i32 {{\n");
        for i in 0..branches {
            s.push_str(&format!("    if x == {i} {{ return {i}; }}\n"));
        }
        s.push_str("    0\n}\n");
        s
    }

    #[test]
    fn formula_parts_add_up() {
        let f = code::analyze(
            kor_graph::lang::Lang::Rust,
            "src/a.rs",
            &complex_fn("big", 14),
        );
        let dup = code::duplication(std::slice::from_ref(&f));
        let churn = HashMap::from([("src/a.rs".to_owned(), 5)]);
        let it = items(&[f], &dup, &churn, None);
        // CC 15 → excess 5 → 1.25 h; churn 5 → ×1.5; no tests → ×1.5
        assert_eq!(it.len(), 1);
        assert!((it[0].hours - 2.8).abs() < 0.05, "{}", it[0].hours);
        assert_eq!(it[0].worst, Some(("big".into(), 1, 15)));
        assert_eq!(it[0].test, TestBasis::NoTests);
    }

    #[test]
    fn growth_from_git_history() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fixture::init(d);
        fixture::write(d, "src/a.rs", &complex_fn("small", 2));
        fixture::commit(d, "feat: start", "Ayşe Dev", "ayse@example.com", 60);
        fixture::write(d, "src/a.rs", &complex_fn("big", 24));
        fixture::commit(d, "feat: grow", "Ayşe Dev", "ayse@example.com", 2);
        let p = Project::gather(d);
        let debt = debt(d, &p, 10.0);
        assert!(debt.total_hours > 0.0);
        let g = debt.growth.expect("growth from git");
        assert!(g.per_month > 0.0, "{g:?}");
        assert!(matches!(g.basis, GrowthBasis::Git { .. }));
        // CC 25 → excess 15 → 3.75 h × churn 1.2 × no tests 1.5
        assert!(
            (debt.total_hours - 6.8).abs() < 0.05,
            "{}",
            debt.total_hours
        );
        assert!(debt.months_to_threshold.is_some_and(|m| m > 0.0));
    }
}
