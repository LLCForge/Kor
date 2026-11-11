//! `kor explain <file> [--level junior|normal|expert]` (§9, §12.2, ADR 0047): code archaeology
//! from git, blame, tree-sitter and the project graph (`kor_runtime::analysis::metrics::explain`),
//! then an optional plain-language explanation by the configured model. `--offline` (and any
//! model error) keeps it to the deterministic facts.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use kor_i18n::{Lang, Localizer, args};
use kor_runtime::analysis::metrics::explain::{self, Archaeology, Level};
use kor_runtime::analysis::metrics::stats::fmt_day;

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LevelArg {
    Junior,
    Normal,
    Expert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    Text,
    Json,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// File to explain (project-relative or absolute).
    pub file: PathBuf,
    #[arg(long, value_enum, default_value = "normal")]
    pub level: LevelArg,
    /// Facts only: do not ask a model.
    #[arg(long)]
    pub offline: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
    #[arg(short, long)]
    pub model: Option<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-explain"))
}

fn level(a: LevelArg) -> Level {
    match a {
        LevelArg::Junior => Level::Junior,
        LevelArg::Normal => Level::Normal,
        LevelArg::Expert => Level::Expert,
    }
}

pub(crate) fn language(l: &Localizer) -> &'static str {
    match l.lang() {
        Lang::Tr => "Turkish",
        Lang::Fr => "French",
        Lang::En => "English",
    }
}

pub fn run(ctx: &Ctx, a: &ExplainArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let file = a.file.to_string_lossy().into_owned();
    let arch = match explain::archaeology(&root, &file, None) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}", l.t_args("metrics-error", &args!("error" => e)));
            return Ok(false);
        }
    };
    let prose = if a.offline {
        None
    } else {
        crate::pr_cmd::model(ctx, a.model.as_deref()).map(|(provider, id)| {
            let source = std::fs::read_to_string(root.join(&arch.file)).unwrap_or_default();
            let prompt = explain::prompt(&arch, level(a.level), language(l), &source);
            crate::providers::runtime()
                .map_err(|e| e.to_string())
                .and_then(|rt| {
                    rt.block_on(kor_agent::gitflow::ask(&provider, &id, prompt, 1500))
                        .map_err(|e| e.to_string())
                })
        })
    };
    if a.format == OutFormat::Json {
        let v = serde_json::json!({
            "facts": arch,
            "explanation": prose.as_ref().and_then(|p| p.as_ref().ok()),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(true);
    }
    print_facts(l, &arch);
    match prose {
        None => println!("\n{}", l.t("explain-no-model")),
        Some(Ok(text)) => {
            let lv = format!("{:?}", a.level).to_lowercase();
            println!(
                "\n{}\n{}",
                l.t_args("explain-model-head", &args!("level" => lv)),
                text.trim()
            );
        }
        Some(Err(e)) => println!(
            "\n{}",
            l.t_args("explain-model-error", &args!("error" => e))
        ),
    }
    Ok(true)
}

#[allow(clippy::too_many_lines)] // one printed section per fact group; splitting only scatters it
pub(crate) fn print_facts(l: &Localizer, a: &Archaeology) {
    let (fns, max) = a
        .stats
        .as_ref()
        .map_or((0, 0), |s| (s.functions.len(), s.max_complexity()));
    let lines = a.stats.as_ref().map_or(0, |s| s.lines);
    println!(
        "{}",
        l.t_args(
            "explain-head",
            &args!("file" => a.file.clone(), "lang" => a.lang.clone().unwrap_or_else(|| "-".into()),
                "lines" => lines, "functions" => fns, "max" => max)
        )
    );
    if a.tracked {
        let (first, last) = (a.first.as_ref(), a.last.as_ref());
        println!(
            "{}",
            l.t_args(
                "explain-history",
                &args!("commits" => a.commits,
                    "first" => first.map(|c| fmt_day(c.time)).unwrap_or_default(),
                    "age" => a.age_days.unwrap_or(0),
                    "last" => last.map(|c| fmt_day(c.time)).unwrap_or_default(),
                    "since" => a.last_change_days.unwrap_or(0),
                    "c90" => a.churn_90d, "c365" => a.churn_365d)
            )
        );
        println!("{}", l.t("explain-authors"));
        for x in &a.authors {
            println!(
                "  {}",
                l.t_args(
                    "explain-author",
                    &args!("name" => x.name.clone(), "email" => x.email.clone(),
                        "share" => format!("{:.1}", x.share), "commits" => x.commits)
                )
            );
        }
        if !a.renames.is_empty() {
            println!("{}", l.t("explain-renames"));
            for r in &a.renames {
                println!(
                    "  {}",
                    l.t_args(
                        "explain-rename",
                        &args!("from" => r.from.clone(), "to" => r.to.clone(),
                            "date" => fmt_day(r.commit.time), "hash" => r.commit.hash.clone())
                    )
                );
            }
        }
        if a.refactors.is_empty() {
            println!("{}", l.t("explain-refactors-none"));
        } else {
            println!(
                "{}",
                l.t_args("explain-refactors", &args!("count" => a.refactors.len()))
            );
            for c in a.refactors.iter().take(10) {
                println!("  {} {} {}", fmt_day(c.time), c.hash, c.subject);
            }
        }
    } else {
        println!("{}", l.t("explain-untracked"));
    }
    if a.danger.is_empty() {
        println!("{}", l.t("explain-danger-none"));
    } else {
        println!("{}", l.t("explain-danger"));
        for z in &a.danger {
            println!(
                "  {}",
                l.t_args(
                    "explain-zone",
                    &args!("name" => z.name.clone(), "line" => z.line, "end" => z.end, "cc" => z.complexity,
                        "commits" => z.commits, "score" => z.score, "recent" => z.recent_lines)
                )
            );
        }
    }
    if let Some(g) = &a.graph {
        println!(
            "{}",
            l.t_args(
                "explain-graph",
                &args!("community" => g.community.clone().unwrap_or_else(|| "-".into()),
                    "count" => g.dependents_total)
            )
        );
        for d in &g.dependents {
            println!("  ← {d}");
        }
        if !g.co_changes.is_empty() {
            let list: Vec<String> = g
                .co_changes
                .iter()
                .map(|(f, r)| format!("{f} ({:.0}%)", r * 100.0))
                .collect();
            println!(
                "{}",
                l.t_args("explain-cochange", &args!("list" => list.join(", ")))
            );
        }
    }
    println!(
        "{}",
        l.t_args("explain-sources", &args!("list" => a.sources.join("; ")))
    );
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{ExplainArgs, LevelArg, OutFormat};

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        a: ExplainArgs,
    }

    #[test]
    fn parses() {
        let c =
            Cli::try_parse_from(["explain", "src/a.rs", "--level", "junior", "--offline"]).unwrap();
        assert_eq!(c.a.level, LevelArg::Junior);
        assert!(c.a.offline && c.a.format == OutFormat::Text);
        assert!(Cli::try_parse_from(["explain", "a.rs", "--level", "guru"]).is_err());
        assert!(Cli::try_parse_from(["explain"]).is_err());
    }
}
