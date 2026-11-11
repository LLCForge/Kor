//! `kor graph` (spec §7.1): the live project graph from the terminal. Everything is computed by
//! `kor_graph`; this module opens the graph, brings it up to date and prints.

use std::path::{Path, PathBuf};

use kor_graph::{Format, Graph, NodeRef};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct GraphArgs {
    #[command(subcommand)]
    command: Option<GraphCmd>,
    /// Write the HTML view and open it in the browser.
    #[arg(long)]
    html: bool,
}

#[derive(Debug, clap::Subcommand)]
pub enum GraphCmd {
    /// Nodes, edges, communities and the last update.
    Status,
    /// Source, community, degree, callers and callees of a symbol or file.
    Explain { name: String },
    /// Shortest path between two concepts.
    Path { from: String, to: String },
    /// Search the graph by words or identifiers.
    Query {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=200))]
        limit: u16,
    },
    /// High degree × high churn: the risk map.
    Hotspots {
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=200))]
        limit: u16,
    },
    /// How the graph changed since a git revision.
    Diff { rev: String },
    /// What depends on a symbol, file or package.
    Impact {
        target: String,
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=10))]
        depth: u32,
    },
    /// Export as Kor JSON, Mermaid, an Obsidian vault, HTML or graphify `graph.json`.
    #[command(group(clap::ArgGroup::new("format").required(true)
        .args(["json", "mermaid", "obsidian", "html", "graphify"])))]
    Export {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        mermaid: bool,
        #[arg(long)]
        obsidian: bool,
        #[arg(long)]
        html: bool,
        #[arg(long)]
        graphify: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Rescan the whole project now.
    Update,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-graph"))
        .mut_arg("html", |a| a.help(l.t("arg-graph-html")))
        .mut_subcommand("status", |s| s.about(l.t("cmd-graph-status")))
        .mut_subcommand("explain", |s| {
            s.about(l.t("cmd-graph-explain"))
                .mut_arg("name", |a| a.help(l.t("arg-graph-name")))
        })
        .mut_subcommand("path", |s| {
            s.about(l.t("cmd-graph-path"))
                .mut_arg("from", |a| a.help(l.t("arg-graph-name")))
                .mut_arg("to", |a| a.help(l.t("arg-graph-name")))
        })
        .mut_subcommand("query", |s| {
            s.about(l.t("cmd-graph-query"))
                .mut_arg("text", |a| a.help(l.t("arg-graph-text")))
                .mut_arg("limit", |a| a.help(l.t("arg-graph-limit")))
        })
        .mut_subcommand("hotspots", |s| {
            s.about(l.t("cmd-graph-hotspots"))
                .mut_arg("limit", |a| a.help(l.t("arg-graph-limit")))
        })
        .mut_subcommand("diff", |s| {
            s.about(l.t("cmd-graph-diff"))
                .mut_arg("rev", |a| a.help(l.t("arg-graph-rev")))
        })
        .mut_subcommand("impact", |s| {
            s.about(l.t("cmd-graph-impact"))
                .mut_arg("target", |a| a.help(l.t("arg-graph-name")))
                .mut_arg("depth", |a| a.help(l.t("arg-graph-depth")))
        })
        .mut_subcommand("export", |s| {
            s.about(l.t("cmd-graph-export"))
                .mut_arg("json", |a| a.help(l.t("arg-graph-export-json")))
                .mut_arg("mermaid", |a| a.help(l.t("arg-graph-export-mermaid")))
                .mut_arg("obsidian", |a| a.help(l.t("arg-graph-export-obsidian")))
                .mut_arg("html", |a| a.help(l.t("arg-graph-export-html")))
                .mut_arg("graphify", |a| a.help(l.t("arg-graph-export-graphify")))
                .mut_arg("out", |a| a.help(l.t("arg-graph-out")))
        })
        .mut_subcommand("update", |s| s.about(l.t("cmd-graph-update")))
}

/// The project root without the Windows `\\?\` prefix (the agent session uses the same form).
pub fn root() -> std::io::Result<PathBuf> {
    Ok(kor_tools::without_verbatim(&kor_runtime::project_root()?))
}

/// Open the graph at `root` and bring it up to date.
pub fn open(root: &Path) -> kor_graph::Result<Graph> {
    let mut g = Graph::open(root)?;
    g.update()?;
    Ok(g)
}

pub fn run(ctx: &Ctx, a: &GraphArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = root()?;
    if let Some(GraphCmd::Diff { rev }) = &a.command {
        return Ok(match kor_graph::diff(&root, rev) {
            Ok(d) => {
                print_diff(l, &d);
                true
            }
            Err(e) => fail(l, &e),
        });
    }
    let g = match open(&root) {
        Ok(g) => g,
        Err(e) => return Ok(fail(l, &e)),
    };
    Ok(match &a.command {
        None if a.html => export(l, &g, Format::Html, None),
        None => {
            print_status(l, &g);
            print_hotspots(l, &g, 10);
            println!("{}", l.t("graph-more-hint"));
            true
        }
        Some(GraphCmd::Status) => {
            print_status(l, &g);
            true
        }
        Some(GraphCmd::Update) => {
            // `open` already scanned; a second full update reports the (now empty) delta.
            let s = g.status();
            println!(
                "{}",
                l.t_args(
                    "graph-updated",
                    &args!("nodes" => s.nodes, "edges" => s.edges, "files" => s.files)
                )
            );
            true
        }
        Some(GraphCmd::Explain { name }) => explain(l, &g, name),
        Some(GraphCmd::Path { from, to }) => path(l, &g, from, to),
        Some(GraphCmd::Query { text, limit }) => query(l, &g, &text.join(" "), *limit),
        Some(GraphCmd::Hotspots { limit }) => {
            print_hotspots(l, &g, usize::from(*limit));
            true
        }
        Some(GraphCmd::Impact { target, depth }) => impact(l, &g, target, *depth),
        Some(GraphCmd::Export {
            json,
            mermaid,
            obsidian,
            html,
            out,
            ..
        }) => {
            let format = if *json {
                Format::Json
            } else if *mermaid {
                Format::Mermaid
            } else if *obsidian {
                Format::Obsidian
            } else if *html {
                Format::Html
            } else {
                Format::Graphify
            };
            export(l, &g, format, out.as_deref())
        }
        Some(GraphCmd::Diff { .. }) => true,
    })
}

fn fail(l: &Localizer, e: &kor_graph::Error) -> bool {
    eprintln!(
        "{}",
        l.t_args("graph-error", &args!("error" => e.to_string()))
    );
    false
}

/// `3 s`, `5 min`, `2 h`, `4 d` in the user's language.
pub fn ago(l: &Localizer, secs: u64) -> String {
    let (n, key) = match secs {
        0..60 => (secs, "graph-unit-s"),
        60..3600 => (secs / 60, "graph-unit-min"),
        3600..86_400 => (secs / 3600, "graph-unit-h"),
        _ => (secs / 86_400, "graph-unit-d"),
    };
    l.t_args(key, &args!("n" => n))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn print_status(l: &Localizer, g: &Graph) {
    let s = g.status();
    let when = s
        .last_update
        .map_or_else(|| "-".to_owned(), |t| ago(l, now().saturating_sub(t)));
    println!(
        "{}",
        l.t_args(
            "graph-status",
            &args!("nodes" => s.nodes, "edges" => s.edges, "communities" => s.communities,
                   "files" => s.files, "ago" => when)
        )
    );
    if !s.languages.is_empty() {
        let langs: Vec<String> = s
            .languages
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        println!(
            "{}",
            l.t_args("graph-languages", &args!("list" => langs.join(", ")))
        );
    }
    if let Some(ws) = g.workspace()
        && ws.is_monorepo()
    {
        println!(
            "{}",
            l.t_args("graph-packages", &args!("count" => ws.packages.len()))
        );
    }
}

fn node_line(n: &NodeRef) -> String {
    let loc = match (&n.file, n.line) {
        (Some(f), _) if f == &n.name => String::new(),
        (Some(f), Some(line)) => format!("  {f}:{line}"),
        (Some(f), None) => format!("  {f}"),
        _ => String::new(),
    };
    format!("{} {}{loc}", n.kind.as_str(), n.name)
}

fn print_hotspots(l: &Localizer, g: &Graph, limit: usize) {
    let hot = g.hotspots(limit);
    if hot.is_empty() {
        println!("{}", l.t("graph-none"));
        return;
    }
    println!("{}", l.t("graph-hotspots-head"));
    for h in hot {
        println!(
            "  {:>7}  {:>4} {:>4}  {}",
            h.score,
            h.degree,
            h.churn_30d,
            node_line(&h.node)
        );
    }
}

fn explain(l: &Localizer, g: &Graph, name: &str) -> bool {
    let Some(ex) = g.explain(name) else {
        println!("{}", l.t_args("graph-not-found", &args!("name" => name)));
        return false;
    };
    let n = &ex.node;
    println!("{}", node_line(&NodeRef::from(n)));
    let community = ex.community.as_ref().map_or_else(
        || n.community.to_string(),
        |c| format!("{} ({})", c.id, c.label),
    );
    println!(
        "{}",
        l.t_args(
            "graph-explain-meta",
            &args!("community" => community, "degree" => n.degree, "churn" => n.churn_30d)
        )
    );
    if n.kind == kor_graph::NodeKind::Table && !n.text.is_empty() {
        println!("{}", l.t("graph-explain-columns"));
        for c in n.text.lines() {
            println!("    {c}");
        }
    }
    if let Some(src) = &ex.source {
        println!("{}", l.t("graph-explain-source"));
        for line in src.lines() {
            println!("    {line}");
        }
    }
    for (key, links) in [
        ("graph-explain-callers", &ex.callers),
        ("graph-explain-callees", &ex.callees),
        ("graph-explain-cochange", &ex.co_changes),
    ] {
        if links.is_empty() {
            continue;
        }
        println!("{}", l.t_args(key, &args!("count" => links.len())));
        for k in links.iter().take(30) {
            println!(
                "    {}  [{} {}]",
                node_line(&k.node),
                k.kind.as_str(),
                k.confidence.as_str()
            );
        }
    }
    if !ex.decisions.is_empty() {
        println!("{}", l.t("graph-explain-decisions"));
        for d in &ex.decisions {
            println!("    {}", node_line(d));
        }
    }
    true
}

fn path(l: &Localizer, g: &Graph, from: &str, to: &str) -> bool {
    let Some(steps) = g.path(from, to) else {
        println!(
            "{}",
            l.t_args("graph-no-path", &args!("from" => from, "to" => to))
        );
        return false;
    };
    for s in steps {
        if let Some(e) = s.edge {
            println!("      {} {}", if s.forward { "↓" } else { "↑" }, e.as_str());
        }
        println!("  {}", node_line(&s.node));
    }
    true
}

fn query(l: &Localizer, g: &Graph, text: &str, limit: u16) -> bool {
    let hits = g.query(text, usize::from(limit));
    if hits.is_empty() {
        println!("{}", l.t_args("graph-not-found", &args!("name" => text)));
        return false;
    }
    for h in hits {
        let pkg = h
            .node
            .file
            .as_deref()
            .and_then(|f| g.package_of(f))
            .map(|p| format!("  [{}]", p.name))
            .unwrap_or_default();
        println!("  {:>5.1}  {}{pkg}", h.score, node_line(&h.node));
    }
    true
}

fn impact(l: &Localizer, g: &Graph, target: &str, depth: u32) -> bool {
    let imp = g.impact(target, depth);
    if imp.roots.is_empty() {
        println!("{}", l.t_args("graph-not-found", &args!("name" => target)));
        return false;
    }
    println!(
        "{}",
        l.t_args(
            "graph-impact-head",
            &args!("count" => imp.affected.len(), "files" => imp.files.len())
        )
    );
    for a in &imp.affected {
        println!(
            "  {}  {}  [{}]",
            a.depth,
            node_line(&a.node),
            a.via.as_str()
        );
    }
    if imp.truncated {
        println!("  …");
    }
    if let Some(ws) = g.workspace()
        && ws.is_monorepo()
    {
        let pkgs = ws.affected(&imp.files.iter().map(PathBuf::from).collect::<Vec<_>>());
        if !pkgs.is_empty() {
            let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
            println!(
                "{}",
                l.t_args("graph-impact-packages", &args!("list" => names.join(", ")))
            );
        }
    }
    true
}

fn print_diff(l: &Localizer, d: &kor_graph::GraphDiff) {
    println!(
        "{}",
        l.t_args(
            "graph-diff-head",
            &args!("rev" => d.rev.clone(), "added" => d.added_nodes.len(),
                   "removed" => d.removed_nodes.len(), "edges_added" => d.added_edges.len(),
                   "edges_removed" => d.removed_edges.len())
        )
    );
    for n in d.added_nodes.iter().take(40) {
        println!("  + {}", node_line(n));
    }
    for n in d.removed_nodes.iter().take(40) {
        println!("  - {}", node_line(n));
    }
}

/// Open a file with the user's default app (the HTML view in the browser).
pub fn open_in_browser(path: &Path) -> std::io::Result<()> {
    // Decided at run time (no `cfg`), like `kor_platform::os`.
    let program = match std::env::consts::OS {
        "windows" => "explorer",
        "macos" => "open",
        _ => "xdg-open",
    };
    std::process::Command::new(program)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

fn export(l: &Localizer, g: &Graph, format: Format, out: Option<&Path>) -> bool {
    let path = out.map_or_else(|| format.default_path(g.root()), Path::to_path_buf);
    match g.export(format, &path) {
        Ok(r) => {
            println!(
                "{}",
                l.t_args(
                    "graph-exported",
                    &args!("path" => r.path.display().to_string(), "files" => r.files_written,
                           "omitted" => r.nodes_omitted)
                )
            );
            if format == Format::Html
                && std::io::IsTerminal::is_terminal(&std::io::stdout())
                && let Err(e) = open_in_browser(&r.path)
            {
                eprintln!(
                    "{}",
                    l.t_args("graph-open-failed", &args!("error" => e.to_string()))
                );
            }
            true
        }
        Err(e) => fail(l, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, Parser)]
    struct T {
        #[command(flatten)]
        a: GraphArgs,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["t", "query", "payment", "error", "--limit", "5"]).unwrap();
        assert!(
            matches!(t.a.command, Some(GraphCmd::Query { ref text, limit: 5 }) if text.len() == 2)
        );
        assert!(
            T::try_parse_from(["t", "export"]).is_err(),
            "a format is required"
        );
        assert!(T::try_parse_from(["t", "export", "--json", "--html"]).is_err());
        let t = T::try_parse_from(["t", "export", "--graphify", "--out", "g.json"]).unwrap();
        assert!(matches!(
            t.a.command,
            Some(GraphCmd::Export { graphify: true, .. })
        ));
        assert!(T::try_parse_from(["t", "--html"]).unwrap().a.html);
        let l = Localizer::new(kor_i18n::Lang::Tr);
        localize(T::command(), &l).debug_assert();
        assert_eq!(ago(&Localizer::new(kor_i18n::Lang::En), 125), "2 min");
    }

    #[test]
    fn commands_run_on_a_fixture() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() { beta(); }\npub fn beta() {}\n",
        )
        .unwrap();
        let g = open(dir.path()).unwrap();
        let l = Localizer::new(kor_i18n::Lang::En);
        assert!(explain(&l, &g, "alpha"));
        assert!(!explain(&l, &g, "nothing_here"));
        assert!(path(&l, &g, "alpha", "beta"));
        assert!(query(&l, &g, "beta", 5));
        assert!(impact(&l, &g, "beta", 2));
        let out = dir.path().join("x.mmd");
        assert!(export(&l, &g, Format::Mermaid, Some(&out)));
        assert!(out.is_file());
    }
}
