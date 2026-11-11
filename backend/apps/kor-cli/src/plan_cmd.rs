//! `kor plan list|show|approve` (Master Build Prompt §28.2, ADR 0050): the plans the agent saved
//! in `.kor/plans/` while in plan mode. Approving one is a user action and opens the write tools
//! of the session that wrote it (the running agent sees it before its next model call).
//! Logic: `kor_tools::plans`.

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_tools::plans::{self, Status};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum PlanCmd {
    /// Plans of this project, newest first.
    List,
    /// Print one plan (id or a unique id prefix; the newest when omitted).
    Show { id: Option<String> },
    /// Approve a plan: write tools open for its session.
    Approve { id: Option<String> },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-plan"))
        .mut_subcommand("list", |s| s.about(l.t("cmd-plan-list")))
        .mut_subcommand("show", |s| {
            s.about(l.t("cmd-plan-show"))
                .mut_arg("id", |a| a.help(l.t("arg-plan-id")))
        })
        .mut_subcommand("approve", |s| {
            s.about(l.t("cmd-plan-approve"))
                .mut_arg("id", |a| a.help(l.t("arg-plan-id")))
        })
}

fn status(l: &Localizer, s: Status) -> String {
    l.t(match s {
        Status::Draft => "plan-status-draft",
        Status::Approved => "plan-status-approved",
    })
}

fn pick(
    l: &Localizer,
    root: &std::path::Path,
    id: Option<&str>,
    draft: bool,
) -> Option<plans::Plan> {
    let p = match id {
        Some(id) => plans::find(root, id),
        None if draft => plans::latest_draft(root, None),
        None => plans::list(root).into_iter().next(),
    };
    if p.is_none() {
        eprintln!(
            "{}",
            l.t_args(
                "plan-not-found",
                &args!("id" => id.unwrap_or("-").to_owned())
            )
        );
    }
    p
}

pub fn run(ctx: &Ctx, cmd: Option<&PlanCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    match cmd.unwrap_or(&PlanCmd::List) {
        PlanCmd::List => {
            let all = plans::list(&root);
            if all.is_empty() {
                println!("{}", l.t("plan-none"));
            }
            for p in all {
                println!("{}  [{}]  {}", p.id, status(l, p.status), p.title);
            }
            Ok(true)
        }
        PlanCmd::Show { id } => {
            let Some(p) = pick(l, &root, id.as_deref(), false) else {
                return Ok(false);
            };
            println!(
                "# {}  [{}]\n{}\n\n{}",
                p.title,
                status(l, p.status),
                kor_tools::display_path(&p.path),
                p.body
            );
            Ok(true)
        }
        PlanCmd::Approve { id } => {
            let Some(p) = pick(l, &root, id.as_deref(), true) else {
                return Ok(false);
            };
            let p = plans::approve(&root, &p.id)?;
            println!(
                "{}",
                l.t_args(
                    "plan-approved",
                    &args!("id" => p.id.clone(), "title" => p.title.clone())
                )
            );
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(clap::Subcommand)]
    enum Top {
        Plan {
            #[command(subcommand)]
            command: Option<PlanCmd>,
        },
    }

    #[test]
    fn parses_plan_commands() {
        let Top::Plan { command } = Cli::try_parse_from(["kor", "plan"]).unwrap().cmd;
        assert!(command.is_none());
        let Top::Plan { command } = Cli::try_parse_from(["kor", "plan", "approve", "2026-11"])
            .unwrap()
            .cmd;
        assert!(matches!(command, Some(PlanCmd::Approve { id: Some(ref i) }) if i == "2026-11"));
        let Top::Plan { command } = Cli::try_parse_from(["kor", "plan", "show"]).unwrap().cmd;
        assert!(matches!(command, Some(PlanCmd::Show { id: None })));
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let _ = localize(
            <Cli as clap::CommandFactory>::command()
                .find_subcommand("plan")
                .unwrap()
                .clone(),
            &l,
        );
    }
}
