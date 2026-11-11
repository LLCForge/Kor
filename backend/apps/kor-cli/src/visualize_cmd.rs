//! `kor visualize [path] [--format html|mermaid|svg]` (spec §10, ADR 0048): architecture map,
//! call graph and data flow from the project graph, without a model. HTML opens in the browser.

use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_runtime::visual::map::{self, Format};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct VisualizeArgs {
    path: Option<PathBuf>,
    #[arg(long, default_value = "html", value_parser = ["html", "mermaid", "svg"])]
    format: String,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    no_open: bool,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-visualize"))
        .mut_arg("path", |a| a.help(l.t("arg-visualize-path")))
        .mut_arg("format", |a| a.help(l.t("arg-visualize-format")))
        .mut_arg("out", |a| a.help(l.t("arg-visualize-out")))
        .mut_arg("no_open", |a| a.help(l.t("arg-visualize-no-open")))
}

pub fn run(ctx: &Ctx, a: &VisualizeArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let format: Format = a.format.parse().map_err(anyhow::Error::msg)?;
    let root = crate::graph_cmd::root()?;
    let g = match crate::graph_cmd::open(&root) {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("visualize-graph-failed", &args!("error" => e.to_string()))
            );
            return Ok(false);
        }
    };
    let scope = map::scope_of(&root, a.path.as_deref());
    let views = map::views(&g, &scope);
    let dir = a.out.clone().unwrap_or_else(|| map::default_dir(&root));
    let w = match map::write(&views, format, &dir) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            return Ok(false);
        }
    };
    for (kind, nodes, edges, omitted) in &w.counts {
        println!(
            "{}",
            l.t_args(
                "visualize-view",
                &args!("title" => kind.title(), "nodes" => *nodes, "edges" => *edges, "omitted" => *omitted)
            )
        );
    }
    println!(
        "{}",
        l.t_args(
            "visualize-done",
            &args!("path" => kor_tools::display_path(&w.main))
        )
    );
    if format == Format::Html
        && !a.no_open
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
        && let Err(e) = crate::graph_cmd::open_in_browser(&w.main)
    {
        eprintln!(
            "{}",
            l.t_args("visualize-open-failed", &args!("error" => e.to_string()))
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Sub,
    }

    #[derive(clap::Subcommand)]
    enum Sub {
        Visualize(VisualizeArgs),
    }

    #[test]
    fn parses_path_and_format() {
        let Sub::Visualize(a) =
            Cli::try_parse_from(["kor", "visualize", "src", "--format", "svg", "--no-open"])
                .unwrap()
                .cmd;
        assert_eq!(a.path, Some(PathBuf::from("src")));
        assert_eq!(a.format, "svg");
        assert!(a.no_open);
        let Sub::Visualize(a) = Cli::try_parse_from(["kor", "visualize"]).unwrap().cmd;
        assert_eq!((a.path, a.format.as_str()), (None, "html"));
        assert!(Cli::try_parse_from(["kor", "visualize", "--format", "png"]).is_err());
    }
}
