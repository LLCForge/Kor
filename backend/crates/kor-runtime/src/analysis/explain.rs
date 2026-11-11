//! `--explain`: the prompt that asks a model to explain and fix the top findings. The
//! deterministic report is printed first; this only adds prose (spec §9 principle).

use std::fmt::Write as _;
use std::path::Path;

use super::{Report, slash};

/// Findings sent to the model, worst first.
pub const TOP: usize = 10;
/// Source lines around each finding.
const CONTEXT: usize = 3;

/// Prompt for the top findings with a few lines of code around each. Secrets are masked and
/// secret findings carry no code at all.
pub fn prompt(report: &Report, root: &Path, lang: kor_i18n::Lang) -> String {
    let mut out = format!(
        "You are a security reviewer. For each finding below (from deterministic scanners), \
         explain in 1-3 sentences why it matters in this code and give a concrete fix as a short \
         code snippet. If a finding is a false positive, say so and why. Keep the numbering. \
         Answer in language `{}`.\n\n",
        lang.code()
    );
    for (i, f) in report.findings.iter().take(TOP).enumerate() {
        let _ = writeln!(
            out,
            "{}. [{}] {} {} ({}{}) — {}",
            i + 1,
            f.severity.as_str(),
            f.location(),
            f.rule,
            f.source,
            f.cwe
                .as_deref()
                .map(|c| format!(", {c}"))
                .unwrap_or_default(),
            f.message
        );
        let secret = f.cwe.as_deref() == Some("CWE-798");
        if secret {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(&f.file)) {
            let line = f.line as usize;
            let lo = line.saturating_sub(CONTEXT + 1);
            let snippet: Vec<String> = text
                .lines()
                .enumerate()
                .skip(lo)
                .take(CONTEXT * 2 + 1)
                .map(|(n, l)| format!("{:>5} | {}", n + 1, kor_tools::redact_secrets(l)))
                .collect();
            let _ = writeln!(
                out,
                "```{}\n{}\n```",
                Path::new(&slash(&f.file))
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                snippet.join("\n")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Finding, Severity};
    use super::*;

    #[test]
    fn secret_findings_carry_no_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.py"),
            "x = 1\nKEY = 'ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\nos.system(cmd)\n",
        )
        .unwrap();
        let mut r = Report::new("security");
        for (rule, line, cwe) in [("github-pat", 2, "CWE-798"), ("kor.os", 3, "CWE-78")] {
            r.findings.push(Finding {
                rule: rule.into(),
                source: "kor".into(),
                severity: Severity::High,
                file: "a.py".into(),
                line,
                message: "m".into(),
                cwe: Some(cwe.into()),
                ..Finding::default()
            });
        }
        let p = prompt(&r, dir.path(), kor_i18n::Lang::Tr);
        assert!(!p.contains("ghp_a"));
        assert!(p.contains("os.system(cmd)"));
        assert!(p.contains("`tr`"));
    }
}
