//! `kor diagram sync|watch|check|list|add` (Master Build Prompt §27.14, ADR 0063): marked
//! Mermaid / PlantUML / D2 blocks in the docs follow the project graph. `check` exits non-zero
//! when a diagram is stale (for CI). Logic: `kor_runtime::diagram_sync`.

use std::collections::BTreeMap;
use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::Path;

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_runtime::diagram_sync::{self as ds, Change, Format, Kind, Status};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum DiagramSub {
    /// Regenerate stale diagram blocks from the graph.
    Sync {
        /// Also overwrite blocks whose generated part was edited by hand.
        #[arg(long)]
        force: bool,
        /// Show what would change, write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Keep diagrams in sync while the code changes (Ctrl+C stops).
    Watch,
    /// Fail when a diagram does not match the code (CI).
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Every marked diagram block and its state, with an ASCII summary.
    List {
        /// Print each diagram's nodes and edges as text.
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        json: bool,
    },
    /// Add an empty marked block to a doc (filled by the next sync).
    Add {
        file: String,
        /// architecture, dependency, sequence, er or deploy.
        #[arg(long, short)]
        kind: String,
        /// mermaid, plantuml or d2 (Markdown files; diagram files use their own).
        #[arg(long, default_value = "mermaid")]
        format: String,
        /// Extra parameters: scope=src depth=2 max=40 from=main.
        #[arg(long = "param", short = 'p')]
        params: Vec<String>,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.mut_subcommand("sync", |s| {
        s.about(l.t("cmd-diagram-sync"))
            .mut_arg("force", |a| a.help(l.t("arg-diagram-force")))
            .mut_arg("dry_run", |a| a.help(l.t("arg-diagram-dry-run")))
    })
    .mut_subcommand("watch", |s| s.about(l.t("cmd-diagram-watch")))
    .mut_subcommand("check", |s| {
        s.about(l.t("cmd-diagram-check"))
            .mut_arg("json", |a| a.help(l.t("arg-json")))
    })
    .mut_subcommand("list", |s| {
        s.about(l.t("cmd-diagram-list"))
            .mut_arg("ascii", |a| a.help(l.t("arg-diagram-ascii")))
            .mut_arg("json", |a| a.help(l.t("arg-json")))
    })
    .mut_subcommand("add", |s| {
        s.about(l.t("cmd-diagram-add"))
            .mut_arg("file", |a| a.help(l.t("arg-diagram-file")))
            .mut_arg("kind", |a| a.help(l.t("arg-diagram-kind")))
            .mut_arg("format", |a| a.help(l.t("arg-diagram-format")))
            .mut_arg("params", |a| a.help(l.t("arg-diagram-param")))
    })
}

fn status_text(l: &Localizer, c: &Change) -> String {
    match c.status {
        Status::UpToDate => l.t("diagram-up-to-date"),
        Status::Stale => l.t("diagram-stale"),
        Status::Edited => l.t("diagram-edited"),
        Status::Invalid => l.t_args(
            "diagram-invalid",
            &args!("error" => c.error.clone().unwrap_or_default()),
        ),
    }
}

fn counts(l: &Localizer, c: &Change) -> String {
    let n = &c.counts;
    l.t_args(
        "diagram-counts",
        &args!("na" => n.nodes_added, "nr" => n.nodes_removed, "ea" => n.edges_added, "er" => n.edges_removed),
    )
}

/// Scan, regenerate from a fresh graph and plan.
fn plan(root: &Path) -> anyhow::Result<(Vec<ds::Block>, Vec<Change>)> {
    let g = crate::graph_cmd::open(root)?;
    let (blocks, bad) = ds::scan(root);
    let mut changes = ds::plan_in(root, &g, &blocks);
    changes.extend(bad);
    Ok((blocks, changes))
}

/// Ask about hand-edited blocks (TTY only): which ones to overwrite.
fn confirm_edited(l: &Localizer, changes: &mut [Change]) {
    if !std::io::stdin().is_terminal() {
        return;
    }
    for c in changes.iter_mut().filter(|c| c.status == Status::Edited) {
        print!(
            "{} ",
            l.t_args(
                "diagram-overwrite-ask",
                &args!("file" => c.file.clone(), "line" => c.line)
            )
        );
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().lock().read_line(&mut s);
        if matches!(s.trim(), "y" | "e" | "o" | "Y") {
            c.status = Status::Stale;
        }
    }
}

fn sync_once(
    l: &Localizer,
    root: &Path,
    force: bool,
    dry: bool,
    ask: bool,
) -> anyhow::Result<bool> {
    let (blocks, mut changes) = plan(root)?;
    if blocks.is_empty() && changes.is_empty() {
        println!("{}", l.t("diagram-none"));
        return Ok(true);
    }
    if ask && !force {
        confirm_edited(l, &mut changes);
    }
    for c in &changes {
        if c.status != Status::UpToDate {
            println!(
                "{}:{}  {}  {}",
                c.file,
                c.line,
                status_text(l, c),
                if c.status == Status::Stale {
                    counts(l, c)
                } else {
                    String::new()
                }
            );
        }
    }
    if dry {
        return Ok(true);
    }
    let written = ds::apply(root, &blocks, &changes, force).map_err(anyhow::Error::msg)?;
    // The summary line belongs in the PR description ("diagram updated: 2 nodes added").
    for c in changes
        .iter()
        .filter(|c| c.status == Status::Stale || (force && c.status == Status::Edited))
    {
        println!(
            "{}",
            l.t_args("diagram-updated", &args!("summary" => ds::summary_line(c)))
        );
    }
    if written.is_empty() {
        println!("{}", l.t("diagram-nothing"));
    }
    Ok(!changes.iter().any(|c| c.status == Status::Invalid))
}

/// §27.14 "the diagram change goes into the PR": before `kor pr create` opens a pull request,
/// stale diagram blocks are regenerated and committed on the branch (only the diagram files).
/// Returns the summary lines for the PR body; empty when nothing was stale or there are no blocks.
pub fn sync_for_pr(root: &Path) -> Vec<String> {
    let Ok((blocks, changes)) = plan(root) else {
        return Vec::new();
    };
    let stale: Vec<&Change> = changes
        .iter()
        .filter(|c| c.status == Status::Stale)
        .collect();
    if stale.is_empty() {
        return Vec::new();
    }
    let Ok(written) = ds::apply(root, &blocks, &changes, false) else {
        return Vec::new();
    };
    if written.is_empty() {
        return Vec::new();
    }
    let git = |args: &[&str]| kor_agent::gitflow::git(root, args).is_ok();
    let mut add = vec!["add", "--"];
    add.extend(written.iter().map(String::as_str));
    let mut commit = vec![
        "commit",
        "--no-verify",
        "-q",
        "-m",
        "docs: update diagrams from the code graph",
        "--",
    ];
    commit.extend(written.iter().map(String::as_str));
    if !(git(&add) && git(&commit)) {
        return Vec::new();
    }
    stale.iter().map(|c| ds::summary_line(c)).collect()
}

#[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
pub fn run(ctx: &Ctx, cmd: &DiagramSub) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    match cmd {
        DiagramSub::Sync { force, dry_run } => sync_once(l, &root, *force, *dry_run, true),
        DiagramSub::Check { json } => {
            let (_, changes) = plan(&root)?;
            let bad: Vec<&Change> = changes
                .iter()
                .filter(|c| c.status != Status::UpToDate)
                .collect();
            if *json {
                println!("{}", serde_json::to_string_pretty(&changes)?);
            } else {
                for c in &bad {
                    println!(
                        "{}:{}  {}  {}",
                        c.file,
                        c.line,
                        status_text(l, c),
                        counts(l, c)
                    );
                }
                println!(
                    "{}",
                    if bad.is_empty() {
                        l.t_args("diagram-check-ok", &args!("count" => changes.len()))
                    } else {
                        l.t_args("diagram-check-fail", &args!("count" => bad.len()))
                    }
                );
            }
            Ok(bad.is_empty())
        }
        DiagramSub::List { ascii, json } => {
            let (_, changes) = plan(&root)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&changes)?);
                return Ok(true);
            }
            if changes.is_empty() {
                println!("{}", l.t("diagram-none"));
            }
            for c in &changes {
                println!(
                    "{}:{}  {}  {}",
                    c.file,
                    c.line,
                    c.kind.map_or("?", Kind::id),
                    status_text(l, c)
                );
                if *ascii {
                    for a in &c.ascii {
                        println!("    {a}");
                    }
                }
            }
            Ok(true)
        }
        DiagramSub::Add {
            file,
            kind,
            format,
            params,
        } => {
            let Some(k) = Kind::parse(kind) else {
                eprintln!(
                    "{}",
                    l.t_args("diagram-bad-kind", &args!("kind" => kind.clone()))
                );
                return Ok(false);
            };
            let Some(f) = Format::parse(format) else {
                eprintln!(
                    "{}",
                    l.t_args("diagram-bad-format", &args!("format" => format.clone()))
                );
                return Ok(false);
            };
            let p: BTreeMap<String, String> = params
                .iter()
                .filter_map(|x| x.split_once('='))
                .filter(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .map(|(k, v)| (k.to_ascii_lowercase(), v.replace(char::is_whitespace, "")))
                .collect();
            ds::add(&root, file, k, f, &p).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args("diagram-added", &args!("file" => file.clone()))
            );
            sync_once(l, &root, false, false, false)
        }
        DiagramSub::Watch => {
            sync_once(l, &root, false, false, false)?;
            println!("{}", l.t("diagram-watching"));
            let (tx, rx) = std::sync::mpsc::channel();
            let _w = kor_graph::watch(&root, move |_| {
                let _ = tx.send(());
            })?;
            while rx.recv().is_ok() {
                while rx.try_recv().is_ok() {}
                if let Err(e) = sync_once(l, &root, false, false, false) {
                    eprintln!(
                        "{}",
                        l.t_args("error-generic", &args!("error" => e.to_string()))
                    );
                }
            }
            Ok(true)
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
        cmd: DiagramSub,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            Cli::try_parse_from(["d", "sync", "--force"]).unwrap().cmd,
            DiagramSub::Sync {
                force: true,
                dry_run: false
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["d", "check"]).unwrap().cmd,
            DiagramSub::Check { json: false }
        ));
        let Cli {
            cmd: DiagramSub::Add {
                file, kind, params, ..
            },
        } = Cli::try_parse_from(["d", "add", "docs/a.md", "-k", "sequence", "-p", "from=main"])
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (file.as_str(), kind.as_str(), params),
            ("docs/a.md", "sequence", vec!["from=main".to_owned()])
        );
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
