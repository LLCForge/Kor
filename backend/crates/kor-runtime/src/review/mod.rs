//! `kor review` (§8.8, ADR 0040): review a diff through a persona. Personas are plain Markdown
//! prompts: the shipped ones (`personas/*.md`), then `~/.kor/review/personas/*.md` and the
//! project's `.kor/review/personas/*.md` (a plugin adds one by dropping a file there; a later
//! layer replaces a persona of the same name). Cheap deterministic checks (added secrets, very
//! large added functions) run first; the model reviews the rest.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use kor_agent::gitflow::{self, FlowError};
use kor_i18n::{Lang, Localizer, args};
use kor_providers::Provider;
use serde::{Deserialize, Serialize};

use crate::forge::InlineNote;

/// Diff characters given to the model.
const MAX_DIFF: usize = 60_000;
/// An added function longer than this is reported.
const LARGE_FUNCTION: usize = 80;
/// Persona files larger than this are skipped.
const MAX_PERSONA: u64 = 32 * 1024;

const BUILTIN: &[(&str, &str)] = &[
    ("general", include_str!("personas/general.md")),
    ("security", include_str!("personas/security.md")),
    ("performance", include_str!("personas/performance.md")),
    ("rust-idiomatic", include_str!("personas/rust-idiomatic.md")),
    ("junior-friendly", include_str!("personas/junior-friendly.md")),
];

/// The persona used without `--persona`.
pub const DEFAULT_PERSONA: &str = "general";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Persona {
    pub name: String,
    pub prompt: String,
    /// `builtin`, `user` or `project`.
    pub source: &'static str,
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn read_dir(dir: &Path, source: &'static str, out: &mut Vec<Persona>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    for p in files {
        let Some(name) = p
            .extension()
            .filter(|e| e.eq_ignore_ascii_case("md"))
            .and(p.file_stem())
            .and_then(|s| s.to_str())
            .filter(|n| valid_name(n))
        else {
            continue;
        };
        if std::fs::metadata(&p).map_or(true, |m| !m.is_file() || m.len() > MAX_PERSONA) {
            continue;
        }
        if let Ok(prompt) = std::fs::read_to_string(&p) {
            out.retain(|x| x.name != name);
            out.push(Persona {
                name: name.to_owned(),
                prompt,
                source,
            });
        }
    }
}

/// Every persona visible from `root`, later layers replacing earlier ones by name.
pub fn personas(root: &Path) -> Vec<Persona> {
    let mut out: Vec<Persona> = BUILTIN
        .iter()
        .map(|(n, p)| Persona {
            name: (*n).to_owned(),
            prompt: (*p).to_owned(),
            source: "builtin",
        })
        .collect();
    if let Some(home) = crate::kor_home() {
        read_dir(&home.join("review").join("personas"), "user", &mut out);
    }
    read_dir(&root.join(".kor").join("review").join("personas"), "project", &mut out);
    out
}

pub fn persona(root: &Path, name: &str) -> Option<Persona> {
    personas(root).into_iter().find(|p| p.name == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "critical" | "blocker" => Self::Critical,
            "high" | "error" | "major" => Self::High,
            "medium" | "warning" | "warn" => Self::Medium,
            "low" | "minor" => Self::Low,
            _ => Self::Info,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: Option<u64>,
    pub severity: Severity,
    pub rule: String,
    pub message: String,
    /// `check` (deterministic) or `model`.
    pub source: &'static str,
}

/// What to review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// Uncommitted changes against HEAD.
    Working,
    Staged,
    /// `A..B` (or `A...B`).
    Range(String),
}

/// The local diff for `src`.
///
/// # Errors
/// git failures; a range starting with `-` is refused.
pub fn local_diff(root: &Path, src: &DiffSource) -> Result<String, FlowError> {
    let base = ["diff", "--no-color", "--no-ext-diff"];
    let out = match src {
        DiffSource::Working => gitflow::git(root, &[&base[..], &["HEAD", "--"]].concat())?,
        DiffSource::Staged => gitflow::git(root, &[&base[..], &["--cached", "--"]].concat())?,
        DiffSource::Range(r) => {
            if r.starts_with('-') || r.trim().is_empty() {
                return Err(FlowError::Git("invalid range".into()));
            }
            gitflow::git(root, &[&base[..], &[r.as_str(), "--"]].concat())?
        }
    };
    Ok(out)
}

/// One added line of a unified diff with its new-file line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub file: String,
    pub line: u64,
    pub text: String,
}

/// Added lines of a unified diff (deleted files contribute none).
pub fn added_lines(diff: &str) -> Vec<Added> {
    let mut out = Vec::new();
    let mut file: Option<String> = None;
    let mut line = 0u64;
    for l in diff.lines() {
        if let Some(p) = l.strip_prefix("+++ ") {
            file = (p != "/dev/null").then(|| p.strip_prefix("b/").unwrap_or(p).to_owned());
        } else if let Some(h) = l.strip_prefix("@@ ") {
            // `@@ -a,b +c,d @@`
            line = h
                .split_whitespace()
                .find_map(|t| t.strip_prefix('+'))
                .and_then(|t| t.split(',').next())
                .and_then(|t| t.parse().ok())
                .unwrap_or(1);
        } else if l.starts_with("diff --git") {
            file = None;
        } else if let (Some(f), Some(t)) = (&file, l.strip_prefix('+')) {
            out.push(Added {
                file: f.clone(),
                line,
                text: t.to_owned(),
            });
            line += 1;
        } else if l.starts_with(' ') || l.is_empty() {
            line += 1;
        }
    }
    out
}

fn is_fn_start(t: &str) -> bool {
    let mut t = t.trim_start();
    for p in [
        "pub(crate) ", "pub(super) ", "pub ", "export ", "default ", "async ", "const ",
        "unsafe ", "static ", "extern \"C\" ",
    ] {
        while let Some(r) = t.strip_prefix(p) {
            t = r.trim_start();
        }
    }
    ["fn ", "def ", "func ", "function "]
        .iter()
        .any(|k| t.starts_with(k))
}

/// Deterministic findings: added secrets and very large added functions.
pub fn prechecks(l: &Localizer, diff: &str) -> Vec<Finding> {
    let added = added_lines(diff);
    let mut out = Vec::new();
    for a in &added {
        for rule in kor_tools::scan_secrets(&a.text) {
            out.push(Finding {
                file: a.file.clone(),
                line: Some(a.line),
                severity: Severity::Critical,
                rule: format!("secret/{rule}"),
                message: l.t_args("review-check-secret", &args!("kind" => rule)),
                source: "check",
            });
        }
    }
    // A function counts from its first added line to the next function, a gap, or another file.
    let mut i = 0;
    while i < added.len() {
        if !is_fn_start(&added[i].text) {
            i += 1;
            continue;
        }
        let start = &added[i];
        let mut j = i + 1;
        while j < added.len()
            && added[j].file == start.file
            && added[j].line == added[j - 1].line + 1
            && !is_fn_start(&added[j].text)
        {
            j += 1;
        }
        let len = j - i;
        if len > LARGE_FUNCTION {
            out.push(Finding {
                file: start.file.clone(),
                line: Some(start.line),
                severity: Severity::Low,
                rule: "large-function".into(),
                message: l.t_args("review-check-large", &args!("lines" => len)),
                source: "check",
            });
        }
        i = j;
    }
    out
}

fn language(l: &Localizer) -> &'static str {
    match l.lang() {
        Lang::Tr => "Turkish",
        Lang::Fr => "French",
        Lang::En => "English",
    }
}

pub fn review_prompt(l: &Localizer, persona: &Persona, diff: &str, pre: &[Finding]) -> String {
    let mut diff = kor_tools::redact_secrets(diff);
    if let Some((i, _)) = diff.char_indices().nth(MAX_DIFF) {
        diff.truncate(i);
        diff.push_str("\n[diff truncated]\n");
    }
    let known = pre.iter().fold(String::new(), |mut s, f| {
        let _ = writeln!(s, "- {}:{} {}", f.file, f.line.unwrap_or(0), f.rule);
        s
    });
    format!(
        "{}\n\nReview this diff. Report only real issues in added or changed lines. Reply with \
         JSON only: [{{\"file\": \"path\", \"line\": <new-file line number or null>, \
         \"severity\": \"info|low|medium|high|critical\", \"rule\": \"short-kebab-id\", \
         \"message\": \"what is wrong and how to fix it\"}}]. `[]` when nothing is wrong. \
         Write messages in {}.\nAlready reported by automatic checks (do not repeat):\n{}\nDiff:\n{diff}",
        persona.prompt.trim(),
        language(l),
        if known.is_empty() { "- none\n".into() } else { known },
    )
}

/// Findings from the model's JSON reply.
///
/// # Errors
/// When the reply holds no JSON array.
pub fn parse_findings(reply: &str) -> Result<Vec<Finding>, String> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        file: String,
        #[serde(default)]
        line: Option<serde_json::Value>,
        #[serde(default)]
        severity: String,
        #[serde(default)]
        rule: String,
        #[serde(default)]
        message: String,
    }
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else {
        return Err("no JSON array".into());
    };
    let raw: Vec<Raw> =
        serde_json::from_str(reply.get(a..=b).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(raw
        .into_iter()
        .filter(|r| !r.message.trim().is_empty())
        .map(|r| Finding {
            file: r.file.trim().trim_start_matches("b/").to_owned(),
            line: r.line.and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            }),
            severity: Severity::parse(&r.severity),
            rule: if r.rule.trim().is_empty() {
                "review".into()
            } else {
                r.rule.trim().to_owned()
            },
            message: r.message.trim().to_owned(),
            source: "model",
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub persona: String,
    pub findings: Vec<Finding>,
    /// The model request failed or was skipped; only deterministic findings are present.
    pub model_error: Option<String>,
}

/// Review `diff` as `persona`: deterministic checks, then the model when one is given.
pub async fn review(
    l: &Localizer,
    model: Option<(&Arc<dyn Provider>, &str)>,
    persona: &Persona,
    diff: &str,
) -> Report {
    let mut findings = prechecks(l, diff);
    let model_error = match model {
        None => Some(l.t("review-no-model")),
        Some(_) if diff.trim().is_empty() => None,
        Some((p, m)) => {
            let prompt = review_prompt(l, persona, diff, &findings);
            match gitflow::ask(p, m, prompt, 4096).await {
                Ok(reply) => match parse_findings(&reply) {
                    Ok(f) => {
                        findings.extend(f);
                        None
                    }
                    Err(e) => Some(e),
                },
                Err(e) => Some(e.to_string()),
            }
        }
    };
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    Report {
        persona: persona.name.clone(),
        findings,
        model_error,
    }
}

fn location(f: &Finding) -> String {
    match f.line {
        Some(n) => format!("{}:{n}", f.file),
        None => f.file.clone(),
    }
}

/// The report as Markdown.
pub fn markdown(l: &Localizer, r: &Report) -> String {
    let mut s = format!(
        "## {}\n\n",
        l.t_args("review-title", &args!("persona" => r.persona.clone()))
    );
    if r.findings.is_empty() {
        s.push_str(&l.t("review-clean"));
        s.push('\n');
    }
    for f in &r.findings {
        let _ = writeln!(
            s,
            "- **{}** `{}` ({}) {}",
            f.severity.as_str(),
            location(f),
            f.rule,
            f.message.replace('\n', " ")
        );
    }
    if let Some(e) = &r.model_error {
        let _ = write!(s, "\n> {}\n", l.t_args("review-model-skipped", &args!("error" => e.clone())));
    }
    s
}

/// A report as a PR review: findings with a line become inline notes, the rest the summary.
pub fn to_notes(l: &Localizer, r: &Report) -> (String, Vec<InlineNote>) {
    let notes: Vec<InlineNote> = r
        .findings
        .iter()
        .filter_map(|f| {
            Some(InlineNote {
                path: f.file.clone(),
                line: f.line?,
                body: format!("**{}** ({}) {}", f.severity.as_str(), f.rule, f.message),
            })
        })
        .collect();
    let rest = Report {
        persona: r.persona.clone(),
        findings: r.findings.iter().filter(|f| f.line.is_none()).cloned().collect(),
        model_error: None,
    };
    let mut body = markdown(l, &rest);
    if rest.findings.is_empty() && !notes.is_empty() {
        body = format!(
            "## {}\n\n{}\n",
            l.t_args("review-title", &args!("persona" => r.persona.clone())),
            l.t_args("review-inline-count", &args!("count" => notes.len()))
        );
    }
    (body, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l() -> Localizer {
        Localizer::new(Lang::En)
    }

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +10,3 @@\n context\n+let k = \"AKIAIOSFODNN7EXAMPLE\";\n-old\n+new\n";

    #[test]
    fn added_lines_track_new_line_numbers() {
        let a = added_lines(DIFF);
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].file.as_str(), a[0].line), ("src/a.rs", 11));
        assert_eq!(a[1].line, 12);
    }

    #[test]
    fn prechecks_find_secrets_and_large_functions() {
        let f = prechecks(&l(), DIFF);
        assert!(f.iter().any(|f| f.rule.starts_with("secret/") && f.line == Some(11)), "{f:?}");
        let mut big = String::from("+++ b/x.py\n@@ -0,0 +1,100 @@\n+def big():\n");
        for _ in 0..90 {
            big.push_str("+    x = 1\n");
        }
        big.push_str("+def small():\n+    pass\n");
        let f = prechecks(&l(), &big);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!((f[0].rule.as_str(), f[0].line), ("large-function", Some(1)));
    }

    #[test]
    fn findings_parse_leniently() {
        let r = "```json\n[{\"file\":\"b/a.rs\",\"line\":\"3\",\"severity\":\"warning\",\"message\":\"m\"},{\"message\":\"\"}]\n```";
        let f = parse_findings(r).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].file.as_str(), f[0].line, f[0].severity), ("a.rs", Some(3), Severity::Medium));
        assert_eq!(f[0].rule, "review");
        assert!(parse_findings("nothing").is_err());
    }

    #[test]
    fn personas_layer_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".kor/review/personas");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("security.md"), "custom").unwrap();
        std::fs::write(p.join("a11y.md"), "accessibility").unwrap();
        std::fs::write(p.join("bad name.md"), "x").unwrap();
        let all = personas(dir.path());
        assert_eq!(persona(dir.path(), "security").unwrap().prompt, "custom");
        assert_eq!(persona(dir.path(), "a11y").unwrap().source, "project");
        assert!(all.iter().any(|p| p.name == "rust-idiomatic" && p.source == "builtin"));
        assert!(!all.iter().any(|p| p.name.contains(' ')));
    }

    #[test]
    fn notes_split_inline_and_summary() {
        let r = Report {
            persona: "security".into(),
            findings: vec![
                Finding { file: "a.rs".into(), line: Some(3), severity: Severity::High, rule: "x".into(), message: "m".into(), source: "model" },
                Finding { file: "b.rs".into(), line: None, severity: Severity::Low, rule: "y".into(), message: "n".into(), source: "model" },
            ],
            model_error: None,
        };
        let (body, notes) = to_notes(&l(), &r);
        assert_eq!(notes.len(), 1);
        assert!(body.contains("b.rs") && !body.contains("a.rs:3"), "{body}");
        assert!(markdown(&l(), &r).contains("`a.rs:3`"));
    }
}
