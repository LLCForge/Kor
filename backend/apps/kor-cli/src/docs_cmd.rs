//! `kor docs add|list|search|refresh|rm` (Master Build Prompt §28.7, ADR 0063): private documents
//! (PDF, DOCX, Markdown, HTML, folders of Confluence/Notion exports, intranet URLs) indexed into
//! the knowledge base for the agent. Logic: `kor_runtime::docs`.

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_runtime::docs;

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum DocsCmd {
    /// Index a file, a folder or a URL.
    Add {
        source: String,
        /// Read the URL through your signed-in Chrome (paired Kor extension).
        #[arg(long)]
        browser: bool,
    },
    /// Indexed documents.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Search the documents.
    Search {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 5)]
        limit: usize,
    },
    /// Re-index documents whose content changed (all, or one id).
    Refresh { id: Option<String> },
    /// Remove a document and its index entries.
    Rm { id: String },
    /// Apply the docs page open in the browser to this project (§27.11).
    Use(crate::docs_web_cmd::UseArgs),
    /// Cached docs pages (§27.11).
    Cache(crate::docs_web_cmd::CacheArgs),
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-docs"))
        .mut_subcommand("add", |s| {
            s.about(l.t("cmd-docs-add"))
                .mut_arg("source", |a| a.help(l.t("arg-docs-source")))
                .mut_arg("browser", |a| a.help(l.t("arg-docs-browser")))
        })
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-docs-list"))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
        .mut_subcommand("search", |s| s.about(l.t("cmd-docs-search")))
        .mut_subcommand("refresh", |s| s.about(l.t("cmd-docs-refresh")))
        .mut_subcommand("rm", |s| s.about(l.t("cmd-docs-rm")))
        .mut_subcommand("use", |s| crate::docs_web_cmd::localize_use(s, l))
        .mut_subcommand("cache", |s| crate::docs_web_cmd::localize_cache(s, l))
}

pub fn run(ctx: &Ctx, cmd: &DocsCmd) -> anyhow::Result<bool> {
    match cmd {
        DocsCmd::Use(a) => return crate::docs_web_cmd::use_page(ctx, a),
        DocsCmd::Cache(a) => return crate::docs_web_cmd::cache(ctx, a),
        _ => {}
    }
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    if let DocsCmd::List { json } = cmd {
        let d = docs::list(&root);
        if *json {
            println!("{}", serde_json::to_string_pretty(&d)?);
        } else if d.is_empty() {
            println!("{}", l.t("docs-none"));
        } else {
            for e in &d {
                println!(
                    "{}  {}  {}  ({})",
                    e.id,
                    e.title,
                    e.source,
                    l.t_args(
                        "docs-chunks",
                        &args!("n" => e.records.len(), "chars" => e.chars)
                    )
                );
            }
        }
        return Ok(true);
    }
    // Privacy mode and the network guard apply to URL fetches.
    if let Ok(c) = &ctx.config {
        kor_runtime::apply_network_policy(l, &c.config);
    }
    let Some(home) = kor_runtime::kor_home() else {
        eprintln!("{}", l.t("memory-no-home"));
        return Ok(false);
    };
    let kb = kor_memory::open_knowledge(&home).map_err(anyhow::Error::msg)?;
    let rt = crate::providers::runtime()?;
    let fail = |e: String| {
        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
        false
    };
    match cmd {
        // Use/Cache returned at the top of `run`.
        DocsCmd::List { .. } | DocsCmd::Use(_) | DocsCmd::Cache(_) => Ok(true),
        DocsCmd::Add { source, browser } => {
            match rt.block_on(docs::add(&kb, &root, source, *browser)) {
                Ok(e) => {
                    println!("{}", l.t_args("docs-added", &args!("id" => e.id, "n" => e.records.len(), "semantic" => kb.semantic().to_string())));
                    Ok(true)
                }
                Err(e) => Ok(fail(e)),
            }
        }
        DocsCmd::Search { query, limit } => {
            match docs::search(&kb, &root, &query.join(" "), (*limit).clamp(1, 50)) {
                Ok(hits) if hits.is_empty() => {
                    println!("{}", l.t("recall-none"));
                    Ok(true)
                }
                Ok(hits) => {
                    for h in hits {
                        println!("{}", h.record.task);
                        for line in h
                            .record
                            .summary
                            .lines()
                            .filter(|x| !x.trim().is_empty())
                            .take(4)
                        {
                            println!("    {line}");
                        }
                    }
                    Ok(true)
                }
                Err(e) => Ok(fail(e)),
            }
        }
        DocsCmd::Refresh { id } => match rt.block_on(docs::refresh(&kb, &root, id.as_deref())) {
            Ok(rows) => {
                for (id, r) in rows {
                    match r {
                        Ok(true) => println!("{}", l.t_args("docs-reindexed", &args!("id" => id))),
                        Ok(false) => println!("{}", l.t_args("docs-unchanged", &args!("id" => id))),
                        Err(e) => eprintln!("{id}: {e}"),
                    }
                }
                Ok(true)
            }
            Err(e) => Ok(fail(e)),
        },
        DocsCmd::Rm { id } => match docs::remove(&kb, &root, id) {
            Ok(()) => {
                println!("{}", l.t_args("docs-removed", &args!("id" => id.clone())));
                Ok(true)
            }
            Err(e) => Ok(fail(e)),
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
        cmd: DocsCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            Cli::try_parse_from(["d", "add", "https://intranet/wiki", "--browser"])
                .unwrap()
                .cmd,
            DocsCmd::Add { browser: true, .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["d", "refresh"]).unwrap().cmd,
            DocsCmd::Refresh { id: None }
        ));
        assert!(Cli::try_parse_from(["d", "search"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
