//! SPDX license classification and `kor audit --licenses` (spec §9, ADR 0046).

use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};

use super::{Ecosystem, Inventory, Package};
use crate::analysis::{Finding, Report, Severity};

/// How a license affects shipping the dependency in a commercial product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenseClass {
    Permissive,
    WeakCopyleft,
    StrongCopyleft,
    /// Source-available or non-commercial terms (`UNLICENSED`, `BUSL`, Elastic, `PolyForm`, `CC-BY-NC`…).
    Proprietary,
    Unknown,
}

impl LicenseClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Permissive => "permissive",
            Self::WeakCopyleft => "weak-copyleft",
            Self::StrongCopyleft => "strong-copyleft",
            Self::Proprietary => "proprietary",
            Self::Unknown => "unknown",
        }
    }
}

/// A parsed SPDX license expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Id(String),
    With(Box<Expr>, String),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

/// Parse `MIT OR (Apache-2.0 AND BSD-3-Clause)`, `GPL-2.0-only WITH Classpath-exception-2.0`.
/// Anything else (free text, `MIT/Apache-2.0`) is `None`.
pub fn parse(expr: &str) -> Option<Expr> {
    let spaced = expr.replace('(', " ( ").replace(')', " ) ");
    let toks: Vec<&str> = spaced.split_whitespace().collect();
    let mut pos = 0;
    let e = parse_or(&toks, &mut pos)?;
    (pos == toks.len()).then_some(e)
}

fn is_op(t: &str, op: &str) -> bool {
    t.eq_ignore_ascii_case(op)
}

fn parse_or(t: &[&str], pos: &mut usize) -> Option<Expr> {
    let mut left = parse_and(t, pos)?;
    while t.get(*pos).is_some_and(|x| is_op(x, "OR")) {
        *pos += 1;
        left = Expr::Or(Box::new(left), Box::new(parse_and(t, pos)?));
    }
    Some(left)
}

fn parse_and(t: &[&str], pos: &mut usize) -> Option<Expr> {
    let mut left = parse_with(t, pos)?;
    while t.get(*pos).is_some_and(|x| is_op(x, "AND")) {
        *pos += 1;
        left = Expr::And(Box::new(left), Box::new(parse_with(t, pos)?));
    }
    Some(left)
}

fn parse_with(t: &[&str], pos: &mut usize) -> Option<Expr> {
    let base = match *t.get(*pos)? {
        "(" => {
            *pos += 1;
            let e = parse_or(t, pos)?;
            (t.get(*pos) == Some(&")")).then_some(())?;
            *pos += 1;
            e
        }
        id if valid_id(id) => {
            *pos += 1;
            Expr::Id(id.to_owned())
        }
        _ => return None,
    };
    if t.get(*pos).is_some_and(|x| is_op(x, "WITH")) {
        let exc = t.get(*pos + 1).filter(|x| valid_id(x))?;
        *pos += 2;
        return Some(Expr::With(Box::new(base), (*exc).to_owned()));
    }
    Some(base)
}

fn valid_id(t: &str) -> bool {
    !t.is_empty()
        && !["AND", "OR", "WITH"].iter().any(|op| is_op(t, op))
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+' | ':'))
}

const PERMISSIVE: &[&str] = &[
    "0BSD",
    "AFL-2.1",
    "AFL-3.0",
    "APACHE-1.1",
    "APACHE-2.0",
    "ARTISTIC-2.0",
    "BLUEOAK-1.0.0",
    "BSD-1-CLAUSE",
    "BSD-2-CLAUSE",
    "BSD-3-CLAUSE",
    "BSD-3-CLAUSE-CLEAR",
    "BSL-1.0",
    "BZIP2-1.0.6",
    "CC-BY-3.0",
    "CC-BY-4.0",
    "CC0-1.0",
    "CURL",
    "HPND",
    "ISC",
    "MIT",
    "MIT-0",
    "MIT-CMU",
    "NCSA",
    "OPENSSL",
    "POSTGRESQL",
    "PSF-2.0",
    "PYTHON-2.0",
    "UNICODE-3.0",
    "UNICODE-DFS-2016",
    "UNLICENSE",
    "UPL-1.0",
    "WTFPL",
    "X11",
    "ZLIB",
    "ZPL-2.1",
];
const WEAK: &[&str] = &["LGPL-", "MPL-", "EPL-", "CDDL-", "CPL-", "MS-RL", "MS-PL"];
const STRONG: &[&str] = &[
    "AGPL-",
    "GPL-",
    "SSPL-",
    "OSL-",
    "EUPL-",
    "CC-BY-SA-",
    "RPL-",
    "SLEEPYCAT",
];
const PROPRIETARY: &[&str] = &[
    "UNLICENSED",
    "BUSL-",
    "ELASTIC-",
    "COMMONS-CLAUSE",
    "POLYFORM-",
    "CC-BY-NC",
];
/// Exceptions that let proprietary code link a GPL library.
const LINKING_EXCEPTIONS: &[&str] = &[
    "CLASSPATH-EXCEPTION-2.0",
    "GCC-EXCEPTION-2.0",
    "GCC-EXCEPTION-3.1",
    "LLVM-EXCEPTION",
    "LGPL-3.0-LINKING-EXCEPTION",
    "FONT-EXCEPTION-2.0",
];

pub fn classify_id(id: &str) -> LicenseClass {
    let u = id.trim_end_matches('+').to_ascii_uppercase();
    let has = |list: &[&str]| list.iter().any(|p| u.starts_with(p));
    if PERMISSIVE.contains(&u.as_str()) {
        LicenseClass::Permissive
    } else if has(PROPRIETARY) {
        LicenseClass::Proprietary
    } else if has(WEAK) {
        LicenseClass::WeakCopyleft
    } else if has(STRONG) {
        LicenseClass::StrongCopyleft
    } else {
        LicenseClass::Unknown
    }
}

/// `OR` lets the user pick the least restrictive side; `AND` binds to the most restrictive.
/// Unknown ranks as the most restrictive, so an unknown part of an `AND` is never hidden.
pub fn classify_expr(e: &Expr) -> LicenseClass {
    match e {
        Expr::Id(id) => classify_id(id),
        Expr::With(base, exc) => {
            let c = classify_expr(base);
            if c == LicenseClass::StrongCopyleft
                && LINKING_EXCEPTIONS.contains(&exc.to_ascii_uppercase().as_str())
            {
                LicenseClass::WeakCopyleft
            } else {
                c
            }
        }
        Expr::And(a, b) => classify_expr(a).max(classify_expr(b)),
        Expr::Or(a, b) => classify_expr(a).min(classify_expr(b)),
    }
}

/// Class of a declared license string; not SPDX → unknown.
pub fn classify(license: Option<&str>) -> LicenseClass {
    license
        .and_then(parse)
        .map_or(LicenseClass::Unknown, |e| classify_expr(&e))
}

/// Case-insensitive glob with `*` only.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.to_ascii_lowercase(), text.to_ascii_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == t;
    }
    let mut rest = t.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(at) = rest.find(part) {
            rest = &rest[at + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// The deny pattern that the license cannot avoid (an `OR` is denied only when every side is).
pub fn denied_by<'a>(license: &str, deny: &'a [String]) -> Option<&'a str> {
    fn hit<'a>(e: &Expr, deny: &'a [String]) -> Option<&'a str> {
        match e {
            Expr::Id(id) => deny.iter().find(|p| glob(p, id)).map(String::as_str),
            Expr::With(base, _) => hit(base, deny),
            Expr::And(a, b) => hit(a, deny).or_else(|| hit(b, deny)),
            Expr::Or(a, b) => hit(a, deny).and(hit(b, deny)),
        }
    }
    match parse(license) {
        Some(e) => hit(&e, deny),
        None => deny
            .iter()
            .find(|p| glob(p, license.trim()))
            .map(String::as_str),
    }
}

/// A replacement with a different license, each entry checked against the alternative's own
/// license page (`source`). Kept deliberately small: no entry, no suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Alternative {
    pub ecosystem: Ecosystem,
    pub package: &'static str,
    pub alternative: &'static str,
    pub license: &'static str,
    pub source: &'static str,
}

pub const ALTERNATIVES: &[Alternative] = &[
    Alternative {
        ecosystem: Ecosystem::PyPI,
        package: "pyqt5",
        alternative: "PySide6",
        license: "LGPL-3.0-only",
        source: "https://pypi.org/project/PySide6/",
    },
    Alternative {
        ecosystem: Ecosystem::PyPI,
        package: "pyqt6",
        alternative: "PySide6",
        license: "LGPL-3.0-only",
        source: "https://pypi.org/project/PySide6/",
    },
    Alternative {
        ecosystem: Ecosystem::PyPI,
        package: "mysqlclient",
        alternative: "PyMySQL",
        license: "MIT",
        source: "https://pypi.org/project/PyMySQL/",
    },
    Alternative {
        ecosystem: Ecosystem::PyPI,
        package: "pymupdf",
        alternative: "pypdf",
        license: "BSD-3-Clause",
        source: "https://pypi.org/project/pypdf/",
    },
    Alternative {
        ecosystem: Ecosystem::Maven,
        package: "com.itextpdf:itextpdf",
        alternative: "org.apache.pdfbox:pdfbox",
        license: "Apache-2.0",
        source: "https://pdfbox.apache.org/",
    },
    Alternative {
        ecosystem: Ecosystem::Maven,
        package: "com.itextpdf:kernel",
        alternative: "org.apache.pdfbox:pdfbox",
        license: "Apache-2.0",
        source: "https://pdfbox.apache.org/",
    },
    Alternative {
        ecosystem: Ecosystem::Packagist,
        package: "mpdf/mpdf",
        alternative: "dompdf/dompdf",
        license: "LGPL-2.1",
        source: "https://github.com/dompdf/dompdf",
    },
    Alternative {
        ecosystem: Ecosystem::Npm,
        package: "highcharts",
        alternative: "chart.js",
        license: "MIT",
        source: "https://github.com/chartjs/Chart.js",
    },
];

pub fn alternative(p: &Package) -> Option<&'static Alternative> {
    ALTERNATIVES
        .iter()
        .find(|a| a.ecosystem == p.ecosystem && super::norm(a.package) == super::norm(&p.name))
}

/// One package's license status (for `kor deps list` and the desktop table).
#[derive(Debug, Clone, Serialize)]
pub struct Classified<'a> {
    #[serde(flatten)]
    pub package: &'a Package,
    pub class: LicenseClass,
}

pub fn classified(inv: &Inventory) -> Vec<Classified<'_>> {
    inv.packages
        .iter()
        .map(|p| Classified {
            package: p,
            class: classify(p.license.as_deref()),
        })
        .collect()
}

/// `name version (ecosystem)` for messages.
pub(crate) fn label(p: &Package) -> String {
    if p.version.is_empty() {
        format!("{} ({})", p.name, p.ecosystem.osv())
    } else {
        format!("{} {} ({})", p.name, p.version, p.ecosystem.osv())
    }
}

/// "; pulled in by a, b, c (+N)" for transitive packages.
pub(crate) fn via_suffix(p: &Package, l: &Localizer) -> String {
    if p.direct || p.via.is_empty() {
        return String::new();
    }
    let mut names = p.via.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if p.via.len() > 3 {
        let _ = write!(names, " (+{})", p.via.len() - 3);
    }
    format!("; {}", l.t_args("deps-via", &args!("names" => names)))
}

pub(crate) fn inventory_notes(inv: &Inventory, l: &Localizer, out: &mut Vec<String>) {
    let sources: Vec<String> = inv
        .sources
        .iter()
        .map(|s| crate::analysis::slash(s))
        .collect();
    out.push(if sources.is_empty() {
        l.t("deps-no-sources")
    } else {
        l.t_args(
            "deps-sources",
            &args!("count" => inv.packages.len(), "files" => sources.join(", ")),
        )
    });
    for (file, e) in &inv.errors {
        out.push(l.t_args(
            "deps-parse-failed",
            &args!("file" => crate::analysis::slash(file), "error" => e.clone()),
        ));
    }
}

/// `kor audit --licenses`: commercial-use problems, policy denials and unknown licenses.
pub fn licenses_report(inv: &Inventory, deny: &[String], l: &Localizer) -> Report {
    let mut r = Report::new("licenses");
    inventory_notes(inv, l, &mut r.notes);
    let mut counts = [0usize; 5];
    for p in &inv.packages {
        let class = classify(p.license.as_deref());
        counts[class as usize] += 1;
        let lic = p
            .license
            .clone()
            .unwrap_or_else(|| l.t("deps-license-none"));
        let pkg = label(p);
        let denied = p.license.as_deref().and_then(|x| denied_by(x, deny));
        let (rule, mut severity, msg) = if let Some(pattern) = denied {
            (
                "license-denied",
                Severity::Critical,
                l.t_args(
                    "deps-lic-denied",
                    &args!("pkg" => pkg, "license" => lic, "pattern" => pattern.to_owned()),
                ),
            )
        } else {
            match class {
                LicenseClass::Permissive => continue,
                LicenseClass::WeakCopyleft => (
                    "license-weak-copyleft",
                    Severity::Info,
                    l.t_args("deps-lic-weak", &args!("pkg" => pkg, "license" => lic)),
                ),
                LicenseClass::StrongCopyleft => (
                    "license-strong-copyleft",
                    Severity::High,
                    l.t_args("deps-lic-strong", &args!("pkg" => pkg, "license" => lic)),
                ),
                LicenseClass::Proprietary => (
                    "license-proprietary",
                    Severity::High,
                    l.t_args(
                        "deps-lic-proprietary",
                        &args!("pkg" => pkg, "license" => lic),
                    ),
                ),
                LicenseClass::Unknown => (
                    "license-unknown",
                    Severity::Low,
                    l.t_args("deps-lic-unknown", &args!("pkg" => pkg, "license" => lic)),
                ),
            }
        };
        // Development-only dependencies are not shipped; a policy denial still counts.
        if p.dev && rule != "license-denied" && severity < Severity::Info {
            severity = Severity::Info;
        }
        let fix = alternative(p).map(|a| {
            l.t_args(
                "deps-lic-alternative",
                &args!("alt" => a.alternative, "license" => a.license, "source" => a.source),
            )
        });
        r.findings.push(Finding {
            rule: rule.into(),
            source: "spdx".into(),
            severity,
            file: p.file.clone(),
            line: p.line,
            column: None,
            message: format!(
                "{msg}{}{}",
                if p.dev {
                    format!(" {}", l.t("deps-dev-only"))
                } else {
                    String::new()
                },
                via_suffix(p, l)
            ),
            fix,
            cwe: None,
        });
    }
    r.notes.push(l.t_args(
        "deps-lic-summary",
        &args!(
            "permissive" => counts[0],
            "weak" => counts[1],
            "strong" => counts[2],
            "proprietary" => counts[3],
            "unknown" => counts[4]
        ),
    ));
    if !deny.is_empty() {
        r.notes
            .push(l.t_args("deps-lic-policy", &args!("patterns" => deny.join(", "))));
    }
    r.normalize();
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use LicenseClass::*;

    #[test]
    fn classification_table() {
        for (expr, want) in [
            ("MIT", Permissive),
            ("Apache-2.0", Permissive),
            ("mit", Permissive),
            ("MIT OR Apache-2.0", Permissive),
            ("(MIT OR Apache-2.0) AND Unicode-3.0", Permissive),
            ("MPL-2.0", WeakCopyleft),
            ("LGPL-2.1-or-later", WeakCopyleft),
            ("EPL-2.0", WeakCopyleft),
            ("GPL-3.0-only", StrongCopyleft),
            ("GPL-2.0+", StrongCopyleft),
            ("AGPL-3.0-or-later", StrongCopyleft),
            ("SSPL-1.0", StrongCopyleft),
            ("GPL-2.0-only WITH Classpath-exception-2.0", WeakCopyleft),
            ("Apache-2.0 WITH LLVM-exception", Permissive),
            ("GPL-3.0-only OR MIT", Permissive),
            ("GPL-3.0-only AND MIT", StrongCopyleft),
            ("MIT AND LicenseRef-custom", Unknown),
            ("UNLICENSED", Proprietary),
            ("BUSL-1.1", Proprietary),
            ("Elastic-2.0", Proprietary),
            ("CC-BY-NC-4.0", Proprietary),
            ("CC-BY-SA-4.0", StrongCopyleft),
            ("MIT/Apache-2.0", Unknown),
            ("BSD License", Unknown),
            ("(MIT", Unknown),
        ] {
            assert_eq!(classify(Some(expr)), want, "{expr}");
        }
        assert_eq!(classify(None), Unknown);
    }

    #[test]
    fn deny_patterns_respect_or_choices() {
        let deny = vec!["AGPL-3.0*".to_owned(), "SSPL-1.0".to_owned()];
        assert_eq!(denied_by("AGPL-3.0-only", &deny), Some("AGPL-3.0*"));
        assert_eq!(denied_by("agpl-3.0-or-later", &deny), Some("AGPL-3.0*"));
        assert_eq!(denied_by("AGPL-3.0-only OR MIT", &deny), None);
        assert_eq!(denied_by("MIT AND SSPL-1.0", &deny), Some("SSPL-1.0"));
        assert_eq!(denied_by("GPL-3.0-only", &deny), None);
        assert!(glob("*gpl*", "LGPL-2.1"));
        assert!(!glob("GPL-*", "LGPL-2.1"));
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn report_flags_with_path_and_alternative() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let f = std::path::Path::new("requirements.txt");
        let mut a = Package::new(Ecosystem::PyPI, "PyMuPDF", "1.23.0", f, 3);
        a.direct = true;
        a.license = Some("AGPL-3.0-only".into());
        let mut b = Package::new(
            Ecosystem::PyPI,
            "chardet",
            "5.0",
            std::path::Path::new("poetry.lock"),
            40,
        );
        b.license = Some("LGPL-2.1-or-later".into());
        b.via = vec!["requests".into()];
        let mut c = Package::new(Ecosystem::PyPI, "ok", "1", f, 4);
        c.license = Some("MIT".into());
        let d = Package::new(Ecosystem::PyPI, "mystery", "1", f, 5);
        let inv = Inventory {
            packages: vec![a, b, c, d],
            sources: vec![f.into()],
            errors: vec![],
        };
        let r = licenses_report(&inv, &["AGPL-3.0*".into()], &l);
        assert_eq!(r.findings.len(), 3);
        let denied = &r.findings[0];
        assert_eq!(
            (denied.rule.as_str(), denied.line, denied.severity),
            ("license-denied", 3, Severity::Critical)
        );
        assert!(denied.fix.as_deref().unwrap().contains("pypdf"));
        let weak = r
            .findings
            .iter()
            .find(|f| f.rule == "license-weak-copyleft")
            .unwrap();
        assert!(weak.message.contains("requests"), "{}", weak.message);
        assert!(r.findings.iter().any(|f| f.rule == "license-unknown"));
        assert!(r.findings.iter().all(|f| f.source == "spdx"));
    }
}
