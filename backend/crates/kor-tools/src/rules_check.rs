//! Rule compliance checker (Master Build Prompt §28.3, ADR 0063): the agent's changes against
//! the loaded rules (AGENTS.md, CLAUDE.md, `.kor/rules`, Cursor/Windsurf/… rules).
//!
//! Deterministic checks come from rules that name something machine-checkable:
//! - a negated rule with code spans ("never use `unwrap()`", "`console.log` kullanma",
//!   "n'utilisez jamais `var`") forbids those spans on added lines;
//! - "do not edit / never touch `path/**`" protects paths;
//! - an explicit `kor:forbid /regex/ [in <glob>]` (also inside an HTML comment) forbids a regex.
//!
//! Every other rule line is prose: it is "relevant" (yellow) when it applies to the changed files,
//! and the model judges it (`kor rules check --model`, the desktop's rule badge). Findings always
//! carry the rule's source file and line. Used before a diff is shown (the agent's review step)
//! and by `kor rules check`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Automatic retries when an edit breaks a rule, per task (§28.3: at most 2).
pub const MAX_RETRIES: u32 = 2;
const MAX_RULE_LINES: usize = 2_000;
const MAX_PATTERN: usize = 200;

/// One rule document as loaded (any tool's format, already without front matter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub source: PathBuf,
    pub text: String,
    /// `None`: always applies; `Some`: only for files matching these globs.
    pub globs: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
enum Check {
    /// Literal or regex that must not appear on added lines.
    Forbid { re: Regex, shown: String },
    /// Paths that must not be changed.
    Protect { set: GlobSet, shown: String },
}

/// One rule line.
#[derive(Debug, Clone)]
pub struct Rule {
    pub source: PathBuf,
    /// 1-based line in `source` (0 when the text was not found in the file).
    pub line: u32,
    pub text: String,
    globs: Option<GlobSet>,
    checks: Vec<Check>,
}

impl Rule {
    /// Whether the rule has a deterministic check.
    pub fn checkable(&self) -> bool {
        !self.checks.is_empty()
    }

    fn applies_to(&self, path: &str) -> bool {
        self.globs.as_ref().is_none_or(|g| g.is_match(path))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Green: checked and followed.
    Applied,
    /// Yellow: applies to the change but cannot be checked without judgment.
    Relevant,
    /// Red: broken.
    Violated,
}

/// One rule's verdict on a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub verdict: Verdict,
    pub rule: String,
    /// Rule file (as given) and 1-based line.
    pub source: String,
    pub line: u32,
    /// Changed file and line the finding is about.
    pub file: Option<String>,
    pub file_line: Option<u32>,
    pub message: String,
    /// `true` for deterministic checks, `false` for the model's judgment.
    pub deterministic: bool,
}

impl Finding {
    /// `CLAUDE.md:12`.
    pub fn location(&self) -> String {
        if self.line == 0 {
            self.source.clone()
        } else {
            format!("{}:{}", self.source, self.line)
        }
    }
}

/// A changed file: the content before and after (`None` = absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Words that turn a rule into a prohibition (en / tr / fr).
const NEGATIONS: &[&str] = &[
    "never",
    "don't",
    "do not",
    "dont",
    "avoid",
    "must not",
    "mustn't",
    "shall not",
    "no ",
    "forbid",
    "prohibit",
    "disallow",
    "not allowed",
    "without",
    "asla",
    "kullanma",
    "kullanmayın",
    "kullanmayin",
    "yapma",
    "yasak",
    "etme",
    "olmasın",
    "olmasin",
    "ekleme",
    "jamais",
    "n'utilise",
    "ne pas",
    "pas de",
    "évite",
    "evite",
    "interdit",
    "proscrit",
];
const PROTECT_VERBS: &[&str] = &[
    "edit",
    "modify",
    "change",
    "touch",
    "write to",
    "düzenleme",
    "değiştirme",
    "degistirme",
    "dokunma",
    "modifier",
    "toucher",
    "éditer",
];

/// Code spans of a line (`` `x` ``).
fn spans(line: &str) -> Vec<&str> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn looks_like_path(s: &str) -> bool {
    (s.contains('/') || s.contains('*') || s.starts_with('.'))
        && !s.contains(' ')
        && !s.contains('(')
        && s.len() <= 200
}

fn glob(pattern: &str) -> Option<GlobSet> {
    let mut b = GlobSetBuilder::new();
    let p = pattern.trim_start_matches("./");
    b.add(Glob::new(p).ok()?);
    // `docs/` and `docs` mean everything under it too.
    if !p.contains('*') {
        b.add(Glob::new(&format!("{}/**", p.trim_end_matches('/'))).ok()?);
    }
    b.build().ok()
}

fn regex(p: &str) -> Option<Regex> {
    RegexBuilder::new(p).size_limit(1 << 20).build().ok()
}

/// `kor:forbid /re/ [in glob]`.
fn directive(line: &str) -> Option<(Regex, String, Option<String>)> {
    let rest = line.split("kor:forbid").nth(1)?.trim_start();
    let rest = rest.strip_prefix('/')?;
    let end = rest.rfind('/')?;
    let (pat, tail) = (&rest[..end], &rest[end + 1..]);
    if pat.is_empty() || pat.len() > MAX_PATTERN {
        return None;
    }
    let scope = tail
        .trim()
        .trim_end_matches("-->")
        .trim()
        .strip_prefix("in ")
        .map(|g| g.trim().to_owned())
        .filter(|g| !g.is_empty());
    Some((regex(pat)?, format!("/{pat}/"), scope))
}

/// Deterministic checks a rule line names.
fn checks_of(line: &str) -> (Vec<Check>, Option<GlobSet>) {
    let mut out = Vec::new();
    let mut scope = None;
    if let Some((re, shown, glob_in)) = directive(line) {
        out.push(Check::Forbid { re, shown });
        scope = glob_in.as_deref().and_then(glob);
        return (out, scope);
    }
    let lower = line.to_lowercase();
    if !NEGATIONS.iter().any(|n| lower.contains(n)) {
        return (out, None);
    }
    let protect = PROTECT_VERBS.iter().any(|v| lower.contains(v));
    for s in spans(line) {
        if protect && looks_like_path(s) {
            if let Some(set) = glob(s) {
                out.push(Check::Protect {
                    set,
                    shown: s.to_owned(),
                });
            }
        } else if s.chars().count() >= 2 && s.len() <= MAX_PATTERN {
            // `unwrap()` also forbids `.unwrap()` calls with arguments removed: match the stem.
            let stem = s.strip_suffix("()").map_or(s, |x| x);
            let p = if s.ends_with("()") {
                format!("{}\\s*\\(", regex::escape(stem))
            } else {
                regex::escape(s)
            };
            if let Some(re) = regex(&p) {
                out.push(Check::Forbid {
                    re,
                    shown: s.to_owned(),
                });
            }
        }
    }
    (out, scope)
}

/// A rule line: a list item, a numbered item, or a sentence-like line (not a heading, not code).
fn rule_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence || t.is_empty() || t.starts_with('#') || t.starts_with("---") || t.starts_with('|')
        {
            continue;
        }
        let t = t
            .trim_start_matches(['-', '*', '+'])
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['.', ')'])
            .trim();
        if t.chars().count() >= 8 || t.contains("kor:forbid") {
            out.push(t.to_owned());
        }
        if out.len() >= MAX_RULE_LINES {
            break;
        }
    }
    out
}

/// Every rule line of the loaded documents, with deterministic checks where they name one.
#[derive(Debug, Default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    retries: AtomicU32,
    /// `<kor home>/rules-log.jsonl` (violations for the weekly report); `None` turns it off.
    log: Option<PathBuf>,
    log_lock: Mutex<()>,
}

impl RuleSet {
    pub fn new(docs: &[Doc], log: Option<PathBuf>) -> Self {
        let mut rules = Vec::new();
        for d in docs {
            let file = std::fs::read_to_string(&d.source).unwrap_or_default();
            let doc_globs = d.globs.as_ref().and_then(|g| {
                let mut b = GlobSetBuilder::new();
                for p in g {
                    b.add(Glob::new(p).ok()?);
                }
                b.build().ok()
            });
            for text in rule_lines(&d.text) {
                let line = file
                    .lines()
                    .position(|l| l.contains(&text))
                    .map_or(0, |i| u32::try_from(i + 1).unwrap_or(u32::MAX));
                let (checks, scope) = checks_of(&text);
                rules.push(Rule {
                    source: d.source.clone(),
                    line,
                    text,
                    globs: scope.or_else(|| doc_globs.clone()),
                    checks,
                });
            }
        }
        Self {
            rules,
            retries: AtomicU32::new(0),
            log,
            log_lock: Mutex::new(()),
        }
    }

    /// A new task: retries start again.
    pub fn reset(&self) {
        self.retries.store(0, Ordering::Relaxed);
    }

    /// Use one automatic retry; `false` once [`MAX_RETRIES`] are used.
    pub fn take_retry(&self) -> bool {
        self.retries.fetch_add(1, Ordering::Relaxed) < MAX_RETRIES
    }

    /// Check `changes`: violations (red), followed checks (green) and prose rules that apply
    /// to the changed files (yellow). `display` turns a rule source into what is shown.
    pub fn check(&self, changes: &[Change], display: &dyn Fn(&Path) -> String) -> Vec<Finding> {
        let mut out = Vec::new();
        for r in &self.rules {
            let touched: Vec<&Change> = changes.iter().filter(|c| r.applies_to(&c.path)).collect();
            if touched.is_empty() {
                continue;
            }
            let base = |verdict, file: Option<String>, file_line, message: String| Finding {
                verdict,
                rule: r.text.clone(),
                source: display(&r.source),
                line: r.line,
                file,
                file_line,
                message,
                deterministic: r.checkable(),
            };
            if !r.checkable() {
                out.push(base(Verdict::Relevant, None, None, String::new()));
                continue;
            }
            let mut broken = false;
            for c in &touched {
                for ch in &r.checks {
                    match ch {
                        Check::Protect { set, shown } if set.is_match(&c.path) => {
                            broken = true;
                            out.push(base(
                                Verdict::Violated,
                                Some(c.path.clone()),
                                None,
                                format!("`{}` is protected ({shown})", c.path),
                            ));
                        }
                        Check::Forbid { re, shown } => {
                            for (n, l) in added_lines(c) {
                                if re.is_match(l) {
                                    broken = true;
                                    out.push(base(
                                        Verdict::Violated,
                                        Some(c.path.clone()),
                                        Some(n),
                                        format!("`{shown}` on an added line"),
                                    ));
                                    break;
                                }
                            }
                        }
                        Check::Protect { .. } => {}
                    }
                }
            }
            if !broken {
                out.push(base(Verdict::Applied, None, None, String::new()));
            }
        }
        out.sort_by(|a, b| b.verdict.cmp(&a.verdict));
        out
    }

    /// [`Self::check`] on per-file review proposals.
    pub fn check_proposals(
        &self,
        files: &[crate::Proposal],
        display: &dyn Fn(&Path) -> String,
    ) -> Vec<Finding> {
        let changes: Vec<Change> = files
            .iter()
            .map(|p| Change {
                path: p.path.replace('\\', "/"),
                before: p.before.clone(),
                after: p.after.clone(),
            })
            .collect();
        self.check(&changes, display)
    }

    /// Append violations to the rules log (the weekly "most broken rules" report).
    pub fn log(&self, model: &str, findings: &[Finding]) {
        let Some(path) = &self.log else { return };
        let _g = self
            .log_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut text = String::new();
        for f in findings.iter().filter(|f| f.verdict == Verdict::Violated) {
            let e = LogEntry {
                at,
                model: model.to_owned(),
                source: f.location(),
                rule: f.rule.clone(),
                deterministic: f.deterministic,
            };
            if let Ok(l) = serde_json::to_string(&e) {
                text.push_str(&l);
                text.push('\n');
            }
        }
        if text.is_empty() {
            return;
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = std::io::Write::write_all(&mut f, text.as_bytes());
        }
    }
}

/// One violation in the rules log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: u64,
    pub model: String,
    pub source: String,
    pub rule: String,
    pub deterministic: bool,
}

/// Lines of `after` that are not in `before`, with their 1-based numbers.
fn added_lines(c: &Change) -> Vec<(u32, &str)> {
    let Some(after) = c.after.as_deref() else {
        return Vec::new();
    };
    let before: std::collections::HashSet<&str> =
        c.before.as_deref().unwrap_or_default().lines().collect();
    after
        .lines()
        .enumerate()
        .filter(|(_, l)| !before.contains(l))
        .map(|(i, l)| (u32::try_from(i + 1).unwrap_or(u32::MAX), l))
        .collect()
}

/// What the model is told when its edit broke rules (the automatic retry).
pub fn retry_message(violated: &[&Finding]) -> String {
    let mut s = String::from(
        "This edit was not applied: it breaks the project's rules. Redo it so that it follows them:\n",
    );
    for f in violated {
        let _ = writeln!(
            s,
            "- {} ({}){}: {}",
            f.rule,
            f.location(),
            f.file
                .as_ref()
                .map(|p| format!(
                    " in {p}{}",
                    f.file_line.map(|n| format!(":{n}")).unwrap_or_default()
                ))
                .unwrap_or_default(),
            f.message
        );
    }
    s
}

/// Violations per (rule location, model) since `since` (Unix seconds), most first.
pub fn report(log: &Path, since: u64) -> Vec<(String, String, String, usize)> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut m: std::collections::BTreeMap<(String, String), (String, usize)> =
        std::collections::BTreeMap::new();
    for e in text
        .lines()
        .filter_map(|l| serde_json::from_str::<LogEntry>(l).ok())
        .filter(|e| e.at >= since)
    {
        let v = m.entry((e.source, e.model)).or_insert((e.rule, 0));
        v.1 += 1;
    }
    let mut out: Vec<_> = m
        .into_iter()
        .map(|((src, model), (rule, n))| (src, rule, model, n))
        .collect();
    out.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str, globs: Option<Vec<String>>) -> (tempfile::TempDir, RuleSet) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("CLAUDE.md");
        std::fs::write(&p, text).unwrap();
        let rs = RuleSet::new(
            &[Doc {
                source: p,
                text: text.into(),
                globs,
            }],
            Some(d.path().join("log.jsonl")),
        );
        (d, rs)
    }

    fn change(path: &str, before: &str, after: &str) -> Change {
        Change {
            path: path.into(),
            before: Some(before.into()),
            after: Some(after.into()),
        }
    }

    fn name(p: &Path) -> String {
        p.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn negated_code_spans_paths_and_directives() {
        let text = "# Rules\n\n- Never use `unwrap()` in library code.\n- Prefer small functions and clear names.\n- Do not edit `migrations/` by hand.\n- `console.log` kullanma.\n<!-- kor:forbid /TODO\\(\\w+\\)/ in *.rs -->\n";
        let (dir, rs) = set(text, None);
        assert_eq!(rs.rules.len(), 5);
        assert_eq!(rs.rules.iter().filter(|r| r.checkable()).count(), 4);
        let f = rs.check(
            &[
                change(
                    "src/lib.rs",
                    "fn a() {}\n",
                    "fn a() {}\nfn b() { x.unwrap(); }\n// TODO(me)\n",
                ),
                change("migrations/1.sql", "", "DROP TABLE x;"),
                change("web/app.js", "a();\n", "a();\nconsole.log(1);\n"),
            ],
            &name,
        );
        let red: Vec<&Finding> = f
            .iter()
            .filter(|x| x.verdict == Verdict::Violated)
            .collect();
        assert_eq!(red.len(), 4, "{f:#?}");
        let unwrap = red.iter().find(|x| x.rule.contains("unwrap")).unwrap();
        assert_eq!((unwrap.source.as_str(), unwrap.line), ("CLAUDE.md", 3));
        assert_eq!(
            (unwrap.file.as_deref(), unwrap.file_line),
            (Some("src/lib.rs"), Some(2))
        );
        assert!(
            red.iter()
                .any(|x| x.file.as_deref() == Some("migrations/1.sql"))
        );
        assert!(
            red.iter()
                .any(|x| x.rule.contains("kor:forbid") && x.file_line == Some(3))
        );
        assert!(
            f.iter()
                .any(|x| x.verdict == Verdict::Relevant && x.rule.starts_with("Prefer small"))
        );
        assert_eq!(f[0].verdict, Verdict::Violated, "violations first");
        // Lines already present before the change are not the agent's.
        let f = rs.check(
            &[change(
                "src/lib.rs",
                "x.unwrap();\n",
                "x.unwrap();\nok();\n",
            )],
            &name,
        );
        assert!(
            f.iter()
                .any(|x| x.rule.contains("unwrap") && x.verdict == Verdict::Applied)
        );
        rs.log("m/x", &red.into_iter().cloned().collect::<Vec<_>>());
        let rep = report(&dir.path().join("log.jsonl"), 0);
        assert_eq!(rep.len(), 4);
        assert_eq!(rep[0].2, "m/x");
    }

    #[test]
    fn scoped_rules_and_retries() {
        let (_d, rs) = set(
            "- Never use `any` in TypeScript.\n",
            Some(vec!["*.ts".into()]),
        );
        assert!(
            rs.check(&[change("a.rs", "", "let any = 1;")], &name)
                .is_empty()
        );
        assert_eq!(
            rs.check(&[change("a.ts", "", "let x: any = 1;")], &name)[0].verdict,
            Verdict::Violated
        );
        assert!(rs.take_retry() && rs.take_retry() && !rs.take_retry());
        rs.reset();
        assert!(rs.take_retry());
        let msg = retry_message(&[&rs.check(&[change("a.ts", "", "let x: any")], &name)[0]]);
        assert!(msg.contains("CLAUDE.md:1") && msg.contains("a.ts:1"));
    }

    #[test]
    fn prose_only_and_code_fences_are_skipped() {
        let (_d, rs) = set("```\nnever `x`\n```\nKeep functions short.\n", None);
        assert_eq!(rs.rules.len(), 1);
        assert!(!rs.rules[0].checkable());
    }
}
