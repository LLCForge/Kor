//! `kor audit`:
//! - analysis (spec §9, ADR 0045): `--security`, `--compliance=kvkk|gdpr` (and `--licenses`,
//!   ADR 0046) with `--format md|json|sarif`, `--out`, `--sign`, `--explain`;
//! - the audit log (spec §6.7): `list|verify|report|export` over the hash-chained `audit.jsonl`
//!   written by the agent loop (`kor_agent::audit`), and `verify-report` for signed analyses.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use kor_i18n::{Localizer, args};
use kor_runtime::analysis::{self, Format, Report, Severity, compliance::Regime};

use crate::Ctx;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReportFormat {
    Md,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AnalysisFormat {
    Md,
    Json,
    Sarif,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ComplianceArg {
    Kvkk,
    Gdpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MinSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// `kor audit [analysis flags] | kor audit <log subcommand>`.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, arg_required_else_help = true)]
#[allow(clippy::struct_excessive_bools)] // independent command-line flags
pub struct AuditArgs {
    #[command(subcommand)]
    pub command: Option<AuditCmd>,
    /// Security scan: secrets, injection, weak crypto, open endpoints, path traversal, TLS.
    #[arg(long)]
    pub security: bool,
    /// Personal-data processing points for KVKK or GDPR.
    #[arg(long, value_enum, value_name = "kvkk|gdpr")]
    pub compliance: Option<ComplianceArg>,
    /// Dependency licenses (SPDX) against the `[audit.licenses] deny` policy (ADR 0046).
    #[arg(long)]
    pub licenses: bool,
    /// Project root (default: current directory).
    #[arg(long)]
    pub path: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = AnalysisFormat::Md)]
    pub format: AnalysisFormat,
    /// Write the report here instead of standard output.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Record the report hash in the audit log (and write .minisig with a configured key).
    #[arg(long, requires = "out")]
    pub sign: bool,
    /// Only findings at this severity or worse.
    #[arg(long, value_enum)]
    pub min_severity: Option<MinSeverity>,
    /// Do not run gitleaks / semgrep even when installed.
    #[arg(long)]
    pub no_external: bool,
    /// Ask the model to explain and fix the top findings (after the deterministic report).
    #[arg(long)]
    pub explain: bool,
    #[arg(short, long, requires = "explain")]
    pub model: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AuditCmd {
    /// Newest entries first.
    List {
        /// Only this action (`tool`, `task`, `model`, `sandbox_denied`).
        #[arg(long)]
        action: Option<String>,
        /// Only models containing this text.
        #[arg(long)]
        model: Option<String>,
        /// Only targets (files, commands) containing this text.
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=500))]
        limit: u16,
    },
    /// Check the hash chain across every file and against the head.
    Verify,
    /// Compliance report (KVKK/GDPR): providers contacted, files, data sent, masked secrets.
    Report {
        /// Only entries on or after this date (`YYYY-MM-DD`, UTC).
        #[arg(long)]
        since: Option<String>,
        #[arg(long, value_enum, default_value_t = ReportFormat::Md)]
        format: ReportFormat,
    },
    /// Copy every entry (all files, oldest first) as JSON lines; prints the verification result.
    Export {
        /// Destination file; standard output when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Check an analysis report against the audit log (hash) and its .minisig, if any.
    VerifyReport { file: PathBuf },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-audit"))
        .mut_arg("security", |a| a.help(l.t("arg-audit-security")))
        .mut_arg("compliance", |a| a.help(l.t("arg-audit-compliance")))
        .mut_arg("licenses", |a| a.help(l.t("arg-audit-licenses")))
        .mut_arg("path", |a| a.help(l.t("arg-audit-path")))
        .mut_arg("format", |a| a.help(l.t("arg-audit-format")))
        .mut_arg("out", |a| a.help(l.t("arg-audit-out")))
        .mut_arg("sign", |a| a.help(l.t("arg-audit-sign")))
        .mut_arg("min_severity", |a| a.help(l.t("arg-audit-min-severity")))
        .mut_arg("no_external", |a| a.help(l.t("arg-audit-no-external")))
        .mut_arg("explain", |a| a.help(l.t("arg-audit-explain")))
        .mut_subcommand("list", |s| s.about(l.t("cmd-audit-list")))
        .mut_subcommand("verify", |s| s.about(l.t("cmd-audit-verify")))
        .mut_subcommand("report", |s| s.about(l.t("cmd-audit-report")))
        .mut_subcommand("export", |s| s.about(l.t("cmd-audit-export")))
        .mut_subcommand("verify-report", |s| s.about(l.t("cmd-audit-verify-report")))
}

/// `kor audit [flags]` or a log subcommand.
pub fn dispatch(ctx: &Ctx, a: AuditArgs) -> anyhow::Result<bool> {
    match a.command {
        Some(cmd) => run(ctx, cmd),
        None => analyze(ctx, &a),
    }
}

fn verify_report(l: &Localizer, file: &Path) -> bool {
    let shown = kor_tools::display_path(file);
    match analysis::verify_report(file) {
        Ok(v) => {
            if v.ok() {
                println!(
                    "{}",
                    l.t_args(
                        "audit-report-verified",
                        &args!("path" => shown, "sha" => v.sha256.clone(),
                            "at" => v.linked_at.clone().unwrap_or_default())
                    )
                );
            } else {
                eprintln!(
                    "{}",
                    l.t_args(
                        "audit-report-unverified",
                        &args!("path" => shown, "sha" => v.sha256.clone())
                    )
                );
                if !v.chain_ok {
                    eprintln!("{}", l.t("audit-verify-report-chain"));
                }
            }
            match v.signature_ok {
                Some(true) => println!("{}", l.t("audit-report-sig-ok")),
                Some(false) => eprintln!("{}", l.t("audit-report-sig-bad")),
                None => {}
            }
            v.ok()
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            false
        }
    }
}

fn analyze(ctx: &Ctx, a: &AuditArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = match &a.path {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let mut parts: Vec<Report> = Vec::new();
    if a.security {
        let opts = analysis::security::Options {
            no_external: a.no_external,
        };
        parts.push(analysis::security::scan(&root, &opts, l));
    }
    if let Some(c) = a.compliance {
        let regime = match c {
            ComplianceArg::Kvkk => Regime::Kvkk,
            ComplianceArg::Gdpr => Regime::Gdpr,
        };
        parts.push(analysis::compliance::scan(&root, regime, l));
    }
    if a.licenses {
        let deny = ctx
            .config
            .as_ref()
            .map(|c| c.config.audit.licenses.deny.clone())
            .unwrap_or_default();
        let inv = analysis::deps::scan(&root);
        parts.push(analysis::deps::license::licenses_report(&inv, &deny, l));
    }
    if parts.is_empty() {
        eprintln!("{}", l.t("audit-analysis-nothing"));
        return Ok(false);
    }
    let mut report = merge(parts);
    if let Some(m) = a.min_severity {
        report.retain_min(match m {
            MinSeverity::Critical => Severity::Critical,
            MinSeverity::High => Severity::High,
            MinSeverity::Medium => Severity::Medium,
            MinSeverity::Low => Severity::Low,
            MinSeverity::Info => Severity::Info,
        });
    }
    let format = match a.format {
        AnalysisFormat::Md => Format::Markdown,
        AnalysisFormat::Json => Format::Json,
        AnalysisFormat::Sarif => Format::Sarif,
    };
    let text = analysis::render(&report, format, l);
    match &a.out {
        Some(out) => {
            std::fs::write(out, &text)?;
            let c = report.counts();
            eprintln!(
                "{}",
                l.t_args(
                    "audit-report-written",
                    &args!("path" => kor_tools::display_path(out),
                        "count" => report.findings.len(),
                        "critical" => c[0], "high" => c[1], "medium" => c[2])
                )
            );
            if a.sign {
                match analysis::sign_and_link(&report, out) {
                    Ok(s) => {
                        eprintln!(
                            "{}",
                            l.t_args("audit-report-linked", &args!("sha" => s.sha256.clone()))
                        );
                        if let Some(sig) = &s.signature {
                            eprintln!(
                                "{}",
                                l.t_args(
                                    "audit-report-signed",
                                    &args!("path" => kor_tools::display_path(sig))
                                )
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                        return Ok(false);
                    }
                }
            }
        }
        None => std::io::stdout().lock().write_all(text.as_bytes())?,
    }
    if a.explain && !report.findings.is_empty() {
        explain(ctx, a, &root, &report, format);
    }
    Ok(true)
}

/// One report from several scans; findings stay deduplicated.
fn merge(parts: Vec<Report>) -> Report {
    let mut out = Report::new(
        parts
            .iter()
            .map(|r| r.tool.as_str())
            .collect::<Vec<_>>()
            .join("+"),
    );
    for p in parts {
        out.findings.extend(p.findings);
        out.notes.extend(p.notes);
    }
    out.normalize();
    out
}

/// Model prose after the deterministic report; machine formats on stdout stay clean.
fn explain(ctx: &Ctx, a: &AuditArgs, root: &Path, report: &Report, format: Format) {
    let l = &ctx.l;
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("error-generic", &args!("error" => e.to_string()))
            );
            return;
        }
    };
    let (provider, model) = match kor_runtime::oneshot_provider(l, loaded, a.model.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    eprintln!(
        "{}",
        l.t_args(
            "audit-explain-sending",
            &args!("count" => report.findings.len().min(analysis::explain::TOP),
                "provider" => provider.manifest().name.clone())
        )
    );
    let prompt = analysis::explain::prompt(report, root, l.lang());
    let reply = crate::providers::runtime()
        .map_err(|e| e.to_string())
        .and_then(|rt| {
            rt.block_on(kor_agent::gitflow::ask(&provider, &model, prompt, 4_000))
                .map_err(|e| format!("{e:?}"))
        });
    match reply {
        Ok(text) => {
            let body = format!("\n## {}\n\n{}\n", l.t("audit-explain-title"), text.trim());
            if a.out.is_none() && format != Format::Markdown {
                eprint!("{body}");
            } else {
                print!("{body}");
            }
        }
        Err(e) => eprintln!("{}", l.t_args("error-generic", &args!("error" => e))),
    }
}

/// Print the verification result; `false` when the chain or the head check failed.
fn print_verify(l: &Localizer, path: &std::path::Path, r: &kor_agent::audit::Verify) -> bool {
    let shown = kor_tools::display_path(path);
    match (r.broken, &r.file) {
        (None, _) => eprintln!(
            "{}",
            l.t_args("audit-ok", &args!("count" => r.entries, "path" => shown))
        ),
        (Some((line, reason)), file) => {
            let at = file
                .as_deref()
                .map_or(shown, |f| kor_tools::display_path(&path.with_file_name(f)));
            eprintln!(
                "{}",
                l.t_args(
                    "audit-broken",
                    &args!("line" => line, "reason" => reason, "path" => at)
                )
            );
        }
    }
    if r.head != "ok" {
        eprintln!("{}", l.t(&format!("audit-head-state-{}", r.head)));
    }
    if r.trimmed {
        eprintln!("{}", l.t("audit-report-trimmed"));
    }
    r.ok()
}

#[allow(clippy::too_many_lines)]
pub fn run(ctx: &Ctx, cmd: AuditCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let path =
        kor_runtime::secure::audit_path().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    let shown = kor_tools::display_path(&path);
    match cmd {
        AuditCmd::List {
            action,
            model,
            file,
            limit,
        } => {
            let q = kor_agent::audit::Query {
                action,
                model,
                file,
                offset: 0,
                limit: usize::from(limit),
            };
            let page = kor_agent::audit::read(&path, &q)?;
            if page.total == 0 {
                println!("{}", l.t_args("audit-empty", &args!("path" => shown)));
                return Ok(true);
            }
            for e in &page.entries {
                let s = |k: &str| e[k].as_str().unwrap_or_default().to_owned();
                let n = |k: &str| e[k].as_u64().unwrap_or_default();
                let what = match s("action").as_str() {
                    "tool" => format!("{} {} {}", s("tool"), s("decision"), s("target")),
                    "model" => {
                        let cost = e["cost_usd"].as_str().map_or(String::new(), |c| {
                            let est = if e["cost_estimated"].as_bool() == Some(true) {
                                "≈"
                            } else {
                                ""
                            };
                            format!(" {est}${c}")
                        });
                        format!("{} → {} token{cost}", n("input"), n("output"))
                    }
                    "task" => s("text").lines().next().unwrap_or_default().to_owned(),
                    _ => s("target"),
                };
                println!(
                    "  {}  {:<5} {:<24} {what}",
                    s("ts"),
                    s("action"),
                    s("model")
                );
            }
            Ok(true)
        }
        AuditCmd::Verify => {
            let r = kor_agent::audit::verify(&path)?;
            Ok(print_verify(l, &path, &r))
        }
        AuditCmd::VerifyReport { file } => Ok(verify_report(l, &file)),
        AuditCmd::Report { since, format } => {
            if let Some(s) = &since
                && !kor_runtime::secure::valid_since(s)
            {
                eprintln!(
                    "{}",
                    l.t_args("audit-bad-since", &args!("value" => s.clone()))
                );
                return Ok(false);
            }
            let r = kor_runtime::secure::audit_report(l, since.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;
            match format {
                ReportFormat::Md => print!("{}", kor_runtime::secure::report_markdown(&r, l)),
                ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&r)?),
            }
            Ok(true)
        }
        AuditCmd::Export { out } => {
            let r = kor_agent::audit::verify(&path)?;
            let mut lines = Vec::new();
            for f in kor_agent::audit::files(&path)? {
                let text = std::fs::read_to_string(&f)?;
                lines.extend(
                    text.lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(str::to_owned),
                );
            }
            let body: String = lines.iter().flat_map(|l| [l.as_str(), "\n"]).collect();
            match &out {
                Some(p) => {
                    std::fs::write(p, body)?;
                    eprintln!(
                        "{}",
                        l.t_args(
                            "audit-exported",
                            &args!("count" => lines.len(), "path" => kor_tools::display_path(p))
                        )
                    );
                }
                None => std::io::stdout().lock().write_all(body.as_bytes())?,
            }
            Ok(print_verify(l, &path, &r))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Debug, Subcommand)]
    enum Top {
        Audit(AuditArgs),
    }

    fn parse(args: &[&str]) -> Result<AuditArgs, clap::Error> {
        let mut v = vec!["kor", "audit"];
        v.extend_from_slice(args);
        Cli::try_parse_from(v).map(|c| match c.cmd {
            Top::Audit(a) => a,
        })
    }

    #[test]
    fn analysis_flags_and_log_subcommands_parse() {
        let a = parse(&["--security", "--compliance", "kvkk", "--format", "sarif"]).unwrap();
        assert!(a.security && a.command.is_none());
        assert_eq!(a.compliance, Some(ComplianceArg::Kvkk));
        assert_eq!(a.format, AnalysisFormat::Sarif);
        assert!(parse(&["--compliance=gdpr"]).is_ok());
        assert!(matches!(
            parse(&["list", "--limit", "5"]).unwrap().command,
            Some(AuditCmd::List { limit: 5, .. })
        ));
        assert!(matches!(
            parse(&["verify-report", "r.sarif"]).unwrap().command,
            Some(AuditCmd::VerifyReport { .. })
        ));
        assert!(matches!(
            parse(&["verify"]).unwrap().command,
            Some(AuditCmd::Verify)
        ));
        // --sign needs --out; analysis flags do not mix with log subcommands.
        assert!(parse(&["--security", "--sign"]).is_err());
        assert!(parse(&["--security", "--out", "r.md", "--sign"]).is_ok());
        assert!(parse(&["--security", "list"]).is_err());
        assert!(parse(&["--compliance", "ccpa"]).is_err());
    }

    #[test]
    fn localize_names_real_args() {
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let c = Cli::command().mut_subcommand("audit", |c| localize(c, &l));
        c.debug_assert();
    }
}
