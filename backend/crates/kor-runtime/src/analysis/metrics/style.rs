//! `kor style`: the developer's own style, measured on the lines `git blame` gives them (naming
//! per language and kind of name, error handling, guard clauses, function size, comment density,
//! preferred constructs). The profile goes into the project memory (`patterns.md`, section
//! `style`, through `kor_memory`'s section API, so a user edit is never overwritten) and as JSON
//! next to it for `kor style check`; [`prompt_section`] puts it in the agent's system prompt so
//! generated code follows it. `kor style check [--staged]` reports added code that departs from
//! the profile as findings.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};
use kor_memory::ProjectMemory;
use kor_memory::project::Doc;
use serde::{Deserialize, Serialize};

use super::code::{self, FileStats};
use super::git::{self, LogOpts};
use super::round1;
use crate::analysis::{Finding, Report, Severity};
use crate::review::{self, DiffSource};

/// Files analysed at most (the ones the author touched most).
pub const MAX_FILES: usize = 150;
/// Memory section name in `patterns.md`.
pub const SECTION: &str = "style";
const PROFILE_FILE: &str = "style.json";
/// A convention counts as the author's when at least this share of their names follow it.
pub const DOMINANT: f64 = 0.7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Convention {
    pub lang: String,
    /// `function`, `type`, `variable`, `constant`.
    pub category: String,
    pub dominant: String,
    /// 0–1.
    pub share: f64,
    pub samples: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleProfile {
    pub author: String,
    pub commits: usize,
    pub files: usize,
    /// Lines blamed to the author in the analysed files.
    pub lines: u32,
    pub functions: u32,
    pub languages: BTreeMap<String, u32>,
    pub naming: Vec<Convention>,
    /// Error-handling pattern → occurrences on the author's lines.
    pub errors: BTreeMap<String, u32>,
    /// Share of the author's functions with at least one guard-clause return, 0–1.
    pub early_return_share: f64,
    pub avg_function_lines: f64,
    pub avg_complexity: f64,
    /// Comment lines per code line, weighted by the author's share of each file.
    pub comment_density: f64,
    /// Documented share of the author's public functions, 0–1.
    pub doc_share: f64,
    /// Constructs weighted by the author's share of each file.
    pub constructs: BTreeMap<String, f64>,
    /// Derived, readable preferences (`` `?` over unwrap (12:1) ``).
    pub preferences: Vec<String>,
    pub source: String,
    pub taken_at: i64,
}

/// Files the author changed (most changes first), still present and parseable.
fn touched_files(root: &Path, author: &str) -> (usize, Vec<String>) {
    let commits = git::log(
        root,
        &LogOpts {
            author: Some(author),
            max: 500,
            ..LogOpts::default()
        },
    )
    .unwrap_or_default();
    let mut count: HashMap<String, u32> = HashMap::new();
    for c in &commits {
        for f in &c.files {
            *count.entry(f.path.clone()).or_default() += 1;
        }
    }
    let mut files: Vec<(String, u32)> = count
        .into_iter()
        .filter(|(p, _)| code::lang_of(p).is_some() && root.join(p).is_file())
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    files.truncate(MAX_FILES);
    (commits.len(), files.into_iter().map(|(p, _)| p).collect())
}

#[derive(Default)]
struct Acc {
    names: HashMap<(String, String), HashMap<String, u32>>,
    errors: BTreeMap<String, u32>,
    constructs: BTreeMap<String, f64>,
    langs: BTreeMap<String, u32>,
    functions: u32,
    early: u32,
    fn_lines: u64,
    complexity: u64,
    public: u32,
    documented: u32,
    code: f64,
    comments: f64,
    lines: u32,
}

impl Acc {
    #[allow(clippy::cast_precision_loss)]
    fn add(&mut self, s: &FileStats, mine: &HashSet<u32>) {
        if mine.is_empty() {
            return;
        }
        let share = mine.len() as f64 / f64::from(s.lines.max(1));
        let n = u32::try_from(mine.len()).unwrap_or(u32::MAX);
        self.lines += n;
        *self.langs.entry(s.lang.clone()).or_default() += n;
        for u in s.names.iter().filter(|u| mine.contains(&u.line)) {
            if let Some(c) = code::convention(&u.name) {
                *self
                    .names
                    .entry((s.lang.clone(), u.category.clone()))
                    .or_default()
                    .entry(c.to_owned())
                    .or_default() += 1;
            }
        }
        for (pattern, lines) in &s.errors {
            let k = u32::try_from(lines.iter().filter(|l| mine.contains(l)).count()).unwrap_or(0);
            if k > 0 {
                *self.errors.entry(pattern.clone()).or_default() += k;
            }
        }
        for (c, k) in &s.constructs {
            *self.constructs.entry(c.clone()).or_default() += f64::from(*k) * share;
        }
        for f in &s.functions {
            let owned = (f.line..=f.end).filter(|l| mine.contains(l)).count();
            if owned * 2 < f.lines() as usize {
                continue;
            }
            self.functions += 1;
            self.early += u32::from(f.early_returns > 0);
            self.fn_lines += u64::from(f.lines());
            self.complexity += u64::from(f.complexity);
            if f.public {
                self.public += 1;
                self.documented += u32::from(f.documented);
            }
        }
        self.code += f64::from(s.code_lines) * share;
        self.comments += f64::from(s.comment_lines) * share;
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(a: f64, b: f64) -> f64 {
    if b <= 0.0 { 0.0 } else { a / b }
}

fn preferences(p: &StyleProfile) -> Vec<String> {
    let mut out = Vec::new();
    let e = |k: &str| f64::from(p.errors.get(k).copied().unwrap_or(0));
    let c = |k: &str| p.constructs.get(k).copied().unwrap_or(0.0);
    let pair = |out: &mut Vec<String>, a: f64, b: f64, yes: &str, no: &str| {
        if a + b >= 5.0 {
            if a >= 2.0 * b {
                out.push(format!("{yes} over {no} ({a:.0}:{b:.0})"));
            } else if b >= 2.0 * a {
                out.push(format!("{no} over {yes} ({b:.0}:{a:.0})"));
            }
        }
    };
    pair(
        &mut out,
        e("?"),
        e("unwrap") + e("expect"),
        "`?`",
        "unwrap/expect",
    );
    pair(
        &mut out,
        c("const"),
        c("let") + c("var"),
        "`const`",
        "`let`/`var`",
    );
    pair(
        &mut out,
        c("iterator chain") + c("comprehension"),
        c("loop"),
        "iterator chains / comprehensions",
        "explicit loops",
    );
    pair(
        &mut out,
        c("arrow function"),
        c("function declaration"),
        "arrow functions",
        "function declarations",
    );
    pair(
        &mut out,
        c("match/switch"),
        c("if"),
        "match/switch",
        "if chains",
    );
    if p.functions >= 5 {
        out.push(format!(
            "guard clauses (early return) in {:.0}% of functions",
            p.early_return_share * 100.0
        ));
        out.push(format!(
            "functions average {:.0} lines, complexity {:.1}",
            p.avg_function_lines, p.avg_complexity
        ));
    }
    if e("bare except") == 0.0 && e("except") >= 3.0 {
        out.push("always names the exception type (no bare `except:`)".into());
    }
    if e("panic!") == 0.0 && e("?") >= 5.0 {
        out.push("no panics in library code".into());
    }
    out
}

/// Measure `author` (default: `git config user.email`) over the files they touched.
pub fn profile(root: &Path, author: Option<&str>) -> Result<StyleProfile, String> {
    if !git::is_repo(root) {
        return Err("not a git repository".into());
    }
    let author = match author {
        Some(a) => a.trim().to_lowercase(),
        None => git::user_email(root).ok_or("git user.email is not set; pass --author <email>")?,
    };
    let (commits, files) = touched_files(root, &author);
    if files.is_empty() {
        return Err(format!("no source files changed by {author}"));
    }
    let mut acc = Acc::default();
    let mut parser = tree_sitter::Parser::new();
    let mut analysed = 0;
    for rel in &files {
        let (Some(lang), Ok(text)) = (code::lang_of(rel), std::fs::read_to_string(root.join(rel)))
        else {
            continue;
        };
        let mine: HashSet<u32> = git::blame(root, rel)
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.email == author)
            .map(|b| b.line)
            .collect();
        if mine.is_empty() {
            continue;
        }
        analysed += 1;
        acc.add(&code::analyze_with(&mut parser, lang, rel, &text), &mine);
    }
    let mut naming: Vec<Convention> = acc
        .names
        .into_iter()
        .filter_map(|((lang, category), m)| {
            let samples: u32 = m.values().sum();
            let (dominant, n) = m
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
            Some(Convention {
                lang,
                category,
                dominant,
                share: round1(ratio(f64::from(n), f64::from(samples)) * 100.0) / 100.0,
                samples,
            })
        })
        .collect();
    naming.sort_by(|a, b| (&a.lang, &a.category).cmp(&(&b.lang, &b.category)));
    #[allow(clippy::cast_precision_loss)]
    let mut p = StyleProfile {
        author: author.clone(),
        commits,
        files: analysed,
        lines: acc.lines,
        functions: acc.functions,
        languages: acc.langs,
        naming,
        errors: acc.errors,
        early_return_share: round1(ratio(f64::from(acc.early), f64::from(acc.functions)) * 100.0)
            / 100.0,
        avg_function_lines: round1(ratio(acc.fn_lines as f64, f64::from(acc.functions))),
        avg_complexity: round1(ratio(acc.complexity as f64, f64::from(acc.functions))),
        comment_density: round1(ratio(acc.comments, acc.code) * 100.0) / 100.0,
        doc_share: round1(ratio(f64::from(acc.documented), f64::from(acc.public)) * 100.0) / 100.0,
        constructs: acc
            .constructs
            .into_iter()
            .map(|(k, v)| (k, round1(v)))
            .collect(),
        preferences: Vec::new(),
        source: format!(
            "git blame of {author}'s lines in {analysed} files ({commits} commits) + tree-sitter"
        ),
        taken_at: super::now_secs(),
    };
    p.preferences = preferences(&p);
    Ok(p)
}

/// The Markdown written to `patterns.md` (English: it is read by the model).
pub fn markdown(p: &StyleProfile) -> String {
    let mut s = String::from("## Coding style (measured by `kor style`)\n\n");
    let _ = writeln!(s, "Source: {}.\n", p.source);
    s.push_str("Naming:\n");
    for c in p.naming.iter().filter(|c| c.samples >= 3) {
        let _ = writeln!(
            s,
            "- {} {} names: {} ({:.0}% of {})",
            c.lang,
            c.category,
            c.dominant,
            c.share * 100.0,
            c.samples
        );
    }
    if !p.errors.is_empty() {
        let list: Vec<String> = p.errors.iter().map(|(k, v)| format!("{k} ×{v}")).collect();
        let _ = writeln!(s, "\nError handling: {}", list.join(", "));
    }
    if !p.preferences.is_empty() {
        s.push_str("\nPreferences:\n");
        for x in &p.preferences {
            let _ = writeln!(s, "- {x}");
        }
    }
    let _ = writeln!(
        s,
        "\nComments: {:.2} comment lines per code line; {:.0}% of public functions documented.",
        p.comment_density,
        p.doc_share * 100.0
    );
    s.push_str("\nWrite new code in this style unless the user asks otherwise.\n");
    s
}

fn memory(home: &Path, root: &Path) -> ProjectMemory {
    ProjectMemory::new(home, root)
}

/// Write the profile into `patterns.md` and `style.json`. `Ok(false)` when the user edited the
/// section in `patterns.md` (kept; the JSON is still updated).
pub fn save(home: &Path, root: &Path, p: &StyleProfile) -> std::io::Result<bool> {
    let mem = memory(home, root);
    mem.ensure()?;
    super::write_json(&mem.dir.join(PROFILE_FILE), p)?;
    mem.set_section(Doc::Patterns, SECTION, &markdown(p))
}

pub fn load(home: &Path, root: &Path) -> Option<StyleProfile> {
    let path: PathBuf = memory(home, root).dir.join(PROFILE_FILE);
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The `style` section of `patterns.md` (as the user may have edited it) for the system prompt;
/// empty when there is none.
pub fn prompt_section(home: &Path, root: &Path) -> String {
    let text = memory(home, root).text(Doc::Patterns);
    let Some(sec) = kor_memory::project::sections(&text)
        .into_iter()
        .find(|s| s.name == SECTION)
    else {
        return String::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let body = lines
        .get(sec.start..sec.end.saturating_sub(1))
        .map(|l| l.join("\n"))
        .unwrap_or_default();
    if body.trim().is_empty() {
        return String::new();
    }
    let body: String = body.chars().take(4_000).collect();
    format!("\n# Developer's coding style (from project memory)\n{body}\n")
}

/// The convention the author uses for `lang`/`category`, when it is clear enough.
fn expected<'p>(p: &'p StyleProfile, lang: &str, category: &str) -> Option<&'p Convention> {
    p.naming
        .iter()
        .find(|c| c.lang == lang && c.category == category && c.samples >= 5 && c.share >= DOMINANT)
}

/// Added code in the diff that departs from the stored profile.
pub fn check(home: &Path, root: &Path, src: &DiffSource, l: &Localizer) -> Result<Report, String> {
    let p = load(home, root).ok_or("no style profile yet: run `kor style` first")?;
    let diff = review::local_diff(root, src).map_err(|e| e.to_string())?;
    let mut added: BTreeMap<String, HashSet<u32>> = BTreeMap::new();
    for a in review::added_lines(&diff) {
        added
            .entry(a.file)
            .or_default()
            .insert(u32::try_from(a.line).unwrap_or(u32::MAX));
    }
    let mut report = Report::new("style");
    report.notes.push(format!("profile: {}", p.source));
    for (file, lines) in &added {
        let Some(lang) = code::lang_of(file) else {
            continue;
        };
        let text = match src {
            DiffSource::Staged => git::git(root, &["show", &format!(":{file}")]),
            DiffSource::Range(r) => {
                let rev = r
                    .rsplit("..")
                    .next()
                    .unwrap_or("HEAD")
                    .trim_start_matches('.');
                let rev = if rev.is_empty() { "HEAD" } else { rev };
                git::git(root, &["show", &format!("{rev}:{file}")])
            }
            DiffSource::Working => {
                std::fs::read_to_string(root.join(file)).map_err(|e| e.to_string())
            }
        };
        let Ok(text) = text else { continue };
        let s = code::analyze(lang, file, &text);
        report.findings.extend(file_findings(&p, &s, lines, l));
    }
    report.normalize();
    Ok(report)
}

fn file_findings(
    p: &StyleProfile,
    s: &FileStats,
    lines: &HashSet<u32>,
    tr: &Localizer,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mk = |rule: &str, line: u32, message: String| Finding {
        rule: rule.into(),
        source: "kor".into(),
        severity: Severity::Low,
        file: s.path.clone().into(),
        line,
        message,
        ..Finding::default()
    };
    for u in s.names.iter().filter(|u| lines.contains(&u.line)) {
        let (Some(conv), Some(exp)) =
            (code::convention(&u.name), expected(p, &s.lang, &u.category))
        else {
            continue;
        };
        if conv != exp.dominant {
            out.push(mk(
                "kor.style.naming",
                u.line,
                tr.t_args(
                    "metrics-f-style-naming",
                    &args!("name" => u.name.clone(), "conv" => conv, "lang" => s.lang.clone(), "category" => u.category.clone(),
                        "expected" => exp.dominant.clone(), "share" => format!("{:.0}", exp.share * 100.0), "samples" => exp.samples),
                ),
            ));
        }
    }
    let count = |k: &str| p.errors.get(k).copied().unwrap_or(0);
    let unwraps = count("unwrap") + count("expect");
    if count("?") >= 5 && count("?") >= 3 * unwraps.max(1) {
        for k in ["unwrap", "expect"] {
            for l in s
                .errors
                .get(k)
                .into_iter()
                .flatten()
                .filter(|l| lines.contains(l))
            {
                out.push(mk(
                    "kor.style.error-handling",
                    *l,
                    tr.t_args(
                        "metrics-f-style-unwrap",
                        &args!("call" => k, "q" => count("?"), "u" => unwraps),
                    ),
                ));
            }
        }
    }
    if count("bare except") == 0 && count("except") >= 3 {
        for l in s
            .errors
            .get("bare except")
            .into_iter()
            .flatten()
            .filter(|l| lines.contains(l))
        {
            out.push(mk(
                "kor.style.error-handling",
                *l,
                tr.t("metrics-f-style-bare-except"),
            ));
        }
    }
    let limit = (p.avg_complexity * 2.0).max(10.0);
    for f in &s.functions {
        if (f.line..=f.end).any(|l| lines.contains(&l)) && f64::from(f.complexity) > limit {
            let mut x = mk(
                "kor.style.complexity",
                f.line,
                tr.t_args(
                    "metrics-f-style-complexity",
                    &args!("name" => f.name.clone(), "cc" => f.complexity, "avg" => format!("{:.1}", p.avg_complexity)),
                ),
            );
            x.severity = Severity::Info;
            out.push(x);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::metrics::fixture;

    const MINE: &str = "pub fn load_user_name(id: u32) -> Result<String, String> {\n    if id == 0 {\n        return Err(\"zero\".into());\n    }\n    let user_name = fetch_row(id)?;\n    let first_line = parse_line(&user_name)?;\n    let trimmed_value = first_line.trim().to_owned();\n    Ok(trimmed_value)\n}\n\nfn fetch_row(id: u32) -> Result<String, String> {\n    let raw_value = read_raw(id)?;\n    Ok(raw_value)\n}\n\nfn parse_line(s: &str) -> Result<String, String> {\n    let line_text = s.lines().next().ok_or(\"empty\")?;\n    Ok(line_text.to_owned())\n}\n\nfn read_raw(id: u32) -> Result<String, String> {\n    let row_count = id + 1;\n    let as_text = row_count.to_string();\n    Ok(as_text)\n}\n\nfn write_out(v: u32) -> Result<u32, String> {\n    let out_value = read_raw(v)?.len();\n    Ok(u32::try_from(out_value).map_err(|e| e.to_string())?)\n}\n";

    #[test]
    fn profile_memory_prompt_and_check() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let d = dir.path();
        fixture::init(d);
        fixture::write(d, "src/mine.rs", MINE);
        fixture::write(
            d,
            "src/theirs.rs",
            "pub fn otherFn() { let someThing = 1; let x = Some(1).unwrap(); }\n",
        );
        fixture::commit(d, "feat: mine", "Ayşe Dev", "ayse@example.com", 5);
        // Someone else's file must not shape Ayşe's profile.
        fixture::write(
            d,
            "src/theirs.rs",
            "pub fn otherFn() { let someThing = 1; let anotherOne = 2; }\n",
        );
        fixture::commit(d, "feat: theirs", "Can Dev", "can@example.com", 4);

        let p = profile(d, None).unwrap();
        assert_eq!(p.author, "ayse@example.com");
        assert_eq!(p.files, 1);
        let vars = p
            .naming
            .iter()
            .find(|c| c.lang == "rust" && c.category == "variable")
            .unwrap();
        assert_eq!(vars.dominant, "snake_case");
        assert!((vars.share - 1.0).abs() < f64::EPSILON);
        assert_eq!(p.errors["?"], 6);
        assert!(
            p.preferences
                .iter()
                .any(|x| x.starts_with("`?` over unwrap"))
        );

        assert!(save(home.path(), d, &p).unwrap());
        let section = prompt_section(home.path(), d);
        assert!(
            section.contains("rust variable names: snake_case"),
            "{section}"
        );
        assert_eq!(load(home.path(), d).unwrap().author, p.author);

        fixture::write(
            d,
            "src/mine.rs",
            &format!(
                "{MINE}\nfn newThing() -> u32 {{\n    let someValue = \"1\".parse::<u32>().unwrap();\n    someValue\n}}\n"
            ),
        );
        let r = check(
            home.path(),
            d,
            &DiffSource::Working,
            &Localizer::new(kor_i18n::Lang::En),
        )
        .unwrap();
        let rules: Vec<(&str, u32)> = r
            .findings
            .iter()
            .map(|f| (f.rule.as_str(), f.line))
            .collect();
        assert!(rules.contains(&("kor.style.naming", 32)), "{rules:?}");
        assert!(rules.contains(&("kor.style.naming", 33)), "{rules:?}");
        assert!(
            rules.contains(&("kor.style.error-handling", 33)),
            "{rules:?}"
        );
    }
}
