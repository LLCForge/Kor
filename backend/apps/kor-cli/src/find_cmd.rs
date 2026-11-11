//! `kor find "<question>"` (spec §8.9, ADR 0042). The search is `kor_agent::find`; this module
//! prints the places, the graph links and, unless `--no-model`, the model's explanation.

use kor_agent::find::{self, Found, TextIndex};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct FindArgs {
    #[arg(required = true, num_args = 1..)]
    question: Vec<String>,
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=50))]
    limit: u8,
    #[arg(long)]
    no_model: bool,
    #[arg(long)]
    json: bool,
    #[arg(short, long)]
    model: Option<String>,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-find"))
        .mut_arg("question", |a| a.help(l.t("arg-find-question")))
        .mut_arg("limit", |a| a.help(l.t("arg-find-limit")))
        .mut_arg("no_model", |a| a.help(l.t("arg-find-no-model")))
        .mut_arg("json", |a| a.help(l.t("arg-find-json")))
        .mut_arg("model", |a| a.help(l.t("arg-find-model")))
}

fn print_found(l: &Localizer, f: &Found) {
    let key = match f.text_index {
        TextIndex::Semantic => "find-index-semantic",
        TextIndex::Keyword => "find-index-keyword",
    };
    eprintln!("{}", l.t_args(key, &args!("nodes" => f.graph_nodes)));
    for n in &f.notes {
        eprintln!(
            "{}",
            l.t_args("find-semantic-failed", &args!("error" => n.clone()))
        );
    }
    if f.hits.is_empty() {
        println!("{}", l.t("find-none"));
        return;
    }
    println!("{}", l.t("find-places"));
    for (i, h) in f.hits.iter().enumerate() {
        let src: Vec<&str> = h
            .sources
            .iter()
            .map(|s| match s {
                find::Source::Graph => "graph",
                find::Source::Semantic => "semantic",
                find::Source::Keyword => "keyword",
            })
            .collect();
        println!("{:>3}. {}  [{}]", i + 1, find::label(h), src.join(", "));
    }
    if !f.links.is_empty() {
        println!("\n{}", l.t("find-flow"));
        for link in &f.links {
            println!("  {}", find::link_line(link));
        }
    }
}

pub fn run(ctx: &Ctx, a: &FindArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let question = a.question.join(" ");
    let root = crate::graph_cmd::root()?;
    let mut tctx = kor_tools::ToolCtx::new(&root)?;
    tctx.load_ai_ignore();
    // The local model's index is the one `file_search` keeps (nothing leaves the machine).
    let semantic = kor_agent::semantic::local_embedder()
        .zip(kor_runtime::kor_home())
        .map(|(m, home)| kor_agent::semantic::FileSearch::new(m, &home.join("index"), &tctx.root));
    let rt = crate::providers::runtime()?;
    let found = match rt.block_on(find::search(
        &tctx,
        &question,
        a.limit.into(),
        semantic.as_ref(),
    )) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            return Ok(false);
        }
    };
    if !a.json {
        print_found(l, &found);
    }
    let mut explanation: Option<String> = None;
    if !a.no_model && !found.hits.is_empty() {
        let provider = ctx
            .config
            .as_ref()
            .map_err(ToString::to_string)
            .and_then(|loaded| kor_runtime::oneshot_provider(l, loaded, a.model.as_deref()));
        match provider {
            Err(e) => eprintln!("\n{}", l.t_args("find-no-model", &args!("error" => e))),
            Ok((provider, model)) => {
                eprintln!(
                    "\n{}",
                    l.t_args(
                        "find-sending",
                        &args!("count" => found.hits.len(), "provider" => provider.manifest().name.clone())
                    )
                );
                match rt.block_on(find::explain(&provider, &model, &tctx.root, &found)) {
                    Ok(text) => explanation = Some(text),
                    Err(e) => eprintln!(
                        "{}",
                        l.t_args("find-model-failed", &args!("error" => e.to_string()))
                    ),
                }
            }
        }
    }
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "found": found, "explanation": explanation })
            )?
        );
    } else if let Some(text) = &explanation {
        println!("\n{text}");
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
        Find(FindArgs),
    }

    #[test]
    fn parses_a_question_in_several_words() {
        let Sub::Find(a) =
            Cli::try_parse_from(["kor", "find", "kullanıcı", "giriş", "yapınca", "--no-model"])
                .unwrap()
                .cmd;
        assert_eq!(a.question.join(" "), "kullanıcı giriş yapınca");
        assert!(a.no_model && !a.json && a.limit == 10);
        assert!(Cli::try_parse_from(["kor", "find"]).is_err());
        assert!(Cli::try_parse_from(["kor", "find", "x", "--limit", "0"]).is_err());
        for lang in kor_i18n::Lang::ALL {
            let l = Localizer::new(lang);
            for key in [
                "cmd-find",
                "arg-find-question",
                "find-index-semantic",
                "find-sending",
                "find-no-model",
            ] {
                assert_ne!(l.t(key), key, "{lang:?} {key}");
            }
        }
    }
}
