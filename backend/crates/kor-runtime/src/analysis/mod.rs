//! Shared analysis contract for `kor audit --security|--compliance|--licenses`, `kor deps`,
//! `kor quality` and friends (spec §9, ADR 0045).
//!
//! Deterministic scanners produce [`Finding`]s; the model only explains them. Every finding
//! carries a rule id, the scanner that produced it (`source`), a project-relative file and a
//! 1-based line (§9.1: no number without a source). [`render`] turns a [`Report`] into Markdown
//! (localized), JSON or SARIF 2.1.0 so CI tools can read it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod compliance;
pub mod explain;
pub mod deps;
pub mod metrics;
mod render;
pub mod rules;
pub mod security;
pub mod sign;
pub mod walk;

pub use render::{Format, check_sarif, render};
pub use sign::{SignOutcome, VerifyOutcome, sign_and_link, verify_report};

/// How bad a finding is. Ordered from worst to mildest, so sorting ascending puts critical first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    #[default]
    Info,
}

impl Severity {
    pub const ALL: [Self; 5] = [
        Self::Critical,
        Self::High,
        Self::Medium,
        Self::Low,
        Self::Info,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    /// `critical|high|medium|low|info` (also `error`/`warning`/`note` from SARIF-style tools).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "critical" | "blocker" => Self::Critical,
            "high" | "error" => Self::High,
            "medium" | "moderate" | "warning" => Self::Medium,
            "low" | "note" | "minor" => Self::Low,
            "info" | "informational" | "none" => Self::Info,
            _ => return None,
        })
    }
}

/// One deterministic result: rule + source + `file:line`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Finding {
    /// Rule id (`kor.sql-injection`, `aws-access-token`, a semgrep check id, an SPDX id, …).
    pub rule: String,
    /// Producer: `kor`, `gitleaks`, `semgrep`, `osv`, `spdx`, `git`, …
    pub source: String,
    pub severity: Severity,
    /// Project-relative path, `/`-separated where possible.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// `CWE-89`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwe: Option<String>,
}

impl Finding {
    /// The dedup key used when several scanners report the same spot.
    pub fn key(&self) -> (PathBuf, u32, String) {
        (self.file.clone(), self.line, self.rule.clone())
    }

    /// `file:line` with `/` separators.
    pub fn location(&self) -> String {
        format!("{}:{}", slash(&self.file), self.line)
    }
}

/// The output of one analysis run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Report {
    /// `security`, `compliance-kvkk`, `licenses`, `deps`, …
    pub tool: String,
    pub findings: Vec<Finding>,
    /// Scanner notes: what ran, what was skipped and why.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Report {
    pub fn new(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            ..Self::default()
        }
    }

    /// Worst first, then by file and line; one finding per `file:line:rule` (the first source
    /// to report it wins, so call this after pushing Kor's own findings first).
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.findings.retain(|f| seen.insert(f.key()));
        self.findings.sort_by(|a, b| {
            (a.severity, &a.file, a.line, &a.rule).cmp(&(b.severity, &b.file, b.line, &b.rule))
        });
    }

    /// Count per severity, in [`Severity::ALL`] order.
    pub fn counts(&self) -> [usize; 5] {
        let mut c = [0; 5];
        for f in &self.findings {
            c[f.severity as usize] += 1;
        }
        c
    }

    /// Keep findings at `min` or worse.
    pub fn retain_min(&mut self, min: Severity) {
        self.findings.retain(|f| f.severity <= min);
    }
}

pub(crate) fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(rule: &str, sev: Severity, file: &str, line: u32) -> Finding {
        Finding {
            rule: rule.into(),
            source: "kor".into(),
            severity: sev,
            file: file.into(),
            line,
            message: "m".into(),
            ..Finding::default()
        }
    }

    #[test]
    fn normalize_dedups_and_sorts_worst_first() {
        let mut r = Report::new("security");
        r.findings = vec![
            f("a", Severity::Low, "x.rs", 3),
            f("b", Severity::Critical, "y.rs", 1),
            f("a", Severity::Low, "x.rs", 3),
        ];
        r.findings[2].source = "semgrep".into();
        r.normalize();
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.findings[0].rule, "b");
        assert_eq!(r.findings[1].source, "kor");
        assert_eq!(r.counts(), [1, 0, 0, 1, 0]);
    }

    #[test]
    fn severity_parses_sarif_levels() {
        assert_eq!(Severity::parse("error"), Some(Severity::High));
        assert_eq!(Severity::parse("WARNING"), Some(Severity::Medium));
        assert_eq!(Severity::parse("bogus"), None);
        for s in Severity::ALL {
            assert_eq!(Severity::parse(s.as_str()), Some(s));
        }
    }
}
