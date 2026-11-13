//! `kor triggers list|test|history|approve|reject|hook` (§8.4, ADR 0039). Logic in
//! `kor_runtime::triggers`; this file prints.

use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_runtime::triggers::{self, daemon, hook};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum TriggersCmd {
    /// Triggers of the user config and watched projects (this project starts being watched).
    List,
    /// Fire a trigger now (in the daemon when it runs, else here).
    Test { name: String },
    /// Past and running jobs.
    History {
        name: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Approve a pending request (id from `kor daemon status` or a notification).
    Approve { id: String },
    /// Reject a pending request.
    Reject { id: String },
    /// Local git hook for `git_push` triggers.
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum HookAction {
    /// Install it (bare repo: post-receive; working repo: reference-transaction).
    Install { repo: Option<PathBuf> },
    /// Remove it.
    Uninstall { repo: Option<PathBuf> },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-triggers"))
        .mut_subcommand("list", |s| s.about(l.t("cmd-triggers-list")))
        .mut_subcommand("test", |s| s.about(l.t("cmd-triggers-test")))
        .mut_subcommand("history", |s| s.about(l.t("cmd-triggers-history")))
        .mut_subcommand("approve", |s| s.about(l.t("cmd-triggers-approve")))
        .mut_subcommand("reject", |s| s.about(l.t("cmd-triggers-reject")))
        .mut_subcommand("hook", |s| s.about(l.t("cmd-triggers-hook")))
}

fn dir() -> anyhow::Result<PathBuf> {
    triggers::dir().ok_or_else(|| anyhow::anyhow!("no Kor data folder"))
}

fn defs(l: &Localizer, dir: &std::path::Path) -> Vec<triggers::Defined> {
    super::daemon_cmd::register_cwd(dir);
    let user = kor_config::user_dir().map(|d| d.join("config.toml"));
    triggers::collect(l, user.as_deref(), &triggers::projects(dir))
}

fn when(ms: Option<u64>) -> String {
    ms.map_or_else(|| "-".into(), kor_runtime::cost::fmt_time)
}

#[allow(clippy::too_many_lines)] // flat subcommand dispatch
pub fn run(ctx: &Ctx, cmd: TriggersCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let dir = dir()?;
    match cmd {
        TriggersCmd::List => {
            let all = defs(l, &dir);
            if all.is_empty() {
                println!("{}", l.t("triggers-none"));
            }
            for d in &all {
                let v = triggers::describe(d);
                let state = match (&d.error, d.trigger.enabled) {
                    (Some(_), _) => "!",
                    (None, false) => "-",
                    (None, true) => "*",
                };
                println!(
                    "{state} {:<20} {:<10} {:<20} {}",
                    d.name,
                    v["event"].as_str().unwrap_or_default(),
                    v["on"].as_str().unwrap_or_default(),
                    d.root.display()
                );
                println!("    {}", d.trigger.task.lines().next().unwrap_or_default());
                if let Some(e) = &d.error {
                    println!("    ! {e}");
                }
            }
            Ok(all.iter().all(|d| d.error.is_none()))
        }
        TriggersCmd::Test { name } => {
            let rt = crate::providers::runtime()?;
            if daemon::state(&dir).is_some() {
                let v = rt
                    .block_on(daemon::control(
                        &dir,
                        "POST",
                        "/control/test",
                        Some(serde_json::json!({ "name": name })),
                    ))
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!(
                    "{}",
                    l.t_args(
                        "triggers-job-queued",
                        &args!("job" => v["job"].as_str().unwrap_or_default().to_owned())
                    )
                );
                return Ok(true);
            }
            let cfg = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut notes = Vec::new();
            let opts = daemon::options(l, &cfg.config, None, false, &mut notes)
                .map_err(|e| anyhow::anyhow!(e))?;
            for n in notes {
                eprintln!("{n}");
            }
            let env = kor_runtime::triggers::runner::Env {
                l: Localizer::new(l.lang()),
                dir: dir.clone(),
                hub: std::sync::Arc::default(),
                notifiers: std::sync::Arc::new(opts.notifiers),
                public_url: None,
                approval_timeout: opts.approval_timeout,
                factory: opts.factory,
                live: std::sync::Arc::default(),
            };
            let all = defs(l, &dir);
            let job = rt
                .block_on(daemon::run_once(&env, &all, &name))
                .map_err(|e| anyhow::anyhow!(e))?;
            print_job(&job);
            Ok(job.status == triggers::Status::Done)
        }
        TriggersCmd::History { name, limit } => {
            let jobs = triggers::history(&dir, name.as_deref(), limit.clamp(1, 500));
            if jobs.is_empty() {
                println!("{}", l.t("triggers-history-none"));
            }
            for j in &jobs {
                print_job(j);
            }
            Ok(true)
        }
        TriggersCmd::Approve { id } => answer(l, &dir, &id, true),
        TriggersCmd::Reject { id } => answer(l, &dir, &id, false),
        TriggersCmd::Hook { action } => {
            let (repo, install) = match action {
                HookAction::Install { repo } => (repo, true),
                HookAction::Uninstall { repo } => (repo, false),
            };
            let repo = match repo {
                Some(r) => r,
                None => std::env::current_dir()?,
            };
            if install {
                let p =
                    hook::install(&repo, &daemon::inbox(&dir)).map_err(|e| anyhow::anyhow!(e))?;
                super::daemon_cmd::register_cwd(&dir);
                println!(
                    "{}",
                    l.t_args(
                        "triggers-hook-installed",
                        &args!("path" => p.display().to_string())
                    )
                );
            } else if hook::uninstall(&repo).map_err(|e| anyhow::anyhow!(e))? {
                println!("{}", l.t("triggers-hook-removed"));
            } else {
                println!("{}", l.t("triggers-hook-none"));
            }
            Ok(true)
        }
    }
}

fn answer(l: &Localizer, dir: &std::path::Path, id: &str, approve: bool) -> anyhow::Result<bool> {
    let v = crate::providers::runtime()?
        .block_on(daemon::control(
            dir,
            "POST",
            "/control/answer",
            Some(serde_json::json!({ "id": id, "approve": approve })),
        ))
        .map_err(|e| anyhow::anyhow!(e))?;
    let ok = v["ok"] == true;
    println!(
        "{}",
        l.t(if ok {
            "triggers-answered"
        } else {
            "triggers-no-pending"
        })
    );
    Ok(ok)
}

fn print_job(j: &triggers::Job) {
    println!(
        "{}  {:<8?} {:<20} {}  {}",
        j.id,
        j.status,
        j.trigger,
        when(j.started.or(Some(j.created))),
        kor_runtime::cost::fmt_usd(j.cost_usd, j.cost_estimated)
    );
    println!("    {}", j.detail.lines().next().unwrap_or_default());
    if let Some(pr) = &j.pr_url {
        println!("    PR {pr}");
    }
    if let Some(e) = &j.error {
        println!("    ! {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        cmd: TriggersCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            T::try_parse_from(["kor", "list"]).unwrap().cmd,
            TriggersCmd::List
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "test", "ci"]).unwrap().cmd,
            TriggersCmd::Test { .. }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "history", "ci", "-n", "3"])
                .unwrap()
                .cmd,
            TriggersCmd::History { limit: 3, .. }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "approve", "ab12"]).unwrap().cmd,
            TriggersCmd::Approve { .. }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "reject", "ab12"]).unwrap().cmd,
            TriggersCmd::Reject { .. }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "hook", "install", "/srv/origin.git"])
                .unwrap()
                .cmd,
            TriggersCmd::Hook {
                action: HookAction::Install { repo: Some(_) }
            }
        ));
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<T as clap::CommandFactory>::command(), &l);
    }
}
