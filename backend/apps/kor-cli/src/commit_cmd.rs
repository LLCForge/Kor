//! `kor commit` and `kor changelog` (§15.5). Logic lives in `kor_agent::gitflow`; this module
//! asks, prints and confirms.

use clap::Args;
use kor_agent::gitflow::{self, FlowError};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // independent command-line flags
pub struct CommitArgs {
    /// Propose several logical commits instead of one (nothing is committed).
    #[arg(long)]
    pub split: bool,
    /// Commit without asking.
    #[arg(long, short)]
    pub yes: bool,
    /// Print the message only (used by the prepare-commit-msg hook).
    #[arg(long, conflicts_with_all = ["split", "yes", "install_hook"])]
    pub message_only: bool,
    /// Install Kor as this repository's prepare-commit-msg hook.
    #[arg(long)]
    pub install_hook: bool,
    #[arg(short, long)]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChangelogArgs {
    /// Start after this tag or revision (default: the latest tag).
    #[arg(long)]
    pub since: Option<String>,
    /// Write the section into CHANGELOG.md instead of printing it.
    #[arg(long)]
    pub write: bool,
}

fn flow_err(l: &Localizer, e: &FlowError) -> bool {
    let msg = match e {
        FlowError::GitMissing => l.t("flow-git-missing"),
        FlowError::NotRepo => l.t("flow-not-repo"),
        FlowError::NothingStaged => l.t("commit-nothing-staged"),
        FlowError::Git(e) => l.t_args("flow-git-failed", &args!("error" => e.clone())),
        FlowError::Model(e) | FlowError::BadReply(e) => {
            l.t_args("flow-model-failed", &args!("error" => e.clone()))
        }
        FlowError::HookExists(p) => l.t_args(
            "commit-hook-exists",
            &args!("path" => kor_tools::display_path(p)),
        ),
    };
    eprintln!("{msg}");
    false
}

pub fn commit(ctx: &Ctx, a: &CommitArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cwd = std::env::current_dir()?;
    if a.install_hook {
        return Ok(match gitflow::install_hook(&cwd) {
            Ok(p) => {
                println!(
                    "{}",
                    l.t_args(
                        "commit-hook-installed",
                        &args!("path" => kor_tools::display_path(&p))
                    )
                );
                true
            }
            Err(e) => flow_err(l, &e),
        });
    }
    // Fail on git problems before choosing a model.
    if let Err(e) = gitflow::repo_root(&cwd).and_then(|r| gitflow::staged(&r)) {
        return Ok(flow_err(l, &e));
    }
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
    let (provider, model) = match kor_runtime::oneshot_provider(l, loaded, a.model.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Ok(false);
        }
    };
    if !a.message_only {
        eprintln!(
            "{}",
            l.t_args(
                "flow-sending-diff",
                &args!("provider" => provider.manifest().name.clone())
            )
        );
    }
    let prop =
        crate::providers::runtime()?.block_on(gitflow::propose(&provider, &model, &cwd, a.split));
    let prop = match prop {
        Ok(p) => p,
        Err(e) => return Ok(flow_err(l, &e)),
    };
    if a.message_only {
        print!("{}", prop.message.render());
        return Ok(true);
    }
    if a.split {
        println!(
            "{}\n",
            l.t_args("commit-split-title", &args!("count" => prop.parts.len()))
        );
        for (i, p) in prop.parts.iter().enumerate() {
            println!("{}. {}", i + 1, p.files.join(", "));
            for line in p.message.render().lines() {
                println!("   {line}");
            }
            println!();
        }
        return Ok(true);
    }
    println!("{}\n\n{}", l.t("commit-proposed"), prop.message.render());
    if !crate::shell_cmd::confirm(l, &l.t("commit-confirm"), a.yes) {
        return Ok(false);
    }
    let root = gitflow::repo_root(&cwd);
    Ok(
        match root.and_then(|r| gitflow::commit(&r, &prop.message.render())) {
            Ok(summary) => {
                println!("{}", l.t_args("commit-done", &args!("summary" => summary)));
                true
            }
            Err(e) => flow_err(l, &e),
        },
    )
}

pub fn changelog(ctx: &Ctx, a: &ChangelogArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = match gitflow::repo_root(&std::env::current_dir()?) {
        Ok(r) => r,
        Err(e) => return Ok(flow_err(l, &e)),
    };
    let since = a.since.clone().or_else(|| gitflow::latest_tag(&root));
    let entries = match gitflow::log(&root, since.as_deref()) {
        Ok(e) => e,
        Err(e) => return Ok(flow_err(l, &e)),
    };
    if entries.is_empty() {
        println!("{}", l.t("changelog-empty"));
        return Ok(true);
    }
    let section = gitflow::section(&entries, "Unreleased");
    if !a.write {
        print!("{section}");
        return Ok(true);
    }
    let path = root.join("CHANGELOG.md");
    let shown = kor_tools::display_path(&path);
    let old = match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("error-generic", &args!("error" => format!("{shown}: {e}")))
            );
            return Ok(false);
        }
    };
    if let Err(e) = std::fs::write(&path, gitflow::merge_changelog(old.as_deref(), &section)) {
        eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => format!("{shown}: {e}")))
        );
        return Ok(false);
    }
    println!("{}", l.t_args("changelog-written", &args!("path" => shown)));
    Ok(true)
}
