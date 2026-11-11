//! Vault backend choice and the KVKK/GDPR compliance report (spec §6.6, §6.7; ADR 0036).
//! Shared by `kor vault` / `kor audit` and the desktop Security panel.

use std::fmt::Write as _;
use std::path::PathBuf;

use kor_agent::audit::{self, Report};
use kor_i18n::{Localizer, args};
use kor_secrets::{KeychainStore, SecretError, SecretStore, SecretString};

/// The encrypted vault file under the Kor home (`[vault] backend = "file"`).
pub const VAULT_FILE: &str = "vault.enc";
/// Passphrase of the vault file for sessions that cannot prompt (agent, desktop).
pub const PASSPHRASE_ENV: &str = "KOR_VAULT_PASSPHRASE";

pub fn vault_file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join(VAULT_FILE))
}

pub fn audit_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join(audit::FILE))
}

/// The configured vault: the OS keychain, or the encrypted file opened with `passphrase`
/// (else [`PASSPHRASE_ENV`]). A file vault without a passphrase is an error ("locked").
pub fn vault_store(
    cfg: &kor_config::Vault,
    passphrase: Option<SecretString>,
) -> Result<Box<dyn SecretStore>, SecretError> {
    match cfg.backend {
        kor_config::VaultBackend::Keychain => Ok(Box::new(KeychainStore)),
        kor_config::VaultBackend::File => {
            let pass = passphrase
                .or_else(|| {
                    std::env::var(PASSPHRASE_ENV)
                        .ok()
                        .filter(|p| !p.is_empty())
                        .map(SecretString::from)
                })
                .ok_or_else(|| SecretError::VaultFile(format!("locked; set {PASSPHRASE_ENV}")))?;
            let path =
                vault_file().ok_or_else(|| SecretError::VaultFile("no data directory".into()))?;
            Ok(Box::new(kor_secrets::filestore::FileStore::open(
                &path, &pass,
            )?))
        }
    }
}

/// The report over the log, with each provider's place and data policy from the catalog
/// (missing values stay `None`, shown as "unknown").
pub fn audit_report(l: &Localizer, since: Option<&str>) -> Result<Report, String> {
    let path = audit_path().ok_or("no data directory")?;
    let mut r = audit::report(&path, since)
        .map_err(|e| format!("{}: {e}", kor_tools::display_path(&path)))?;
    let reg = crate::registry(l, &mut Vec::new()).ok();
    for p in &mut r.providers {
        if let Some(m) = reg.as_ref().and_then(|g| g.get(&p.provider)) {
            p.local = Some(crate::is_local_provider(m));
            p.jurisdiction.clone_from(&m.privacy.jurisdiction);
            p.training_on_data = m.privacy.training_on_data;
            p.retention = Some(m.privacy.retention.clone())
                .filter(|r| !r.trim().is_empty() && !r.eq_ignore_ascii_case("unknown"));
        }
    }
    Ok(r)
}

/// `since` as given to `--since`: a `YYYY-MM-DD` date or an RFC 3339 time.
pub fn valid_since(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
        || chrono::DateTime::parse_from_rfc3339(s).is_ok()
}

/// Table cells must not break the Markdown table.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// The report as Markdown in the interface language (no prompts or file contents in it).
#[allow(clippy::too_many_lines)]
pub fn report_markdown(r: &Report, l: &Localizer) -> String {
    let unknown = l.t("audit-report-unknown");
    let yes_no = |b: Option<bool>| match b {
        Some(true) => l.t("audit-report-yes"),
        Some(false) => l.t("audit-report-no"),
        None => unknown.clone(),
    };
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", l.t("audit-report-title"));
    let _ = writeln!(out, "{}\n", l.t("audit-report-intro"));
    let period = match &r.since {
        Some(d) => l.t_args("audit-report-since", &args!("date" => d.clone())),
        None => l.t("audit-report-all"),
    };
    let _ = writeln!(
        out,
        "- {}: {period} ({} – {})",
        l.t("audit-report-period"),
        r.first.as_deref().unwrap_or("—"),
        r.last.as_deref().unwrap_or("—")
    );
    let _ = writeln!(
        out,
        "- {}",
        l.t_args(
            "audit-report-entries",
            &args!("count" => r.entries, "tasks" => r.tasks)
        )
    );

    let _ = writeln!(out, "\n## {}\n", l.t("audit-report-integrity"));
    let v = &r.verify;
    let chain = match v.broken {
        None => l.t_args(
            "audit-report-chain-ok",
            &args!("entries" => v.entries, "files" => v.files),
        ),
        Some((line, why)) => l.t_args(
            "audit-report-chain-broken",
            &args!("file" => v.file.clone().unwrap_or_default(), "line" => line, "reason" => why),
        ),
    };
    let _ = writeln!(out, "- {chain}");
    let _ = writeln!(out, "- {}", l.t(&format!("audit-head-state-{}", v.head)));
    if v.trimmed {
        let _ = writeln!(out, "- {}", l.t("audit-report-trimmed"));
    }

    let _ = writeln!(out, "\n## {}\n", l.t("audit-report-providers"));
    if r.providers.is_empty() {
        let _ = writeln!(out, "{}", l.t("audit-report-none-models"));
    } else {
        let cols = [
            "audit-report-col-provider",
            "audit-report-col-where",
            "audit-report-col-region",
            "audit-report-col-training",
            "audit-report-col-retention",
            "audit-report-col-calls",
            "audit-report-col-sent",
            "audit-report-col-tokens",
        ];
        let head: Vec<String> = cols.iter().map(|c| l.t(c)).collect();
        let _ = writeln!(out, "| {} |", head.join(" | "));
        let _ = writeln!(out, "|{}", "---|".repeat(cols.len()));
        let (mut off_bytes, mut off_calls) = (0, 0);
        for p in &r.providers {
            let place = match p.local {
                Some(true) => l.t("audit-report-local"),
                Some(false) => l.t("audit-report-remote"),
                None => unknown.clone(),
            };
            if p.local != Some(true) {
                off_bytes += p.bytes;
                off_calls += p.calls;
            }
            let _ = writeln!(
                out,
                "| {} | {place} | {} | {} | {} | {} | {} | {} / {} |",
                cell(&p.provider),
                cell(p.jurisdiction.as_deref().unwrap_or(&unknown)),
                yes_no(p.training_on_data),
                cell(p.retention.as_deref().unwrap_or(&unknown)),
                p.calls,
                p.bytes,
                p.input,
                p.output
            );
        }
        let _ = writeln!(
            out,
            "\n{}",
            l.t_args(
                "audit-report-offmachine",
                &args!("bytes" => off_bytes, "calls" => off_calls)
            )
        );

        let _ = writeln!(out, "\n### {}\n", l.t("audit-report-models"));
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |\n|---|---|---|---|",
            l.t("audit-report-col-model"),
            l.t("audit-report-col-calls"),
            l.t("audit-report-col-tokens"),
            l.t("audit-report-col-cost")
        );
        for m in &r.models {
            let mut cost = if m.calls > m.cost_unknown {
                format!(
                    "{}${:.4}",
                    if m.cost_estimated { "≈" } else { "" },
                    m.cost_usd
                )
            } else {
                unknown.clone()
            };
            if m.cost_unknown > 0 && m.calls > m.cost_unknown {
                let _ = write!(
                    cost,
                    " ({})",
                    l.t_args(
                        "audit-report-cost-unknown",
                        &args!("count" => m.cost_unknown)
                    )
                );
            }
            let _ = writeln!(
                out,
                "| {} | {} | {} / {} | {cost} |",
                cell(&m.model),
                m.calls,
                m.input,
                m.output
            );
        }
    }

    let _ = writeln!(out, "\n## {}\n", l.t("audit-report-files"));
    for (key, count, list) in [
        ("audit-report-files-read", r.reads, &r.files_read),
        ("audit-report-files-written", r.writes, &r.files_written),
    ] {
        let _ = writeln!(
            out,
            "- {}",
            l.t_args(key, &args!("count" => count, "distinct" => list.len()))
        );
        for f in list {
            let _ = writeln!(out, "  - `{}`", f.replace('`', "'"));
        }
        if list.len() >= audit::REPORT_FILES {
            let _ = writeln!(
                out,
                "  - {}",
                l.t_args(
                    "audit-report-files-more",
                    &args!("max" => audit::REPORT_FILES)
                )
            );
        }
    }

    let _ = writeln!(out, "\n## {}\n", l.t("audit-report-actions"));
    for line in [
        l.t_args("audit-report-commands", &args!("count" => r.commands)),
        l.t_args("audit-report-network", &args!("count" => r.network_tools)),
        l.t_args(
            "audit-report-approvals",
            &args!("approved" => r.approved, "rejected" => r.rejected, "denied" => r.denied),
        ),
        l.t_args("audit-report-masked", &args!("count" => r.masked)),
        l.t_args("audit-report-sandbox", &args!("count" => r.sandbox_denied)),
    ] {
        let _ = writeln!(out, "- {line}");
    }

    let _ = writeln!(out, "\n## {}\n", l.t("audit-report-retention"));
    let t = &r.retention;
    let _ = writeln!(
        out,
        "- {}",
        l.t_args(
            "audit-report-retention-files",
            &args!("files" => t.files, "bytes" => t.bytes, "rotate" => t.rotate_bytes)
        )
    );
    if let Some(ts) = &t.oldest {
        let _ = writeln!(
            out,
            "- {}",
            l.t_args("audit-report-retention-oldest", &args!("ts" => ts.clone()))
        );
    }
    let _ = writeln!(out, "- {}", l.t("audit-report-retention-policy"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn markdown_report_shows_counts_and_unknowns_never_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(audit::FILE);
        audit::append(&p, "task", json!({"text": "gizli istem metni"})).unwrap();
        audit::append(
            &p,
            "model",
            json!({"model": "acme/m|x", "provider": "acme", "input": 5, "output": 7,
                   "bytes": 900, "masked": 1, "cost_usd": "0.002000", "cost_estimated": true}),
        )
        .unwrap();
        audit::append(
            &p,
            "tool",
            json!({"tool": "Read", "target": "src/a.rs", "decision": "allow"}),
        )
        .unwrap();
        let r = audit::report(&p, None).unwrap();
        for lang in [kor_i18n::Lang::Tr, kor_i18n::Lang::En, kor_i18n::Lang::Fr] {
            let md = report_markdown(&r, &Localizer::new(lang));
            assert!(!md.contains("gizli istem"), "{md}");
            assert!(md.contains("≈$0.0020"), "{md}");
            assert!(md.contains("acme/m\\|x"), "{md}");
            assert!(md.contains("`src/a.rs`"), "{md}");
            assert!(!md.contains("audit-report-"), "missing translation: {md}");
            assert!(!md.contains("audit-head-"), "missing translation: {md}");
        }
        let tr = report_markdown(&r, &Localizer::new(kor_i18n::Lang::Tr));
        assert!(tr.contains("bilinmiyor"), "{tr}");
        assert!(valid_since("2026-01-31") && valid_since("2026-01-31T10:00:00Z"));
        assert!(!valid_since("31.01.2026") && !valid_since("2026-13-01"));
    }
}
