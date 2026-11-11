//! Markdown (localized), JSON and SARIF 2.1.0 renderers for a [`Report`].

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use serde_json::{Value, json};

use super::{Report, Severity, slash};

/// Output format of [`render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Markdown,
    Json,
    Sarif,
}

impl Format {
    /// `md|markdown|json|sarif`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "sarif" => Some(Self::Sarif),
            _ => None,
        }
    }

    /// File extension for `--out` defaults.
    pub fn ext(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
            Self::Sarif => "sarif",
        }
    }
}

/// Render `report`; only Markdown is localized (JSON and SARIF are machine formats).
pub fn render(report: &Report, format: Format, l: &Localizer) -> String {
    match format {
        Format::Markdown => markdown(report, l),
        Format::Json => serde_json::to_string_pretty(report).unwrap_or_default(),
        Format::Sarif => serde_json::to_string_pretty(&sarif(report)).unwrap_or_default(),
    }
}

pub(crate) fn severity_label(s: Severity, l: &Localizer) -> String {
    l.t(&format!("analysis-sev-{}", s.as_str()))
}

/// Table cells must not break the row.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn markdown(r: &Report, l: &Localizer) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {}\n",
        l.t_args("analysis-md-title", &args!("tool" => r.tool.clone()))
    );
    let counts = r.counts();
    let summary: Vec<String> = Severity::ALL
        .iter()
        .zip(counts)
        .map(|(s, n)| format!("{}: {n}", severity_label(*s, l)))
        .collect();
    let _ = writeln!(
        out,
        "{} — {}\n",
        l.t_args("analysis-md-total", &args!("count" => r.findings.len())),
        summary.join(", ")
    );
    if r.findings.is_empty() {
        let _ = writeln!(out, "{}\n", l.t("analysis-md-none"));
    } else {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |\n|---|---|---|---|---|",
            l.t("analysis-md-col-severity"),
            l.t("analysis-md-col-location"),
            l.t("analysis-md-col-rule"),
            l.t("analysis-md-col-source"),
            l.t("analysis-md-col-message"),
        );
        for f in &r.findings {
            let rule = match &f.cwe {
                Some(c) => format!("{} ({c})", f.rule),
                None => f.rule.clone(),
            };
            let _ = writeln!(
                out,
                "| {} | `{}` | {} | {} | {} |",
                severity_label(f.severity, l),
                cell(&f.location()),
                cell(&rule),
                cell(&f.source),
                cell(&f.message)
            );
        }
        let fixes: Vec<_> = r.findings.iter().filter(|f| f.fix.is_some()).collect();
        if !fixes.is_empty() {
            let _ = writeln!(out, "\n## {}\n", l.t("analysis-md-fixes"));
            for f in fixes {
                let _ = writeln!(
                    out,
                    "- `{}` {}: {}",
                    f.location(),
                    f.rule,
                    f.fix.as_deref().unwrap_or_default()
                );
            }
        }
    }
    if !r.notes.is_empty() {
        let _ = writeln!(out, "\n## {}\n", l.t("analysis-md-notes"));
        for n in &r.notes {
            let _ = writeln!(out, "- {n}");
        }
    }
    out
}

fn level(s: Severity) -> &'static str {
    match s {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low | Severity::Info => "note",
    }
}

/// RFC 3986 path for `artifactLocation.uri` (relative to `%SRCROOT%`).
fn uri(p: &std::path::Path) -> String {
    let mut out = String::new();
    for b in slash(p).bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn sarif(r: &Report) -> Value {
    // One rule descriptor per id; the first finding's data describes it.
    let mut rules: BTreeMap<&str, Value> = BTreeMap::new();
    for f in &r.findings {
        rules.entry(f.rule.as_str()).or_insert_with(|| {
            let mut props = json!({ "source": f.source });
            if let Some(c) = &f.cwe {
                props["tags"] = json!([c, "security"]);
            }
            let mut rule = json!({
                "id": f.rule,
                "shortDescription": { "text": f.rule },
                "defaultConfiguration": { "level": level(f.severity) },
                "properties": props,
            });
            if let Some(fix) = &f.fix {
                rule["help"] = json!({ "text": fix });
            }
            rule
        });
    }
    let index: BTreeMap<&str, usize> = rules.keys().enumerate().map(|(i, k)| (*k, i)).collect();
    let results: Vec<Value> = r
        .findings
        .iter()
        .map(|f| {
            let mut region = json!({ "startLine": f.line.max(1) });
            if let Some(c) = f.column {
                region["startColumn"] = json!(c.max(1));
            }
            json!({
                "ruleId": f.rule,
                "ruleIndex": index[f.rule.as_str()],
                "level": level(f.severity),
                "message": { "text": f.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": uri(&f.file), "uriBaseId": "%SRCROOT%" },
                        "region": region,
                    }
                }],
                "properties": { "source": f.source, "severity": f.severity.as_str() },
            })
        })
        .collect();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "kor",
                "informationUri": kor_platform::brand().urls.repository,
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "rules": rules.into_values().collect::<Vec<_>>(),
            }},
            "automationDetails": { "id": format!("kor/{}/", r.tool) },
            "results": results,
            "properties": { "notes": r.notes },
        }]
    })
}

/// Structural SARIF 2.1.0 check (the required fields CI readers rely on); used by tests and
/// `kor audit verify-report`. Returns the first problem found.
pub fn check_sarif(v: &Value) -> Result<(), String> {
    if v["version"] != "2.1.0" {
        return Err("version != 2.1.0".into());
    }
    let runs = v["runs"].as_array().ok_or("runs missing")?;
    for run in runs {
        if run["tool"]["driver"]["name"]
            .as_str()
            .is_none_or(str::is_empty)
        {
            return Err("tool.driver.name missing".into());
        }
        let rules = run["tool"]["driver"]["rules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for res in run["results"].as_array().ok_or("results missing")? {
            let id = res["ruleId"].as_str().filter(|s| !s.is_empty());
            let id = id.ok_or("result without ruleId")?;
            if res["message"]["text"].as_str().is_none() {
                return Err(format!("{id}: message.text missing"));
            }
            if !matches!(
                res["level"].as_str(),
                Some("none" | "note" | "warning" | "error")
            ) {
                return Err(format!("{id}: bad level"));
            }
            if let Some(i) = res["ruleIndex"].as_u64() {
                let ok = usize::try_from(i)
                    .ok()
                    .and_then(|i| rules.get(i))
                    .is_some_and(|r| r["id"] == id);
                if !ok {
                    return Err(format!("{id}: ruleIndex does not point at its rule"));
                }
            }
            let loc = &res["locations"][0]["physicalLocation"];
            if loc["artifactLocation"]["uri"]
                .as_str()
                .is_none_or(str::is_empty)
            {
                return Err(format!("{id}: artifactLocation.uri missing"));
            }
            if loc["region"]["startLine"].as_u64().is_none_or(|n| n < 1) {
                return Err(format!("{id}: region.startLine must be >= 1"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::Finding;
    use super::*;
    use kor_i18n::Lang;

    fn report() -> Report {
        Report {
            tool: "security".into(),
            findings: vec![Finding {
                rule: "kor.sql-injection".into(),
                source: "kor".into(),
                severity: Severity::High,
                file: "src/a b|c.py".into(),
                line: 7,
                column: Some(3),
                message: "query | built".into(),
                fix: Some("use parameters".into()),
                cwe: Some("CWE-89".into()),
            }],
            notes: vec!["semgrep not found".into()],
        }
    }

    #[test]
    fn sarif_has_required_fields() {
        let v: Value =
            serde_json::from_str(&render(&report(), Format::Sarif, &Localizer::new(Lang::En)))
                .unwrap();
        check_sarif(&v).unwrap();
        let res = &v["runs"][0]["results"][0];
        assert_eq!(res["level"], "error");
        assert_eq!(
            res["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "src/a%20b%7Cc.py"
        );
        assert_eq!(
            res["locations"][0]["physicalLocation"]["region"]["startLine"],
            7
        );
    }

    #[test]
    fn check_sarif_rejects_missing_rule() {
        let mut v = sarif(&report());
        v["runs"][0]["results"][0]["ruleId"] = json!("");
        assert!(check_sarif(&v).is_err());
    }

    #[test]
    fn json_round_trips() {
        let s = render(&report(), Format::Json, &Localizer::new(Lang::En));
        let back: Report = serde_json::from_str(&s).unwrap();
        assert_eq!(back, report());
    }

    #[test]
    fn markdown_is_localized_and_escaped() {
        let tr = render(&report(), Format::Markdown, &Localizer::new(Lang::Tr));
        let en = render(&report(), Format::Markdown, &Localizer::new(Lang::En));
        assert_ne!(tr, en);
        assert!(en.contains("`src/a b|c.py:7`") || en.contains("src/a b\\|c.py:7"));
        assert!(en.contains("query \\| built"));
        assert!(en.contains("CWE-89"));
        assert!(!en.contains("analysis-md-"));
    }
}
