//! `kor sessions fork|tree|compare|merge|unfork` (Master Build Prompt §27.3, ADR 0063): branch a
//! conversation at any message, see a session's branches as a tree, compare two branches, and
//! take a branch's conclusion back (its file changes reviewed file by file). Logic:
//! `kor_runtime::fork`.

use std::io::{BufRead as _, IsTerminal as _, Write as _};

use clap::Subcommand;
use kor_core::Role;
use kor_i18n::{Localizer, args};
use kor_runtime::fork::{self, Node};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum ForkCmd {
    /// Fork a session after message N into a new session (its own git worktree).
    Fork {
        id: String,
        /// Message number to fork after; without it the messages are listed.
        #[arg(long)]
        at: Option<usize>,
        /// Continue the branch with another model ("try this with …").
        #[arg(short = 'm', long)]
        model: Option<String>,
        /// Share the project folder instead of a worktree.
        #[arg(long)]
        no_worktree: bool,
        #[arg(long)]
        json: bool,
    },
    /// A session's branches as a tree (every tree without an id).
    Tree {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compare two branches: shared history, conclusions and file differences.
    Compare {
        a: String,
        b: String,
        /// Print the file diffs too.
        #[arg(long)]
        diff: bool,
        #[arg(long)]
        json: bool,
    },
    /// Take a branch's conclusion back into its parent; file changes are reviewed one by one.
    Merge {
        id: String,
        /// Accept every file without asking.
        #[arg(short, long)]
        yes: bool,
        /// Do not add the conclusion note to the parent.
        #[arg(long)]
        no_note: bool,
    },
    /// Forget a branch record and remove its worktree (the transcript stays).
    Unfork {
        id: String,
        #[arg(long)]
        keep_worktree: bool,
    },
}

/// Localized help for the fork subcommands of `kor sessions`.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let json = |s: clap::Command| s.mut_arg("json", |a| a.help(l.t("arg-json")));
    c.mut_subcommand("fork", |s| {
        json(s.about(l.t("cmd-sessions-fork")))
            .mut_arg("id", |a| a.help(l.t("arg-session-id")))
            .mut_arg("at", |a| a.help(l.t("arg-fork-at")))
            .mut_arg("model", |a| a.help(l.t("arg-fork-model")))
            .mut_arg("no_worktree", |a| a.help(l.t("arg-fork-no-worktree")))
    })
    .mut_subcommand("tree", |s| {
        json(s.about(l.t("cmd-sessions-tree"))).mut_arg("id", |a| a.help(l.t("arg-session-id")))
    })
    .mut_subcommand("compare", |s| {
        json(s.about(l.t("cmd-sessions-compare"))).mut_arg("diff", |a| a.help(l.t("arg-fork-diff")))
    })
    .mut_subcommand("merge", |s| {
        s.about(l.t("cmd-sessions-merge"))
            .mut_arg("yes", |a| a.help(l.t("arg-fork-yes")))
            .mut_arg("no_note", |a| a.help(l.t("arg-fork-no-note")))
    })
    .mut_subcommand("unfork", |s| {
        s.about(l.t("cmd-sessions-unfork"))
            .mut_arg("keep_worktree", |a| a.help(l.t("arg-fork-keep-worktree")))
    })
}

fn paths() -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let s = kor_runtime::sessions_dir().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    let f = fork::store_path().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    Ok((s, f))
}

fn fail(l: &Localizer, e: &dyn std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

fn print_tree(n: &Node, depth: usize, l: &Localizer) {
    let pad = "  ".repeat(depth);
    let branch = if depth == 0 {
        String::new()
    } else {
        "└─ ".into()
    };
    let mut line = format!(
        "{pad}{branch}{}  {}",
        n.id,
        if n.missing {
            l.t("fork-missing")
        } else {
            n.title.clone()
        }
    );
    if let Some(at) = n.at {
        line.push_str(&format!("  @{at}"));
    }
    if let Some(m) = &n.model {
        line.push_str(&format!("  [{m}]"));
    }
    if n.merged.is_some() {
        line.push_str(&format!("  ({})", l.t("fork-merged")));
    }
    println!("{line}");
    for c in &n.children {
        print_tree(c, depth + 1, l);
    }
}

fn role(r: Role) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
    }
}

/// Ask per file: accept, reject, accept all, quit. `None` = quit.
fn review(l: &Localizer, files: &[kor_tools::Proposal]) -> Option<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();
    let mut all = false;
    let stdin = std::io::stdin();
    for f in files {
        let d = kor_runtime::textdiff::unified(&f.path, f.before.as_deref(), f.after.as_deref());
        if !all {
            println!();
            for line in d.lines() {
                let c = if line.starts_with('+') && !line.starts_with("+++") {
                    format!("\x1b[32m{line}\x1b[0m")
                } else if line.starts_with('-') && !line.starts_with("---") {
                    format!("\x1b[31m{line}\x1b[0m")
                } else {
                    line.to_owned()
                };
                println!("{c}");
            }
        }
        let answer = if all {
            "a".to_owned()
        } else {
            print!(
                "{} ",
                l.t_args("fork-merge-ask", &args!("path" => f.path.clone()))
            );
            let _ = std::io::stdout().flush();
            let mut s = String::new();
            if stdin.lock().read_line(&mut s).unwrap_or(0) == 0 {
                return None;
            }
            s.trim().to_owned()
        };
        match answer.as_str() {
            "A" => {
                all = true;
                out.push((f.path.clone(), f.after.clone()));
            }
            "a" | "y" | "e" | "o" => out.push((f.path.clone(), f.after.clone())),
            "q" => return None,
            _ => {}
        }
    }
    Some(out)
}

pub fn run(ctx: &Ctx, cmd: &ForkCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let (sessions, store) = paths()?;
    match cmd {
        ForkCmd::Fork {
            id,
            at,
            model,
            no_worktree,
            json,
        } => {
            let Some(at) = *at else {
                // List the messages so the user can pick the fork point.
                let path = sessions.join(format!("{id}.jsonl"));
                if !kor_runtime::session_tree::valid_id(id) || !path.is_file() {
                    return Ok(fail(l, &format!("unknown session `{id}`")));
                }
                let msgs = kor_agent::session::load(&path)?;
                println!("{}", l.t("fork-pick"));
                for (i, m) in msgs.iter().enumerate() {
                    let first: String = m
                        .plain_text()
                        .lines()
                        .map(str::trim)
                        .find(|x| !x.is_empty())
                        .unwrap_or("")
                        .chars()
                        .take(90)
                        .collect();
                    println!("  {:>4}  {:<9} {first}", i + 1, role(m.role));
                }
                return Ok(true);
            };
            let root = crate::graph_cmd::root().ok();
            let undo = kor_runtime::undo_dir();
            let opts = fork::Options {
                model: model.clone(),
                root: root.as_deref(),
                no_worktree: *no_worktree,
                undo_base: undo.as_deref(),
            };
            let f = match fork::fork(&sessions, &store, id, at, &opts) {
                Ok(f) => f,
                Err(e) => return Ok(fail(l, &e)),
            };
            if *json {
                println!("{}", serde_json::to_string_pretty(&f)?);
                return Ok(true);
            }
            println!(
                "{}",
                l.t_args(
                    "fork-created",
                    &args!("id" => f.id.clone(), "parent" => id.clone(), "at" => at)
                )
            );
            match &f.worktree {
                Some(wt) => println!(
                    "{}",
                    l.t_args(
                        "fork-worktree",
                        &args!("path" => kor_tools::display_path(wt))
                    )
                ),
                None => println!("{}", l.t("fork-no-worktree")),
            }
            if !f.rewound.is_empty() {
                println!(
                    "{}",
                    l.t_args(
                        "fork-rewound",
                        &args!("count" => f.rewound.len(), "files" => f.rewound.join(", "))
                    )
                );
            }
            if !f.unrestored.is_empty() {
                println!(
                    "{}",
                    l.t_args(
                        "fork-unrestored",
                        &args!("files" => f.unrestored.join(", "))
                    )
                );
            }
            if f.shell_after {
                println!("{}", l.t("fork-shell-after"));
            }
            let cd = f
                .worktree
                .as_ref()
                .map(|w| format!("cd \"{}\" && ", kor_tools::display_path(w)))
                .unwrap_or_default();
            let m = f
                .model
                .as_ref()
                .map(|m| format!(" -m {m}"))
                .unwrap_or_default();
            println!(
                "{}",
                l.t_args(
                    "fork-continue",
                    &args!("command" => format!("{cd}kor resume {}{m}", f.id))
                )
            );
            Ok(true)
        }
        ForkCmd::Tree { id, json } => {
            let meta = kor_runtime::session_tree::meta_path();
            let t = match fork::tree(&sessions, &store, meta.as_deref(), id.as_deref()) {
                Ok(t) => t,
                Err(e) => return Ok(fail(l, &e)),
            };
            if *json {
                println!("{}", serde_json::to_string_pretty(&t)?);
            } else if t.is_empty() {
                println!("{}", l.t("fork-none"));
            } else {
                for n in &t {
                    print_tree(n, 0, l);
                }
            }
            Ok(true)
        }
        ForkCmd::Compare { a, b, diff, json } => {
            let c = match fork::compare(&sessions, &store, a, b) {
                Ok(c) => c,
                Err(e) => return Ok(fail(l, &e)),
            };
            if *json {
                println!("{}", serde_json::to_string_pretty(&c)?);
                return Ok(true);
            }
            println!("{}", l.t_args("fork-common", &args!("n" => c.common)));
            for s in [&c.a, &c.b] {
                println!(
                    "\n■ {}  ({})",
                    s.id,
                    l.t_args("fork-after", &args!("n" => s.after))
                );
                if !s.conclusion.is_empty() {
                    println!(
                        "  {}",
                        s.conclusion
                            .lines()
                            .take(8)
                            .collect::<Vec<_>>()
                            .join("\n  ")
                    );
                }
                if !s.changed.is_empty() {
                    println!(
                        "  {}",
                        l.t_args("fork-changed", &args!("files" => s.changed.join(", ")))
                    );
                }
            }
            println!();
            if c.files.is_empty() {
                println!("{}", l.t("fork-same-files"));
            }
            for f in &c.files {
                println!("  {}  +{} −{}", f.path, f.added, f.removed);
                if *diff {
                    println!("{}", f.diff);
                }
            }
            Ok(true)
        }
        ForkCmd::Merge { id, yes, no_note } => {
            let plan = match fork::merge_plan(&sessions, &store, id) {
                Ok(p) => p,
                Err(e) => return Ok(fail(l, &e)),
            };
            println!(
                "{}",
                l.t_args(
                    "fork-merge-into",
                    &args!("fork" => plan.fork.clone(), "parent" => plan.parent.clone())
                )
            );
            for s in &plan.skipped {
                println!(
                    "  {}",
                    l.t_args("fork-merge-skipped", &args!("path" => s.clone()))
                );
            }
            let accepted = if plan.files.is_empty() {
                Vec::new()
            } else if *yes {
                plan.files
                    .iter()
                    .map(|f| (f.path.clone(), f.after.clone()))
                    .collect()
            } else if !std::io::stdin().is_terminal() {
                return Ok(fail(l, &l.t("fork-merge-needs-tty")));
            } else if let Some(a) = review(l, &plan.files) {
                a
            } else {
                println!("{}", l.t("fork-merge-cancelled"));
                return Ok(false);
            };
            let note = (!*no_note).then_some(plan.summary.as_str());
            let undo = kor_runtime::undo_dir();
            match fork::merge_apply(&sessions, &store, undo.as_deref(), id, &accepted, note) {
                Ok(r) => {
                    println!(
                        "{}",
                        l.t_args("fork-merged-done", &args!("written" => r.written.len(), "offered" => plan.files.len(), "parent" => plan.parent.clone()))
                    );
                    if !r.written.is_empty() {
                        println!(
                            "{}",
                            l.t_args("fork-merge-undo", &args!("parent" => plan.parent))
                        );
                    }
                    Ok(true)
                }
                Err(e) => Ok(fail(l, &e)),
            }
        }
        ForkCmd::Unfork { id, keep_worktree } => match fork::remove(&store, id, !*keep_worktree) {
            Ok(()) => {
                println!("{}", l.t_args("fork-removed", &args!("id" => id.clone())));
                Ok(true)
            }
            Err(e) => Ok(fail(l, &e)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: ForkCmd,
    }

    #[test]
    fn parses() {
        let Cli {
            cmd:
                ForkCmd::Fork {
                    id,
                    at,
                    model,
                    no_worktree,
                    ..
                },
        } = Cli::try_parse_from([
            "s",
            "fork",
            "20260101-000000-1",
            "--at",
            "4",
            "-m",
            "anthropic/claude",
        ])
        .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (id.as_str(), at, model.as_deref(), no_worktree),
            (
                "20260101-000000-1",
                Some(4),
                Some("anthropic/claude"),
                false
            )
        );
        assert!(matches!(
            Cli::try_parse_from(["s", "compare", "a", "b", "--diff"])
                .unwrap()
                .cmd,
            ForkCmd::Compare { diff: true, .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["s", "merge", "x", "-y"]).unwrap().cmd,
            ForkCmd::Merge { yes: true, .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["s", "tree"]).unwrap().cmd,
            ForkCmd::Tree { id: None, .. }
        ));
        assert!(Cli::try_parse_from(["s", "compare", "a"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
