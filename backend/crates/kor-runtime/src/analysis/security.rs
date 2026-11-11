//! `kor audit --security` (spec §9, ADR 0045): Kor's own line rules + kor-tools' secret scanner,
//! merged with gitleaks / semgrep JSON when they are on PATH. Deterministic and offline (semgrep
//! registry rules are only fetched outside privacy/offline mode).

use kor_platform::NoWindow as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kor_i18n::Localizer;
use serde_json::Value;

use super::rules::{self, Rule};
use super::walk::{self, Source};
use super::{Finding, Report, Severity};

/// Longest a single external scanner may run.
pub const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Skip gitleaks / semgrep even when installed (tests, `--no-external`).
    pub no_external: bool,
}

/// Run every scanner over `root`.
pub fn scan(root: &Path, opts: &Options, l: &Localizer) -> Report {
    let mut report = Report::new("security");
    let (rules, notes) = rules::load(root);
    report.notes.extend(notes);
    let (sources, capped) = walk::sources(root);
    if capped {
        report.notes.push(l.t_args(
            "analysis-note-capped",
            &kor_i18n::args!("count" => walk::MAX_FILES),
        ));
    }
    for src in &sources {
        report.findings.extend(secrets(src, l));
        report.findings.extend(line_rules(src, &rules, l));
    }
    if !opts.no_external {
        external(root, &mut report, l);
    }
    report.notes.push(l.t_args(
        "analysis-note-scanned",
        &kor_i18n::args!("files" => sources.len(), "rules" => rules.len()),
    ));
    report.normalize();
    report
}

/// kor-tools' high-confidence secret rules (gitleaks ids, so gitleaks' own hits dedup).
pub fn secrets(src: &Source, l: &Localizer) -> Vec<Finding> {
    kor_tools::find_secrets(&src.text)
        .into_iter()
        .map(|(id, at)| Finding {
            rule: id.to_owned(),
            source: "kor".into(),
            severity: Severity::Critical,
            file: src.rel.clone(),
            line: walk::line_of(&src.text, at),
            column: None,
            message: l.t_args("analysis-secret", &kor_i18n::args!("rule" => id)),
            fix: Some(l.t("analysis-secret-fix")),
            cwe: Some("CWE-798".into()),
        })
        .collect()
}

pub fn line_rules(src: &Source, rules: &[Rule], l: &Localizer) -> Vec<Finding> {
    let lang = l.lang();
    rules
        .iter()
        .flat_map(|r| {
            r.hits(src).into_iter().map(|h| Finding {
                rule: r.def.id.clone(),
                source: "kor".into(),
                severity: r.def.severity,
                file: src.rel.clone(),
                line: h.line,
                column: Some(h.column),
                message: r.def.rationale.get(lang),
                fix: r.def.fix.as_ref().map(|f| f.get(lang)),
                cwe: r.def.cwe.clone(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// External scanners
// ---------------------------------------------------------------------------------------------

fn external(root: &Path, report: &mut Report, l: &Localizer) {
    match crate::deploy::which("gitleaks") {
        Some(bin) => {
            let out =
                std::env::temp_dir().join(format!("kor-gitleaks-{}.json", std::process::id()));
            let res = run(Command::new(bin).no_window()
                .args([
                    "detect",
                    "--no-git",
                    "--redact",
                    "--no-banner",
                    "--exit-code",
                    "0",
                ])
                .args(["--report-format", "json", "--report-path"])
                .arg(&out)
                .arg("--source")
                .arg(root));
            let text = std::fs::read_to_string(&out).unwrap_or_default();
            let _ = std::fs::remove_file(&out);
            match res.and_then(|()| parse_gitleaks(&text, root)) {
                Ok(f) => {
                    report.notes.push(l.t_args(
                        "analysis-note-external",
                        &kor_i18n::args!("tool" => "gitleaks", "count" => f.len()),
                    ));
                    report.findings.extend(f);
                }
                Err(e) => report.notes.push(l.t_args(
                    "analysis-note-external-failed",
                    &kor_i18n::args!("tool" => "gitleaks", "error" => e),
                )),
            }
        }
        None => report.notes.push(l.t_args(
            "analysis-note-missing",
            &kor_i18n::args!("tool" => "gitleaks"),
        )),
    }
    let Some(bin) = crate::deploy::which("semgrep") else {
        report.notes.push(l.t_args(
            "analysis-note-missing",
            &kor_i18n::args!("tool" => "semgrep"),
        ));
        return;
    };
    let Some(config) = semgrep_config(root, crate::privacy_mode()) else {
        report.notes.push(l.t("analysis-note-semgrep-private"));
        return;
    };
    let res = output(
        Command::new(bin).no_window()
            .args(["scan", "--json", "--quiet", "--metrics", "off", "--config"])
            .arg(&config)
            .arg(root),
    );
    match res.and_then(|s| parse_semgrep(&s, root)) {
        Ok(f) => {
            report.notes.push(l.t_args(
                "analysis-note-external",
                &kor_i18n::args!("tool" => "semgrep", "count" => f.len()),
            ));
            report.findings.extend(f);
        }
        Err(e) => report.notes.push(l.t_args(
            "analysis-note-external-failed",
            &kor_i18n::args!("tool" => "semgrep", "error" => e),
        )),
    }
}

/// The project's own semgrep config, else the registry pack `p/default` — which is a download,
/// so not in privacy/offline mode.
pub fn semgrep_config(root: &Path, private: bool) -> Option<String> {
    for name in [".semgrep.yml", ".semgrep.yaml", ".semgrep"] {
        let p = root.join(name);
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    (!private).then(|| "p/default".to_owned())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    output(cmd).map(drop)
}

/// Stdout of `cmd`, killed after [`EXTERNAL_TIMEOUT`].
fn output(cmd: &mut Command) -> Result<String, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = std::io::Read::read_to_string(o, &mut s);
        }
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if start.elapsed() > EXTERNAL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timeout".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    reader.join().map_err(|_| "reader panicked".to_owned())
}

/// Scanners print absolute or root-relative paths; findings are project-relative.
fn rel(root: &Path, file: &str) -> PathBuf {
    let p = Path::new(file);
    let root_abs = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    p.strip_prefix(&root_abs)
        .or_else(|_| p.strip_prefix(root))
        .map_or_else(|_| p.to_path_buf(), Path::to_path_buf)
}

fn line(v: &Value) -> u32 {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1)
        .max(1)
}

/// gitleaks `--report-format json`: `[{ RuleID, File, StartLine, StartColumn, Description }]`.
/// The secret itself (`Secret`, `Match`) is never copied.
pub fn parse_gitleaks(json: &str, root: &Path) -> Result<Vec<Finding>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("gitleaks: not an array")?;
    Ok(list
        .iter()
        .filter_map(|r| {
            let rule = r["RuleID"].as_str().filter(|s| !s.is_empty())?;
            let file = r["File"].as_str().filter(|s| !s.is_empty())?;
            Some(Finding {
                rule: rule.to_owned(),
                source: "gitleaks".into(),
                severity: Severity::Critical,
                file: rel(root, file),
                line: line(&r["StartLine"]),
                column: r["StartColumn"]
                    .as_u64()
                    .and_then(|c| u32::try_from(c).ok())
                    .filter(|c| *c > 0),
                message: r["Description"].as_str().unwrap_or(rule).to_owned(),
                fix: None,
                cwe: Some("CWE-798".into()),
            })
        })
        .collect())
}

/// semgrep `--json`: `{ results: [{ check_id, path, start: {line, col}, extra: { message,
/// severity, fix, metadata: { cwe } } }] }`.
pub fn parse_semgrep(json: &str, root: &Path) -> Result<Vec<Finding>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = v["results"].as_array().ok_or("semgrep: no results")?;
    Ok(list
        .iter()
        .filter_map(|r| {
            let rule = r["check_id"].as_str().filter(|s| !s.is_empty())?;
            let file = r["path"].as_str().filter(|s| !s.is_empty())?;
            let ex = &r["extra"];
            let cwe = match &ex["metadata"]["cwe"] {
                Value::Array(a) => a.first().and_then(Value::as_str),
                Value::String(s) => Some(s.as_str()),
                _ => None,
            }
            .and_then(|s| s.split(':').next())
            .map(|s| s.trim().to_owned());
            Some(Finding {
                rule: rule.to_owned(),
                source: "semgrep".into(),
                severity: ex["severity"]
                    .as_str()
                    .and_then(Severity::parse)
                    .unwrap_or(Severity::Medium),
                file: rel(root, file),
                line: line(&r["start"]["line"]),
                column: r["start"]["col"]
                    .as_u64()
                    .and_then(|c| u32::try_from(c).ok()),
                message: ex["message"].as_str().unwrap_or(rule).trim().to_owned(),
                fix: ex["fix"].as_str().map(str::to_owned),
                cwe,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitleaks_json_merges_and_dedups_with_kor() {
        let root = Path::new("proj");
        let json = r#"[{"RuleID":"github-pat","File":"proj/src/a.py","StartLine":3,"StartColumn":7,
            "Description":"GitHub PAT","Secret":"REDACTED","Match":"REDACTED"},
            {"RuleID":"","File":"x","StartLine":1}]"#;
        let f = parse_gitleaks(json, root).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].file, PathBuf::from("src/a.py"));
        assert!(!f[0].message.contains("REDACTED"));
        let mut r = Report::new("security");
        r.findings.push(Finding {
            rule: "github-pat".into(),
            source: "kor".into(),
            file: "src/a.py".into(),
            line: 3,
            ..Finding::default()
        });
        r.findings.extend(f);
        r.normalize();
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].source, "kor");
    }

    #[test]
    fn semgrep_json_maps_fields() {
        let json = r#"{"results":[{"check_id":"python.lang.security.audit.eval","path":"app.py",
            "start":{"line":12,"col":5},"extra":{"message":"eval ","severity":"ERROR",
            "metadata":{"cwe":["CWE-95: Improper Neutralization"]}}}],"errors":[]}"#;
        let f = parse_semgrep(json, Path::new(".")).unwrap();
        assert_eq!(f[0].severity, Severity::High);
        assert_eq!(f[0].cwe.as_deref(), Some("CWE-95"));
        assert_eq!(f[0].line, 12);
        assert_eq!(f[0].message, "eval");
    }

    #[test]
    fn semgrep_registry_only_outside_privacy() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(semgrep_config(dir.path(), true), None);
        assert_eq!(
            semgrep_config(dir.path(), false).as_deref(),
            Some("p/default")
        );
        std::fs::write(dir.path().join(".semgrep.yml"), "rules: []").unwrap();
        assert!(semgrep_config(dir.path(), true).is_some());
    }
}
