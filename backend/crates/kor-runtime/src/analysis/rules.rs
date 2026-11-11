//! Line rules as data (ADR 0045): built-in `data/security.toml`, extended or overridden by
//! `<project>/.kor/audit/rules/*.toml` (same id replaces the built-in; `enabled = false` turns it
//! off). One rule = one regex per line, with an optional negative regex checked on the
//! surrounding lines (`unless` + `near`) and a file-level precondition (`requires`).

use std::collections::BTreeMap;
use std::path::Path;

use kor_i18n::Lang;
use regex::Regex;
use serde::Deserialize;

use super::Severity;
use super::walk::Source;

pub const BUILTIN: &str = include_str!("data/security.toml");
pub const USER_DIR: &str = ".kor/audit/rules";
/// Lines longer than this (minified bundles) are not matched.
pub const MAX_LINE: usize = 2_000;

/// A message in one language or per language (`{ en = "…", tr = "…" }`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Text {
    One(String),
    Many(BTreeMap<String, String>),
}

impl Text {
    pub fn get(&self, lang: Lang) -> String {
        match self {
            Self::One(s) => s.clone(),
            Self::Many(m) => m
                .get(lang.code())
                .or_else(|| m.get("en"))
                .or_else(|| m.values().next())
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDef {
    pub id: String,
    pub severity: Severity,
    #[serde(default)]
    pub cwe: Option<String>,
    /// File extensions (lowercase, no dot); empty = every text file.
    #[serde(default)]
    pub ext: Vec<String>,
    pub pattern: String,
    /// No finding when this matches the line or one of the `near` lines around it.
    #[serde(default)]
    pub unless: Option<String>,
    #[serde(default)]
    pub near: usize,
    /// Only files whose whole text matches this.
    #[serde(default)]
    pub requires: Option<String>,
    pub rationale: Text,
    #[serde(default)]
    pub fix: Option<Text>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<RuleDef>,
}

/// A rule with its regexes compiled.
#[derive(Debug, Clone)]
pub struct Rule {
    pub def: RuleDef,
    pattern: Regex,
    unless: Option<Regex>,
    requires: Option<Regex>,
}

/// One match of a rule: 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub line: u32,
    pub column: u32,
}

impl Rule {
    pub fn compile(def: RuleDef) -> Result<Self, String> {
        let re = |s: &str| Regex::new(s).map_err(|e| format!("{}: {e}", def.id));
        Ok(Self {
            pattern: re(&def.pattern)?,
            unless: def.unless.as_deref().map(re).transpose()?,
            requires: def.requires.as_deref().map(re).transpose()?,
            def,
        })
    }

    pub fn applies_to(&self, src: &Source) -> bool {
        (self.def.ext.is_empty() || self.def.ext.contains(&src.ext()))
            && self.requires.as_ref().is_none_or(|r| r.is_match(&src.text))
    }

    /// Matches in `src`, skipping comment-only lines.
    pub fn hits(&self, src: &Source) -> Vec<Hit> {
        if !self.applies_to(src) {
            return Vec::new();
        }
        let lines: Vec<&str> = src.text.lines().collect();
        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.len() > MAX_LINE || is_comment(line) {
                continue;
            }
            let Some(m) = self.pattern.find(line) else {
                continue;
            };
            if let Some(u) = &self.unless {
                let lo = i.saturating_sub(self.def.near);
                let hi = (i + self.def.near).min(lines.len() - 1);
                if lines[lo..=hi].iter().any(|l| u.is_match(l)) {
                    continue;
                }
            }
            out.push(Hit {
                line: u32::try_from(i + 1).unwrap_or(u32::MAX),
                column: u32::try_from(line[..m.start()].chars().count() + 1).unwrap_or(1),
            });
        }
        out
    }
}

/// `//`, `/*`, `*`, `#` (not Rust `#[`/`#!`), `--`, `<!--` line comments.
pub fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    ["//", "/*", "* ", "--", "<!--"]
        .iter()
        .any(|p| t.starts_with(p))
        || t == "*"
        || (t.starts_with('#') && !t.starts_with("#[") && !t.starts_with("#!"))
}

/// Parse one rules file.
pub fn parse(text: &str) -> Result<Vec<RuleDef>, String> {
    toml::from_str::<RuleFile>(text)
        .map(|f| f.rule)
        .map_err(|e| e.to_string())
}

/// Built-in rules plus the project's `.kor/audit/rules/*.toml`; problems become notes.
pub fn load(root: &Path) -> (Vec<Rule>, Vec<String>) {
    let mut notes = Vec::new();
    let mut defs: BTreeMap<String, RuleDef> = BTreeMap::new();
    for d in parse(BUILTIN).unwrap_or_else(|e| {
        notes.push(format!("built-in rules: {e}"));
        Vec::new()
    }) {
        defs.insert(d.id.clone(), d);
    }
    let dir = root.join(USER_DIR);
    let mut user: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect()
        })
        .unwrap_or_default();
    user.sort();
    for p in user {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(list) => {
                for d in list {
                    defs.insert(d.id.clone(), d);
                }
            }
            Err(e) => notes.push(format!("{USER_DIR}/{name}: {e}")),
        }
    }
    let mut rules = Vec::new();
    for d in defs.into_values().filter(|d| d.enabled) {
        match Rule::compile(d) {
            Ok(r) => rules.push(r),
            Err(e) => notes.push(e),
        }
    }
    (rules, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(name: &str, text: &str) -> Source {
        Source {
            rel: name.into(),
            text: text.into(),
        }
    }

    #[test]
    fn builtin_rules_compile_and_are_complete() {
        let (rules, notes) = load(Path::new("/nonexistent"));
        assert!(notes.is_empty(), "{notes:?}");
        assert!(rules.len() > 30);
        for r in &rules {
            let d = &r.def;
            assert!(d.id.starts_with("kor."), "{}", d.id);
            assert!(
                d.cwe.as_deref().is_some_and(|c| c.starts_with("CWE-")),
                "{}",
                d.id
            );
            assert!(d.fix.is_some(), "{} has no fix hint", d.id);
            for l in Lang::ALL {
                assert!(!d.rationale.get(l).is_empty(), "{}", d.id);
            }
        }
    }

    #[test]
    fn unless_near_and_requires() {
        let def = RuleDef {
            id: "t".into(),
            severity: Severity::Low,
            cwe: None,
            ext: vec!["js".into()],
            pattern: r"\bexec\(".into(),
            unless: Some("safe".into()),
            near: 1,
            requires: Some("child_process".into()),
            rationale: Text::One("r".into()),
            fix: None,
            enabled: true,
        };
        let r = Rule::compile(def).unwrap();
        let text = "const cp = require('child_process')\nexec(a)\n// exec(b)\nx\nexec(c)\nsafe\n";
        let hits = r.hits(&src("a.js", text));
        assert_eq!(hits, vec![Hit { line: 2, column: 1 }]);
        assert!(r.hits(&src("a.py", text)).is_empty());
        assert!(r.hits(&src("b.js", "exec(a)\n")).is_empty());
    }

    #[test]
    fn user_rules_override_and_disable() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join(USER_DIR);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("mine.toml"),
            "[[rule]]\nid = \"kor.eval\"\nseverity = \"low\"\npattern = \"x\"\nrationale = \"r\"\nenabled = false\n\n[[rule]]\nid = \"acme.todo\"\nseverity = \"info\"\npattern = \"TODO\"\nrationale = \"r\"\n",
        )
        .unwrap();
        std::fs::write(d.join("bad.toml"), "[[rule]]\nid = 1\n").unwrap();
        let (rules, notes) = load(dir.path());
        assert!(rules.iter().any(|r| r.def.id == "acme.todo"));
        assert!(!rules.iter().any(|r| r.def.id == "kor.eval"));
        assert_eq!(notes.len(), 1);
    }
}
