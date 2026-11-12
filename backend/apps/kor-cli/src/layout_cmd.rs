//! `kor layout save|restore|list|delete` (§27.7, ADR 0059): window arrangements across monitors,
//! optionally linked to the project (opening it in the desktop app brings the layout back).
//! Logic in `kor_computer::layout` and `kor_tools::layouts`.

use kor_computer::layout::Store;
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum LayoutCmd {
    /// Save the open windows' arrangement.
    Save {
        name: String,
        /// Link it to the current project.
        #[arg(long)]
        project: bool,
    },
    /// Put the windows back (default: the project's linked layout).
    Restore {
        name: Option<String>,
        /// Show the moves without doing them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Saved layouts.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Delete a saved layout.
    Delete { name: String },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let name = |s: clap::Command| s.mut_arg("name", |a| a.help(l.t("arg-layout-name")));
    c.about(l.t("cmd-layout"))
        .mut_subcommand("save", |s| {
            name(s.about(l.t("cmd-layout-save")))
                .mut_arg("project", |a| a.help(l.t("arg-layout-project")))
        })
        .mut_subcommand("restore", |s| {
            name(s.about(l.t("cmd-layout-restore")))
                .mut_arg("dry_run", |a| a.help(l.t("arg-layout-dry-run")))
        })
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-layout-list"))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
        .mut_subcommand("delete", |s| name(s.about(l.t("cmd-layout-delete"))))
}

fn store(l: &Localizer) -> anyhow::Result<Store> {
    kor_runtime::kor_home()
        .map(|h| Store::at(&h.join("layouts")))
        .ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))
}

fn project() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    Some(kor_runtime::osint::project_root(&cwd))
}

#[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
pub fn run(ctx: &Ctx, cmd: LayoutCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let s = store(l)?;
    let cwd = std::env::current_dir()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    match cmd {
        LayoutCmd::Save {
            name,
            project: link,
        } => {
            let proj = if link { project() } else { None };
            let saved = rt
                .block_on(kor_tools::layouts::save(&s, &name, &cwd, proj.as_deref()))
                .map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "{}",
                l.t_args("layout-saved", &args!("name" => name.clone(), "windows" => saved.windows.len(), "monitors" => saved.monitors.len()))
            );
            if let Some(p) = proj {
                println!(
                    "{}",
                    l.t_args(
                        "layout-linked",
                        &args!("name" => name, "project" => p.display().to_string())
                    )
                );
            }
            Ok(true)
        }
        LayoutCmd::Restore { name, dry_run } => {
            let Some(name) =
                name.or_else(|| project().and_then(|p| kor_computer::layout::linked(&p)))
            else {
                eprintln!("{}", l.t("layout-no-link"));
                return Ok(false);
            };
            let plan = rt
                .block_on(kor_tools::layouts::restore(&s, &name, &cwd, dry_run))
                .map_err(|e| anyhow::anyhow!(e))?;
            for m in &plan.moves {
                println!(
                    "  {} — {}  {},{} {}x{} {}",
                    m.process, m.title, m.x, m.y, m.w, m.h, m.state
                );
            }
            for m in &plan.missing {
                eprintln!(
                    "{}",
                    l.t_args("layout-missing", &args!("window" => m.clone()))
                );
            }
            let key = if dry_run {
                "layout-planned"
            } else {
                "layout-restored"
            };
            println!(
                "{}",
                l.t_args(key, &args!("name" => name, "windows" => plan.moves.len()))
            );
            Ok(plan.missing.is_empty())
        }
        LayoutCmd::List { json } => {
            let all = s.list();
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(true);
            }
            if all.is_empty() {
                println!("{}", l.t("layout-none"));
            }
            let linked = project().and_then(|p| kor_computer::layout::linked(&p));
            for x in &all {
                println!(
                    "{}{}  {}",
                    x.name,
                    if linked.as_deref() == Some(x.name.as_str()) {
                        " *"
                    } else {
                        ""
                    },
                    l.t_args(
                        "layout-row",
                        &args!("windows" => x.windows.len(), "monitors" => x.monitors.len())
                    )
                );
            }
            Ok(true)
        }
        LayoutCmd::Delete { name } => {
            s.delete(&name).map_err(|e| anyhow::anyhow!(e))?;
            if let Some(p) = project()
                && kor_computer::layout::linked(&p).as_deref() == Some(name.as_str())
            {
                kor_computer::layout::link(&p, None).map_err(|e| anyhow::anyhow!(e))?;
            }
            println!("{}", l.t_args("layout-deleted", &args!("name" => name)));
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: LayoutCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            T::parse_from(["t", "save", "kodlama", "--project"]).c,
            LayoutCmd::Save { project: true, .. }
        ));
        assert!(matches!(
            T::parse_from(["t", "restore"]).c,
            LayoutCmd::Restore {
                name: None,
                dry_run: false
            }
        ));
        assert!(matches!(
            T::parse_from(["t", "restore", "kodlama", "--dry-run"]).c,
            LayoutCmd::Restore { dry_run: true, .. }
        ));
        assert!(matches!(
            T::parse_from(["t", "list"]).c,
            LayoutCmd::List { json: false }
        ));
        let _ = localize(
            <T as clap::CommandFactory>::command(),
            &Localizer::new(kor_i18n::Lang::Tr),
        );
    }
}
