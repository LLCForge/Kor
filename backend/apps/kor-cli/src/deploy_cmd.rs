//! `kor deploy <env>`, `kor deploy risk-check <env>`, `kor deploy status` (spec §8.6, ADR 0041).
//! The plan and the run live in `kor_runtime::deploy`; this file prints and asks.

use std::io::IsTerminal as _;

use kor_i18n::{Localizer, args};
use kor_runtime::deploy::{self, Event, Level, Step};

use crate::Ctx;

#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DeployArgs {
    #[command(subcommand)]
    pub command: Option<DeploySub>,
    /// Environment from .kor/deploy.toml (`local` without one).
    pub env: Option<String>,
    /// Approve every non-destructive step (destructive steps still ask).
    #[arg(long)]
    pub yes: bool,
    /// Show the plan only.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum DeploySub {
    /// Irreversible migrations, breaking API changes, peak hours.
    RiskCheck { env: String },
    /// Environments and their last deploy.
    Status,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-deploy"))
        .mut_arg("env", |a| a.help(l.t("arg-deploy-env")))
        .mut_arg("yes", |a| a.help(l.t("arg-deploy-yes")))
        .mut_arg("dry_run", |a| a.help(l.t("arg-deploy-dry-run")))
        .mut_subcommand("risk-check", |s| {
            s.about(l.t("cmd-deploy-risk"))
                .mut_arg("env", |a| a.help(l.t("arg-deploy-env")))
        })
        .mut_subcommand("status", |s| s.about(l.t("cmd-deploy-status")))
}

fn red(s: &str) -> String {
    if std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal() {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_owned()
    }
}

fn step_line(l: &Localizer, s: &Step) -> String {
    let line = l.t_args("deploy-step", &args!("id" => s.id, "cmd" => s.display.clone()));
    if s.destructive {
        red(&format!("{line}  [{}]", l.t("deploy-destructive")))
    } else {
        line
    }
}

pub fn run(ctx: &Ctx, a: &DeployArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    match (&a.command, &a.env) {
        (Some(DeploySub::Status), _) | (None, None) => Ok(status(l, &root)),
        (Some(DeploySub::RiskCheck { env }), _) => Ok(risk(l, &root, env)),
        (None, Some(env)) => deploy_env(l, &root, env, a.yes, a.dry_run),
    }
}

fn err(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!("{}", l.t_args("deploy-error", &args!("error" => e.to_string())));
    false
}

fn deploy_env(l: &Localizer, root: &std::path::Path, env: &str, yes: bool, dry: bool) -> anyhow::Result<bool> {
    let plan = match deploy::plan(root, env) {
        Ok(p) => p,
        Err(e) => return Ok(err(l, e)),
    };
    let services = if plan.services.is_empty() { "-".to_owned() } else { plan.services.join(", ") };
    println!(
        "{}",
        l.t_args("deploy-plan-head", &args!("env" => env, "target" => plan.target.clone(), "services" => services))
    );
    match (&plan.since, &plan.changed_files) {
        (Some(c), Some(f)) => println!(
            "{}",
            l.t_args("deploy-since", &args!("commit" => c[..c.len().min(12)].to_owned(), "count" => f.len()))
        ),
        _ => println!("{}", l.t_args("deploy-first", &args!("env" => env))),
    }
    for n in &plan.notes {
        println!("{}", l.t_args("deploy-note", &args!("text" => n.clone())));
    }
    if plan.steps.is_empty() {
        println!("{}", l.t_args("deploy-nothing", &args!("env" => env)));
        return Ok(true);
    }
    for s in &plan.steps {
        println!("  {}", step_line(l, s));
    }
    if dry {
        println!("{}", l.t("deploy-dry-run"));
        return Ok(true);
    }
    let rt = crate::providers::runtime()?;
    let mut approve = |s: &Step| {
        println!("{}", step_line(l, s));
        let q = l.t_args("deploy-step-ask", &args!("id" => s.id));
        // Destructive steps always ask, even with --yes (ADR 0030).
        crate::shell_cmd::confirm(l, &q, yes && !s.destructive)
    };
    let on = |e: &Event| match e {
        Event::Start { step } => println!("$ {}", step.display),
        Event::Output { line, .. } => println!("  {line}"),
        Event::Done { ok: false, detail, .. } => eprintln!("  {}", red(detail)),
        Event::Note { text } => println!("{}", l.t_args("deploy-note", &args!("text" => text.clone()))),
        _ => {}
    };
    let report = rt.block_on(deploy::run(root, &plan, &mut approve, &on));
    for h in &report.health {
        let status = if h.ok { "ok" } else { "FAILED" };
        println!(
            "{}",
            l.t_args("deploy-health", &args!("service" => h.service.clone(), "status" => status, "detail" => h.detail.clone()))
        );
    }
    if report.ok {
        let c = report.commit.clone().unwrap_or_default();
        println!("{}", l.t_args("deploy-ok", &args!("env" => env, "commit" => c[..c.len().min(12)].to_owned())));
    } else if report.cancelled {
        println!("{}", l.t("deploy-cancelled"));
    } else {
        eprintln!("{}", red(&l.t_args("deploy-failed", &args!("env" => env))));
        if report.rolled_back {
            println!("{}", l.t("deploy-rolled-back"));
        }
    }
    Ok(report.ok)
}

fn risk(l: &Localizer, root: &std::path::Path, env: &str) -> bool {
    let now = chrono::Local::now().time();
    let r = match deploy::risk_check(root, env, now) {
        Ok(r) => r,
        Err(e) => return err(l, e),
    };
    if r.risks.is_empty() {
        println!("{}", l.t_args("deploy-risk-none", &args!("env" => env)));
        return true;
    }
    println!("{}", l.t_args("deploy-risk-head", &args!("env" => env, "count" => r.risks.len())));
    for x in &r.risks {
        let tag = match x.level {
            Level::High => red("HIGH"),
            Level::Medium => "MEDIUM".to_owned(),
            Level::Info => "info".to_owned(),
        };
        let file = x.file.as_deref().map(|f| format!(" ({f})")).unwrap_or_default();
        println!("  [{tag}] {}{file}\n      {}", x.title, x.detail);
    }
    !r.risks.iter().any(|x| x.level == Level::High)
}

fn status(l: &Localizer, root: &std::path::Path) -> bool {
    let envs = match deploy::envs(root) {
        Ok(e) => e,
        Err(e) => return err(l, e),
    };
    let state = deploy::load_state(root);
    for env in envs {
        match state.envs.get(&env) {
            Some(s) => {
                let commit = s.commit.as_deref().map_or("-", |c| &c[..c.len().min(12)]).to_owned();
                let when = chrono::DateTime::from_timestamp(i64::try_from(s.at).unwrap_or(0), 0)
                    .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let result = match &s.last {
                    Some(r) if r.ok => "ok",
                    Some(r) if r.rolled_back => "rolled back",
                    Some(_) => "failed",
                    None => "-",
                };
                println!("{}", l.t_args("deploy-status-line", &args!("env" => env, "commit" => commit, "when" => when, "result" => result)));
            }
            None => println!("{}", l.t_args("deploy-status-never", &args!("env" => env))),
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: DeployArgs,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["x", "staging", "--dry-run"]).unwrap();
        assert_eq!(t.a.env.as_deref(), Some("staging"));
        assert!(t.a.dry_run && t.a.command.is_none());
        let t = T::try_parse_from(["x", "risk-check", "prod"]).unwrap();
        assert!(matches!(t.a.command, Some(DeploySub::RiskCheck { ref env }) if env == "prod"));
        let l = Localizer::new(kor_i18n::Lang::En);
        localize(T::command(), &l).debug_assert();
    }
}
