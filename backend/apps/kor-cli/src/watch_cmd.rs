//! `kor watch logs|incidents|fix|pr|clear` (spec §8.7, ADR 0041). Detection, incidents and the
//! fix flow live in `kor_runtime::watch`; `fix` runs the agent in the normal REPL (every write is
//! reviewed per file) and asks before committing, pushing and opening the pull request.

use kor_i18n::{Localizer, args};
use kor_runtime::watch::{self, Event, Source};

use crate::Ctx;

#[derive(Debug, Clone, clap::Subcommand)]
pub enum WatchCmd {
    /// Follow a log and report errors.
    Logs { source: String },
    /// Errors seen so far.
    Incidents,
    /// Let the agent find the root cause and fix an incident on a new branch, then open a PR.
    Fix {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Commit, push and open the pull request for a fix branch.
    Pr {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Forget one incident, or all of them.
    Clear { id: Option<String> },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-watch"))
        .mut_subcommand("logs", |s| {
            s.about(l.t("cmd-watch-logs"))
                .mut_arg("source", |a| a.help(l.t("arg-watch-source")))
        })
        .mut_subcommand("incidents", |s| s.about(l.t("cmd-watch-incidents")))
        .mut_subcommand("fix", |s| {
            s.about(l.t("cmd-watch-fix"))
                .mut_arg("id", |a| a.help(l.t("arg-watch-id")))
                .mut_arg("yes", |a| a.help(l.t("arg-watch-yes")))
        })
        .mut_subcommand("pr", |s| {
            s.about(l.t("cmd-watch-pr"))
                .mut_arg("id", |a| a.help(l.t("arg-watch-id")))
                .mut_arg("yes", |a| a.help(l.t("arg-watch-yes")))
        })
        .mut_subcommand("clear", |s| {
            s.about(l.t("cmd-watch-clear"))
                .mut_arg("id", |a| a.help(l.t("arg-watch-id")))
        })
}

fn err(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("watch-error", &args!("error" => e.to_string()))
    );
    false
}

pub fn run(ctx: &Ctx, cmd: &WatchCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    Ok(match cmd {
        WatchCmd::Logs { source } => logs(l, &root, source)?,
        WatchCmd::Incidents => {
            let list = watch::incidents(&root);
            if list.is_empty() {
                println!("{}", l.t("watch-no-incidents"));
            }
            for i in list {
                let pr = i
                    .fix
                    .as_ref()
                    .and_then(|f| f.pr.clone())
                    .map(|u| format!("  PR {u}"))
                    .unwrap_or_default();
                println!("{}  {:>5}x  {}  {}{pr}", i.id, i.count, i.source, i.title);
            }
            true
        }
        WatchCmd::Fix { id, yes } => fix(ctx, &root, id, *yes)?,
        WatchCmd::Pr { id, yes } => pr(l, &root, id, *yes)?,
        WatchCmd::Clear { id } => match watch::clear(&root, id.as_deref()) {
            Ok(n) => {
                println!("{}", l.t_args("watch-cleared", &args!("count" => n)));
                true
            }
            Err(e) => err(l, e),
        },
    })
}

fn logs(l: &Localizer, root: &std::path::Path, source: &str) -> anyhow::Result<bool> {
    let src = match Source::parse(source, root) {
        Ok(s) => s,
        Err(e) => return Ok(err(l, e)),
    };
    let cfg = match watch::load_config(root) {
        Ok(c) => c,
        Err(e) => return Ok(err(l, e)),
    };
    println!(
        "{}",
        l.t_args("watch-started", &args!("source" => src.label()))
    );
    let rt = crate::providers::runtime()?;
    let cancel = watch::CancellationToken::new();
    let on = |e: &Event| match e {
        Event::Listening { url } => println!(
            "{}",
            l.t_args("watch-listening", &args!("url" => url.clone()))
        ),
        Event::Incident { incident } => {
            println!("\n[{}] {}", incident.id, incident.title);
            println!(
                "{}",
                l.t_args("watch-suggest-fix", &args!("id" => incident.id.clone()))
            );
        }
        Event::Repeat { id, count } if count.is_power_of_two() => {
            println!(
                "{}",
                l.t_args(
                    "watch-repeat",
                    &args!("id" => id.clone(), "count" => *count)
                )
            );
        }
        Event::Limited { total } => {
            println!("{}", l.t_args("watch-limited", &args!("count" => *total)));
        }
        Event::Ended { error: Some(e) } => {
            eprintln!("{}", l.t_args("watch-error", &args!("error" => e.clone())));
        }
        _ => {}
    };
    let result = rt.block_on(async {
        let c = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            c.cancel();
        });
        watch::watch(root, &src, &cfg, &on, cancel).await
    });
    Ok(result.is_ok())
}

fn fix(ctx: &Ctx, root: &std::path::Path, id: &str, yes: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(i) = watch::incident(root, id) else {
        return Ok(err(l, format!("no incident `{id}` (kor watch incidents)")));
    };
    println!("[{}] {}\n", i.id, i.title);
    if !crate::shell_cmd::confirm(l, &l.t("watch-fix-ask"), yes) {
        return Ok(false);
    }
    let (_, prompt) = match watch::fix_start(root, id) {
        Ok(v) => v,
        Err(e) => return Ok(err(l, e)),
    };
    let ok = crate::repl::run(
        ctx,
        &crate::repl::RunArgs {
            prompt: Some(prompt),
            ..crate::repl::RunArgs::default()
        },
    )?;
    if !ok {
        return Ok(false);
    }
    pr(l, root, id, false)
}

fn pr(l: &Localizer, root: &std::path::Path, id: &str, yes: bool) -> anyhow::Result<bool> {
    let files = match watch::fix_files(root) {
        Ok(f) => f,
        Err(e) => return Ok(err(l, e)),
    };
    println!(
        "{}",
        l.t_args("watch-pr-files", &args!("count" => files.len()))
    );
    for f in &files {
        println!("  {f}");
    }
    if !crate::shell_cmd::confirm(l, &l.t("watch-pr-ask"), yes) {
        return Ok(false);
    }
    let rt = crate::providers::runtime()?;
    match rt.block_on(watch::fix_finish(root, id)) {
        Ok(pr) => {
            println!(
                "{}",
                l.t_args(
                    "watch-pr-opened",
                    &args!("url" => pr.url, "forge" => pr.forge)
                )
            );
            Ok(true)
        }
        Err(e) => Ok(err(l, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: WatchCmd,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["x", "logs", "docker:web"]).unwrap();
        assert!(matches!(t.c, WatchCmd::Logs { ref source } if source == "docker:web"));
        let t = T::try_parse_from(["x", "fix", "abc", "--yes"]).unwrap();
        assert!(matches!(t.c, WatchCmd::Fix { yes: true, .. }));
        let l = Localizer::new(kor_i18n::Lang::En);
        localize(T::command(), &l).debug_assert();
    }
}
