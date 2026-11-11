//! `kor deps check|list` (spec §9, ADR 0046); `kor audit --licenses` lives in `audit_cmd`. The
//! analysis is `kor_runtime::analysis::deps`; this module picks the root and the output.

use std::io::Write as _;
use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_runtime::analysis::deps::{self, check, license};
use kor_runtime::analysis::{self, Format};

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DepsFormat {
    Md,
    Json,
    Sarif,
}

#[derive(Debug, clap::Subcommand)]
pub enum DepsCmd {
    /// Known vulnerabilities (osv.dev), unused and duplicate-purpose packages, upgrades.
    Check {
        #[arg(long)]
        offline: bool,
        #[arg(long, value_enum, default_value_t = DepsFormat::Md)]
        format: DepsFormat,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Every dependency with version, license and the `file:line` that declares it.
    List {
        #[arg(long)]
        json: bool,
    },
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-deps"))
        .mut_subcommand("check", |s| {
            s.about(l.t("cmd-deps-check"))
                .mut_arg("offline", |a| a.help(l.t("arg-deps-offline")))
                .mut_arg("format", |a| a.help(l.t("arg-deps-format")))
                .mut_arg("out", |a| a.help(l.t("arg-deps-out")))
        })
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-deps-list"))
                .mut_arg("json", |a| a.help(l.t("arg-deps-list-json")))
        })
}

pub fn run(ctx: &Ctx, cmd: DepsCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = std::env::current_dir()?;
    match cmd {
        DepsCmd::List { json } => {
            let inv = deps::scan(&root);
            let rows = license::classified(&inv);
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(true);
            }
            if rows.is_empty() {
                println!("{}", l.t("deps-list-empty"));
            }
            for r in &rows {
                let p = r.package;
                let kind = if p.dev {
                    l.t("deps-dev")
                } else if p.direct {
                    l.t("deps-direct")
                } else {
                    l.t("deps-transitive")
                };
                println!(
                    "{:<10} {} {}  [{kind}]  {} ({})  {}:{}",
                    p.ecosystem.osv(),
                    p.name,
                    p.version,
                    p.license.clone().unwrap_or_else(|| "-".into()),
                    l.t(&format!("deps-class-{}", r.class.as_str())),
                    p.file.to_string_lossy().replace('\\', "/"),
                    p.line,
                );
            }
            for (file, e) in &inv.errors {
                eprintln!(
                    "{}",
                    l.t_args(
                        "deps-parse-failed",
                        &args!("file" => file.to_string_lossy().replace('\\', "/"), "error" => e.clone())
                    )
                );
            }
            Ok(true)
        }
        DepsCmd::Check {
            offline,
            format,
            out,
        } => {
            let inv = deps::scan(&root);
            let opts = check::Options {
                offline,
                endpoints: check::Endpoints::default(),
                cache_dir: deps::cache_dir(),
            };
            let rt = crate::providers::runtime()?;
            let report = rt.block_on(check::check_report(&root, &inv, &opts, l));
            let format = match format {
                DepsFormat::Md => Format::Markdown,
                DepsFormat::Json => Format::Json,
                DepsFormat::Sarif => Format::Sarif,
            };
            let text = analysis::render(&report, format, l);
            match &out {
                Some(path) => {
                    std::fs::write(path, &text)?;
                    let c = report.counts();
                    eprintln!(
                        "{}",
                        l.t_args(
                            "audit-report-written",
                            &args!("path" => kor_tools::display_path(path),
                                "count" => report.findings.len(),
                                "critical" => c[0], "high" => c[1], "medium" => c[2])
                        )
                    );
                }
                None => std::io::stdout().lock().write_all(text.as_bytes())?,
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Sub,
    }

    #[derive(clap::Subcommand)]
    enum Sub {
        Deps {
            #[command(subcommand)]
            command: DepsCmd,
        },
    }

    #[test]
    fn parses_check_and_list() {
        let Sub::Deps {
            command:
                DepsCmd::Check {
                    offline,
                    format,
                    out,
                },
        } = Cli::parse_from([
            "kor",
            "deps",
            "check",
            "--offline",
            "--format",
            "sarif",
            "--out",
            "r.sarif",
        ])
        .cmd
        else {
            panic!("check");
        };
        assert!(offline);
        assert_eq!(format, DepsFormat::Sarif);
        assert_eq!(out.as_deref(), Some(std::path::Path::new("r.sarif")));
        assert!(matches!(
            Cli::parse_from(["kor", "deps", "list", "--json"]).cmd,
            Sub::Deps {
                command: DepsCmd::List { json: true }
            }
        ));
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let c = localize(
            <DepsCmd as clap::Subcommand>::augment_subcommands(clap::Command::new("deps")),
            &l,
        );
        assert_eq!(
            c.get_about().map(ToString::to_string),
            Some(l.t("cmd-deps"))
        );
    }
}
