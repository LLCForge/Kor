//! `kor dead-code [--prune]` (§15.14, ADR 0047). Finding and pruning live in
//! `kor_runtime::analysis::metrics::deadcode`; `--prune` shows the verified branch and asks
//! before anything is pushed or a pull request is opened.

use clap::{Args, ValueEnum};
use kor_i18n::{Localizer, args};
use kor_runtime::analysis::metrics::{self, deadcode};
use kor_runtime::analysis::{self, Format};
use kor_runtime::forge;

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    Text,
    Json,
    Sarif,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // independent command-line flags
pub struct DeadCodeArgs {
    /// Delete the high-confidence items on a new branch, verify, and open a PR after confirmation.
    #[arg(long)]
    pub prune: bool,
    /// With --prune: also remove medium-confidence items (recently changed files).
    #[arg(long, requires = "prune")]
    pub include_medium: bool,
    /// With --prune: compiler / type-checker command (detected when omitted).
    #[arg(long, requires = "prune")]
    pub check_cmd: Option<String>,
    /// With --prune: test command (`[orchestrator] test_command` or detected when omitted).
    #[arg(long, requires = "prune")]
    pub test_cmd: Option<String>,
    /// With --prune: push and open the PR without asking.
    #[arg(long, short, requires = "prune")]
    pub yes: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutFormat,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-dead-code"))
}

fn conf(l: &Localizer, c: deadcode::Confidence) -> String {
    l.t(match c {
        deadcode::Confidence::High => "deadcode-conf-high",
        deadcode::Confidence::Medium => "deadcode-conf-medium",
        deadcode::Confidence::Ambiguous => "deadcode-conf-ambiguous",
    })
}

pub(crate) fn why(l: &Localizer, d: &deadcode::DeadItem) -> String {
    match (&d.reason, d.unchanged_days) {
        (deadcode::Reason::Unreachable, Some(days)) => {
            l.t_args("deadcode-why-stale", &args!("days" => days))
        }
        (deadcode::Reason::Unreachable, None) => l.t("deadcode-why-unreachable"),
        (deadcode::Reason::NameElsewhere { count }, _) => {
            l.t_args("deadcode-why-name", &args!("count" => *count))
        }
        (deadcode::Reason::Reflection, _) => l.t("deadcode-why-reflection"),
    }
}

/// A plan note: a message id (`prune-…`) or raw error text.
pub(crate) fn note(l: &Localizer, n: &str) -> String {
    if n.starts_with("prune-") {
        l.t(&format!("deadcode-{n}"))
    } else {
        n.to_owned()
    }
}

fn item(l: &Localizer, d: &deadcode::DeadItem) -> String {
    l.t_args(
        "deadcode-item",
        &args!("conf" => conf(l, d.confidence), "kind" => d.kind.clone(), "name" => d.name.clone(),
            "file" => d.file.clone(), "line" => d.line, "end" => d.end, "why" => why(l, d)),
    )
}

pub fn run(ctx: &Ctx, a: &DeadCodeArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let graph = match metrics::graph(&root) {
        Ok(graph) => graph,
        Err(e) => {
            eprintln!("{}", l.t_args("metrics-error", &args!("error" => e)));
            return Ok(false);
        }
    };
    let report = deadcode::find(&root, &graph);
    if !a.prune {
        match a.format {
            OutFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            OutFormat::Sarif => print!(
                "{}",
                analysis::render(&deadcode::report(&report, l), Format::Sarif, l)
            ),
            OutFormat::Text => print_report(l, &report),
        }
        return Ok(true);
    }
    print_report(l, &report);
    let Ok(loaded) = &ctx.config else {
        eprintln!("{}", l.t_args("metrics-error", &args!("error" => "config")));
        return Ok(false);
    };
    let rt = crate::providers::runtime()?;
    let opts = deadcode::PruneOptions {
        include_medium: a.include_medium,
        check: a.check_cmd.clone(),
        test: a.test_cmd.clone(),
    };
    println!("\n{}", l.t("deadcode-prune-running"));
    let plan = match rt.block_on(deadcode::prune(&root, &report, &opts, &loaded.config, l)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", l.t_args("metrics-error", &args!("error" => e)));
            return Ok(false);
        }
    };
    print_plan(l, &plan);
    if !plan.ready {
        return Ok(false);
    }
    let question = l.t_args(
        "deadcode-prune-confirm",
        &args!("branch" => plan.branch.clone(), "base" => plan.base.clone()),
    );
    if !crate::shell_cmd::confirm(l, &question, a.yes) {
        println!(
            "{}",
            l.t_args(
                "deadcode-prune-kept",
                &args!("branch" => plan.branch.clone())
            )
        );
        return Ok(true);
    }
    match rt.block_on(deadcode::publish(&root, &plan)) {
        Ok(pr) => {
            println!("{}", l.t_args("deadcode-prune-pr", &args!("url" => pr.url)));
            Ok(true)
        }
        Err(e) => {
            eprintln!("{}", forge::error_text(l, &e));
            println!(
                "{}",
                l.t_args(
                    "deadcode-prune-kept",
                    &args!("branch" => plan.branch.clone())
                )
            );
            Ok(false)
        }
    }
}

fn print_report(l: &Localizer, r: &deadcode::DeadReport) {
    println!(
        "{}",
        l.t_args(
            "deadcode-head",
            &args!("symbols" => r.symbols, "entries" => r.entries)
        )
    );
    let rules: Vec<String> = r
        .entry_rules
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    println!(
        "{}",
        l.t_args("deadcode-rules", &args!("list" => rules.join(", ")))
    );
    if r.dead.is_empty() && r.ambiguous.is_empty() {
        println!("{}", l.t("deadcode-none"));
    }
    if !r.dead.is_empty() {
        println!(
            "\n{}",
            l.t_args("deadcode-dead-head", &args!("count" => r.dead.len()))
        );
        for d in &r.dead {
            println!("  {}", item(l, d));
        }
    }
    if !r.ambiguous.is_empty() {
        println!(
            "\n{}",
            l.t_args(
                "deadcode-ambiguous-head",
                &args!("count" => r.ambiguous.len())
            )
        );
        for d in &r.ambiguous {
            println!("  {}", item(l, d));
        }
    }
    for n in &r.notes {
        println!("{n}");
    }
}

fn print_plan(l: &Localizer, p: &deadcode::PrunePlan) {
    println!(
        "{}",
        l.t_args("deadcode-prune-head", &args!("branch" => p.branch.clone()))
    );
    println!(
        "  {}",
        l.t_args("deadcode-prune-counts", &args!("removed" => p.removed.len(), "restored" => p.restored.len(), "skipped" => p.skipped.len()))
    );
    for (key, run) in [
        ("deadcode-prune-check", &p.check),
        ("deadcode-prune-tests", &p.tests),
    ] {
        if let Some(c) = run {
            let res = l.t(if c.ok {
                "deadcode-ok"
            } else {
                "deadcode-failed"
            });
            println!(
                "  {}",
                l.t_args(key, &args!("command" => c.command.clone(), "result" => res))
            );
            if !c.ok {
                println!("{}", c.output.trim_end());
            }
        }
    }
    for n in &p.notes {
        println!("  {}", note(l, n));
    }
    if p.ready {
        println!(
            "{}",
            l.t_args("deadcode-prune-ready", &args!("branch" => p.branch.clone(),
                "commit" => p.commit.clone().unwrap_or_default().chars().take(10).collect::<String>()))
        );
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{DeadCodeArgs, OutFormat};

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        a: DeadCodeArgs,
    }

    #[test]
    fn parses() {
        assert_eq!(
            Cli::try_parse_from(["dead-code", "--format", "sarif"])
                .unwrap()
                .a
                .format,
            OutFormat::Sarif
        );
        let c = Cli::try_parse_from(["dead-code", "--prune", "--check-cmd", "cargo check", "-y"])
            .unwrap();
        assert!(c.a.prune && c.a.yes);
        assert!(Cli::try_parse_from(["dead-code", "--include-medium"]).is_err());
    }
}
