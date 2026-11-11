//! `kor rules check|list|report` (Master Build Prompt §28.3, ADR 0063): the working tree's
//! changes against the loaded rules (AGENTS.md, CLAUDE.md, `.kor/rules`, Cursor/Windsurf/… rules).
//! Deterministic checks always; `--model` lets a light model judge the prose rules. Findings carry
//! the rule's source <file:line>; `check` exits non-zero on a violation (CI). The agent runs the same
//! check before it shows a diff. Logic: `kor_runtime::compliance`, `kor_tools::rules_check`.

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_tools::rules_check::{Finding, Verdict};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum RulesCmd {
    /// Check the uncommitted changes (or the staged ones) against the rules.
    Check {
        /// Only staged changes.
        #[arg(long)]
        staged: bool,
        /// Compare against this revision instead of HEAD.
        #[arg(long)]
        base: Option<String>,
        /// Let a model judge the prose rules (optionally which model).
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        model: Option<String>,
        /// Show followed and relevant rules too.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// The loaded rule lines and which ones have a deterministic check.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Most broken rules and by which model (from the agent's checks).
    Report {
        #[arg(long, default_value_t = 7)]
        days: u64,
        #[arg(long)]
        json: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let json = |s: clap::Command| s.mut_arg("json", |a| a.help(l.t("arg-json")));
    c.about(l.t("cmd-rules"))
        .mut_subcommand("check", |s| {
            json(s.about(l.t("cmd-rules-check")))
                .mut_arg("staged", |a| a.help(l.t("arg-rules-staged")))
                .mut_arg("base", |a| a.help(l.t("arg-rules-base")))
                .mut_arg("model", |a| a.help(l.t("arg-rules-model")))
                .mut_arg("all", |a| a.help(l.t("arg-rules-all")))
        })
        .mut_subcommand("list", |s| json(s.about(l.t("cmd-rules-list"))))
        .mut_subcommand("report", |s| {
            json(s.about(l.t("cmd-rules-report")))
                .mut_arg("days", |a| a.help(l.t("arg-rules-days")))
        })
}

fn badge(v: Verdict) -> &'static str {
    match v {
        Verdict::Violated => "\x1b[31m✗\x1b[0m",
        Verdict::Relevant => "\x1b[33m•\x1b[0m",
        Verdict::Applied => "\x1b[32m✓\x1b[0m",
    }
}

fn print_finding(f: &Finding) {
    let at = f
        .file
        .as_ref()
        .map(|p| {
            format!(
                "  {p}{}",
                f.file_line.map(|n| format!(":{n}")).unwrap_or_default()
            )
        })
        .unwrap_or_default();
    println!("{} {}  ({}){at}", badge(f.verdict), f.rule, f.location());
    if !f.message.is_empty() {
        println!("    {}", f.message);
    }
}

pub fn run(ctx: &Ctx, cmd: &RulesCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    let compat = kor_runtime::load_compat(&root);
    let set = kor_runtime::compliance::rule_set(&compat);
    let display = kor_runtime::compliance::display(&root);
    match cmd {
        RulesCmd::List { json } => {
            if *json {
                let rows: Vec<serde_json::Value> = set
                    .rules
                    .iter()
                    .map(|r| serde_json::json!({ "source": display(&r.source), "line": r.line, "rule": r.text, "checkable": r.checkable() }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if set.rules.is_empty() {
                println!("{}", l.t("rules-none"));
            } else {
                for r in &set.rules {
                    println!(
                        "{} {}:{}  {}",
                        if r.checkable() { "⚙" } else { "·" },
                        display(&r.source),
                        r.line,
                        r.text
                    );
                }
            }
            Ok(true)
        }
        RulesCmd::Report { days, json } => {
            let since = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
                .saturating_sub(days.saturating_mul(86_400));
            let rows = kor_runtime::compliance::log_path()
                .map(|p| kor_tools::rules_check::report(&p, since))
                .unwrap_or_default();
            if *json {
                let v: Vec<serde_json::Value> = rows.iter().map(|(s, r, m, n)| serde_json::json!({ "source": s, "rule": r, "model": m, "count": n })).collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else if rows.is_empty() {
                println!("{}", l.t_args("rules-report-none", &args!("days" => *days)));
            } else {
                println!("{}", l.t_args("rules-report-head", &args!("days" => *days)));
                for (src, rule, model, n) in rows.iter().take(30) {
                    println!("  {n:>4}×  {rule}  ({src})  {model}");
                }
            }
            Ok(true)
        }
        RulesCmd::Check {
            staged,
            base,
            model,
            all,
            json,
        } => {
            let changes =
                match kor_runtime::compliance::working_changes(&root, base.as_deref(), *staged) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                        return Ok(false);
                    }
                };
            let mut findings = set.check(&changes, &display);
            if let Some(m) = model {
                let loaded = match &ctx.config {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "{}",
                            l.t_args("error-generic", &args!("error" => e.to_string()))
                        );
                        return Ok(false);
                    }
                };
                let pick = (!m.is_empty()).then_some(m.as_str());
                match kor_runtime::oneshot_provider(l, loaded, pick) {
                    Ok((provider, name)) => {
                        eprintln!(
                            "{}",
                            l.t_args("rules-judging", &args!("model" => name.clone()))
                        );
                        match crate::providers::runtime()?.block_on(kor_runtime::compliance::judge(
                            &provider,
                            &name,
                            findings.clone(),
                            &changes,
                        )) {
                            Ok(f) => findings = f,
                            Err(e) => eprintln!(
                                "{}",
                                l.t_args("rules-judge-failed", &args!("error" => e))
                            ),
                        }
                    }
                    Err(e) => eprintln!("{e}"),
                }
            }
            let violated = findings
                .iter()
                .filter(|f| f.verdict == Verdict::Violated)
                .count();
            if *json {
                println!("{}", serde_json::to_string_pretty(&findings)?);
                return Ok(violated == 0);
            }
            if changes.is_empty() {
                println!("{}", l.t("rules-no-changes"));
                return Ok(true);
            }
            for f in findings
                .iter()
                .filter(|f| *all || f.verdict == Verdict::Violated)
            {
                print_finding(f);
            }
            let relevant = findings
                .iter()
                .filter(|f| f.verdict == Verdict::Relevant)
                .count();
            let applied = findings
                .iter()
                .filter(|f| f.verdict == Verdict::Applied)
                .count();
            println!(
                "{}",
                l.t_args("rules-summary", &args!("violated" => violated, "relevant" => relevant, "applied" => applied, "files" => changes.len()))
            );
            Ok(violated == 0)
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
        cmd: RulesCmd,
    }

    #[test]
    fn parses() {
        assert!(
            matches!(Cli::try_parse_from(["r", "check", "--model"]).unwrap().cmd, RulesCmd::Check { model: Some(ref m), .. } if m.is_empty())
        );
        assert!(
            matches!(Cli::try_parse_from(["r", "check", "--model", "x/y", "--staged"]).unwrap().cmd, RulesCmd::Check { staged: true, model: Some(ref m), .. } if m == "x/y")
        );
        assert!(matches!(
            Cli::try_parse_from(["r", "check"]).unwrap().cmd,
            RulesCmd::Check { model: None, .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["r", "report", "--days", "30"])
                .unwrap()
                .cmd,
            RulesCmd::Report { days: 30, .. }
        ));
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
