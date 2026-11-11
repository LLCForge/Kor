//! `kor audit --compliance=kvkk|gdpr` (spec §9, ADR 0045): where personal data is processed.
//!
//! Sources, all deterministic and local:
//! - schema: tables and columns from kor-graph's extractors (SQL DDL, Prisma, Django, Rails,
//!   EF Core, Knex, Laravel) — the same facts the graph's `Table` nodes are built from;
//! - code: `ReadsTable` / `WritesTable` facts (SQL literals, ORM calls) on tables with personal
//!   columns, with the enclosing function from the tree-sitter parse;
//! - identifiers named after a personal-data category (`data/pii.toml`), log calls given one,
//!   checksum-valid TC kimlik / IBAN literals, and tables with no deletion/TTL path (Info).
//!
//! Article references are attached only where the mapping is direct; anything else has none.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use kor_i18n::{Localizer, args};
use regex::Regex;
use serde::Deserialize;

use super::rules::{MAX_LINE, Text, is_comment};
use super::walk::{self, Source};
use super::{Finding, Report, Severity};

pub const BUILTIN: &str = include_str!("data/pii.toml");
pub const USER_FILE: &str = ".kor/audit/pii.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Regime {
    Kvkk,
    Gdpr,
}

impl Regime {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "kvkk" => Some(Self::Kvkk),
            "gdpr" => Some(Self::Gdpr),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kvkk => "kvkk",
            Self::Gdpr => "gdpr",
        }
    }

    /// Law 6698 (KVKK) / Regulation (EU) 2016/679 (GDPR) articles for each kind of finding.
    fn reference(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::Kvkk, Kind::Inventory) => "KVKK m.16",
            (Self::Gdpr, Kind::Inventory) => "GDPR Art. 30",
            (Self::Kvkk, Kind::Special) => "KVKK m.6",
            (Self::Gdpr, Kind::Special) => "GDPR Art. 9",
            (Self::Kvkk, Kind::Security) => "KVKK m.12",
            (Self::Gdpr, Kind::Security) => "GDPR Art. 32",
            (Self::Kvkk, Kind::Retention) => "KVKK m.4/2-d, m.7",
            (Self::Gdpr, Kind::Retention) => "GDPR Art. 5(1)(e), Art. 17",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Processing inventory (records of processing / data controllers' registry).
    Inventory,
    Special,
    Security,
    Retention,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoryDef {
    id: String,
    label: Text,
    identifier: String,
    #[serde(default)]
    special: bool,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct CategoryFile {
    #[serde(default)]
    category: Vec<CategoryDef>,
}

#[derive(Debug, Clone)]
pub struct Category {
    pub id: String,
    pub label: Text,
    pub special: bool,
    re: Regex,
}

impl Category {
    pub fn matches(&self, s: &str) -> bool {
        self.re.is_match(s)
    }
}

/// Built-in categories plus `.kor/audit/pii.toml`; problems become notes.
pub fn categories(root: &Path) -> (Vec<Category>, Vec<String>) {
    let mut notes = Vec::new();
    let mut defs: BTreeMap<String, CategoryDef> = BTreeMap::new();
    let mut add = |text: &str, origin: &str, notes: &mut Vec<String>| match toml::from_str::<
        CategoryFile,
    >(text)
    {
        Ok(f) => {
            for d in f.category {
                defs.insert(d.id.clone(), d);
            }
        }
        Err(e) => notes.push(format!("{origin}: {e}")),
    };
    add(BUILTIN, "pii.toml", &mut notes);
    if let Ok(t) = std::fs::read_to_string(root.join(USER_FILE)) {
        add(&t, USER_FILE, &mut notes);
    }
    let mut out = Vec::new();
    for d in defs.into_values().filter(|d| d.enabled) {
        match Regex::new(&d.identifier) {
            Ok(re) => out.push(Category {
                id: d.id,
                label: d.label,
                special: d.special,
                re,
            }),
            Err(e) => notes.push(format!("{}: {e}", d.id)),
        }
    }
    (out, notes)
}

static LOG_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\bconsole\.(log|info|warn|error|debug|trace)|\blogger\.\w+|\blog\.(info|warn|warning|error|debug|trace|fatal|printf|println|print)|\blogging\.\w+|\bLog::\w+|\b(info|warn|error|debug|trace)!|\beprintln!|\bprintln!|\berror_log|\bSystem\.(out|err)\.print\w*|\b_?logger\.Log\w+|\bfmt\.Print\w*|\bprint_r|\bvar_dump|^\s*print)\s*\(",
    )
    .expect("static regex")
});
static TCKN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[1-9][0-9]{10}\b").expect("static regex"));
static IBAN_TR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bTR[0-9]{2}(?: ?[0-9]{4}){5} ?[0-9]{2}\b").expect("static regex")
});
static RETENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(delete|destroy|forceDelete|remove|purge|ttl|retention|expires?_?at|expire|anonymi[sz]\w*|prun\w*|truncate|softDeletes?|deleted_at|on_delete)\b",
    )
    .expect("static regex")
});

/// TC kimlik numarası checksum (10th and 11th digits).
pub fn valid_tckn(s: &str) -> bool {
    let d: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() != 11 || d[0] == 0 {
        return false;
    }
    let odd = d[0] + d[2] + d[4] + d[6] + d[8];
    let even = d[1] + d[3] + d[5] + d[7];
    let d10 = (odd * 7 + 10 * 9 - even) % 10;
    d10 == d[9] && (d[..10].iter().sum::<u32>() % 10) == d[10]
}

/// ISO 13616 mod-97 check.
pub fn valid_iban(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() < 15 || !s.is_ascii() {
        return false;
    }
    let (head, tail) = s.split_at(4);
    let mut rem: u64 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => u64::from(v),
            None => return false,
        };
        rem = if v >= 10 {
            (rem * 100 + v) % 97
        } else {
            (rem * 10 + v) % 97
        };
    }
    rem == 1
}

/// `user_profiles` / `UserProfile` → `userprofile`: model ↔ table matching.
fn loose(name: &str) -> String {
    let k = kor_graph::db::table_key(name).replace('_', "");
    k.strip_suffix("ies")
        .map(|s| format!("{s}y"))
        .or_else(|| k.strip_suffix('s').map(str::to_owned))
        .unwrap_or(k)
}

struct PiiTable {
    name: String,
    model: Option<String>,
    columns: Vec<String>,
    file: PathBuf,
    line: u32,
}

fn finding(
    rule: &str,
    severity: Severity,
    src: &Path,
    line: u32,
    message: String,
    fix: Option<String>,
    cwe: Option<&str>,
) -> Finding {
    Finding {
        rule: rule.to_owned(),
        source: "kor".into(),
        severity,
        file: src.to_path_buf(),
        line: line.max(1),
        column: None,
        message,
        fix,
        cwe: cwe.map(str::to_owned),
    }
}

#[allow(clippy::too_many_lines)]
/// Run the compliance scan over `root`.
pub fn scan(root: &Path, regime: Regime, l: &Localizer) -> Report {
    let lang = l.lang();
    let mut report = Report::new(format!("compliance-{}", regime.as_str()));
    let (cats, notes) = categories(root);
    report.notes.extend(notes);
    let (sources, capped) = walk::sources(root);
    if capped {
        report
            .notes
            .push(l.t_args("analysis-note-capped", &args!("count" => walk::MAX_FILES)));
    }
    let with_ref = |msg: String, kind: Kind| format!("{msg} [{}]", regime.reference(kind));
    let label = |c: &Category| c.label.get(lang);

    // Parse once: schema facts and table uses.
    let mut parsed = Vec::new();
    for src in &sources {
        let rel = walk_rel(src);
        let facts = kor_graph::parse::classify(&rel).and_then(|(kind, lang)| {
            matches!(
                kind,
                kor_graph::parse::FileKind::Code
                    | kor_graph::parse::FileKind::Sql
                    | kor_graph::parse::FileKind::Prisma
            )
            .then(|| kor_graph::parse::parse(kind, lang, &src.text))
        });
        parsed.push(facts);
    }

    // 1. Schema: personal columns.
    let mut pii_tables: BTreeMap<String, PiiTable> = BTreeMap::new();
    for (src, facts) in sources.iter().zip(&parsed) {
        let Some(facts) = facts else { continue };
        let lines: Vec<&str> = src.text.lines().collect();
        for t in &facts.tables {
            let display = if t.name.is_empty() {
                t.model.clone().unwrap_or_default()
            } else {
                t.name.clone()
            };
            let mut cols = Vec::new();
            for col in &t.columns {
                let col = col.split_whitespace().next().unwrap_or_default();
                let Some(c) = cats.iter().find(|c| c.matches(col)) else {
                    continue;
                };
                let line = column_line(&lines, t.start, t.end, col);
                let msg = l.t_args(
                    "analysis-pii-column",
                    &args!("column" => col, "table" => display.clone(), "category" => label(c)),
                );
                report.findings.push(if c.special {
                    finding(
                        "kor.pii.special-column",
                        Severity::Medium,
                        &src.rel,
                        line,
                        with_ref(msg, Kind::Special),
                        Some(l.t("analysis-pii-special-fix")),
                        Some("CWE-359"),
                    )
                } else {
                    finding(
                        "kor.pii.column",
                        Severity::Low,
                        &src.rel,
                        line,
                        with_ref(msg, Kind::Inventory),
                        None,
                        Some("CWE-359"),
                    )
                });
                cols.push(col.to_owned());
            }
            if !cols.is_empty() {
                let entry = pii_tables
                    .entry(loose(&display))
                    .or_insert_with(|| PiiTable {
                        name: display.clone(),
                        model: t.model.clone(),
                        columns: Vec::new(),
                        file: src.rel.clone(),
                        line: t.start.max(1),
                    });
                entry.columns.extend(cols);
            }
        }
    }

    // 2. Functions that read/write those tables.
    for (src, facts) in sources.iter().zip(&parsed) {
        let Some(facts) = facts else { continue };
        let mut seen = BTreeSet::new();
        for u in &facts.table_uses {
            let Some(t) = pii_tables.get(&loose(&u.name)) else {
                continue;
            };
            if !seen.insert((u.line, t.name.clone())) {
                continue;
            }
            let func = u
                .from
                .and_then(|i| facts.defs.get(i as usize))
                .map_or_else(|| l.t("analysis-pii-file-level"), |d| d.name.clone());
            let msg = l.t_args(
                if u.write {
                    "analysis-pii-writes"
                } else {
                    "analysis-pii-reads"
                },
                &args!(
                    "function" => func,
                    "table" => t.name.clone(),
                    "columns" => t.columns.join(", ")
                ),
            );
            report.findings.push(finding(
                "kor.pii.table-access",
                Severity::Info,
                &src.rel,
                u.line,
                with_ref(msg, Kind::Inventory),
                None,
                None,
            ));
        }
    }

    // 3. Code identifiers, log calls, literals.
    for (src, facts) in sources.iter().zip(&parsed) {
        let code = facts
            .as_ref()
            .is_some_and(|f| f.kind == kor_graph::parse::FileKind::Code);
        let mut first: BTreeMap<&str, (u32, usize)> = BTreeMap::new();
        for (i, line) in src.text.lines().enumerate() {
            let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
            if line.len() > MAX_LINE {
                continue;
            }
            for m in TCKN.find_iter(line).filter(|m| valid_tckn(m.as_str())) {
                let _ = m;
                report.findings.push(finding(
                    "kor.pii.literal-national-id",
                    Severity::Low,
                    &src.rel,
                    n,
                    with_ref(l.t("analysis-pii-literal-tckn"), Kind::Security),
                    Some(l.t("analysis-pii-literal-fix")),
                    Some("CWE-359"),
                ));
            }
            for _ in IBAN_TR.find_iter(line).filter(|m| valid_iban(m.as_str())) {
                report.findings.push(finding(
                    "kor.pii.literal-iban",
                    Severity::Low,
                    &src.rel,
                    n,
                    with_ref(l.t("analysis-pii-literal-iban"), Kind::Security),
                    Some(l.t("analysis-pii-literal-fix")),
                    Some("CWE-359"),
                ));
            }
            if !code || is_comment(line) {
                continue;
            }
            if let Some(m) = LOG_CALL.find(line) {
                let args_part = &line[m.end()..];
                if let Some(c) = cats.iter().find(|c| c.matches(args_part)) {
                    report.findings.push(finding(
                        "kor.pii.log",
                        if c.special {
                            Severity::High
                        } else {
                            Severity::Medium
                        },
                        &src.rel,
                        n,
                        with_ref(
                            l.t_args("analysis-pii-log", &args!("category" => label(c))),
                            Kind::Security,
                        ),
                        Some(l.t("analysis-pii-log-fix")),
                        Some("CWE-532"),
                    ));
                }
            }
            for c in &cats {
                if c.matches(line) {
                    let e = first.entry(c.id.as_str()).or_insert((n, 0));
                    e.1 += 1;
                }
            }
        }
        for (id, (line, count)) in first {
            let Some(c) = cats.iter().find(|c| c.id == id) else {
                continue;
            };
            let msg = l.t_args(
                "analysis-pii-identifier",
                &args!("category" => label(c), "count" => count),
            );
            report.findings.push(finding(
                "kor.pii.identifier",
                Severity::Info,
                &src.rel,
                line,
                with_ref(
                    msg,
                    if c.special {
                        Kind::Special
                    } else {
                        Kind::Inventory
                    },
                ),
                None,
                None,
            ));
        }
    }

    // 4. Retention: a personal table nobody deletes, expires or anonymizes.
    for t in pii_tables.values() {
        let mut names = vec![kor_graph::db::table_key(&t.name)];
        names.extend(t.model.iter().map(|m| m.to_lowercase()));
        names.retain(|n| !n.is_empty());
        let handled = sources.iter().any(|s| {
            s.text.lines().any(|line| {
                RETENTION.is_match(line) && {
                    let low = line.to_lowercase();
                    names.iter().any(|n| kor_graph::db::mentions_table(&low, n))
                }
            })
        });
        if !handled {
            report.findings.push(finding(
                "kor.pii.retention",
                Severity::Info,
                &t.file,
                t.line,
                with_ref(
                    l.t_args("analysis-pii-retention", &args!("table" => t.name.clone())),
                    Kind::Retention,
                ),
                Some(l.t("analysis-pii-retention-fix")),
                None,
            ));
        }
    }

    report.notes.push(l.t_args(
        "analysis-note-compliance",
        &args!("files" => sources.len(), "tables" => pii_tables.len()),
    ));
    report.normalize();
    report
}

fn walk_rel(src: &Source) -> String {
    super::slash(&src.rel)
}

/// Line of `col` inside the table's span, else the span start.
fn column_line(lines: &[&str], start: u32, end: u32, col: &str) -> u32 {
    let word = Regex::new(&format!(r"\b{}\b", regex::escape(col))).ok();
    let lo = start.max(1) as usize;
    let hi = (end as usize).max(lo).min(lines.len());
    (lo..=hi)
        .find(|n| {
            word.as_ref()
                .is_some_and(|w| lines.get(n - 1).is_some_and(|l| w.is_match(l)))
        })
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_else(|| start.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_i18n::Lang;

    #[test]
    fn checksums() {
        assert!(valid_tckn("10000000146"));
        assert!(!valid_tckn("10000000147"));
        assert!(!valid_tckn("12345678901"));
        assert!(valid_iban("TR33 0006 1005 1978 6457 8413 26"));
        assert!(!valid_iban("TR33 0006 1005 1978 6457 8413 27"));
    }

    #[test]
    fn categories_avoid_obvious_false_positives() {
        let (cats, notes) = categories(Path::new("/nonexistent"));
        assert!(notes.is_empty(), "{notes:?}");
        let hit = |s: &str| cats.iter().find(|c| c.matches(s)).map(|c| c.id.clone());
        assert_eq!(hit("tc_kimlik_no").as_deref(), Some("national-id"));
        assert_eq!(hit("user_email").as_deref(), Some("email"));
        assert_eq!(hit("userPhone").as_deref(), Some("phone"));
        assert_eq!(hit("dogum_tarihi").as_deref(), Some("birth-date"));
        assert_eq!(hit("blood_type").as_deref(), Some("health"));
        for no in [
            "classname",
            "microphone",
            "trace",
            "adobe",
            "healthcheck",
            "quirk",
            "brace",
        ] {
            assert_eq!(hit(no), None, "{no}");
        }
    }

    #[test]
    fn regime_references_differ() {
        assert_eq!(Regime::parse("KVKK"), Some(Regime::Kvkk));
        assert_ne!(
            Regime::Kvkk.reference(Kind::Security),
            Regime::Gdpr.reference(Kind::Security)
        );
        let _ = Localizer::new(Lang::En);
    }
}
