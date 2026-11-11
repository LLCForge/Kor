//! `kor clean --suggest` and `kor clean <id>…` (§19.3, ADR 0059): what can be cleaned, with sizes,
//! and deletion of the chosen items after a confirmation. Only recognised cache folders are ever
//! deleted (`kor_health::clean`).

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;

use kor_health::clean::{self, Scan, human};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct CleanArgs {
    /// List what can be cleaned and how big it is; deletes nothing.
    #[arg(long)]
    pub suggest: bool,
    /// Ids from `--suggest` to delete.
    pub ids: Vec<String>,
    /// Delete everything `--suggest` lists.
    #[arg(long, conflicts_with = "ids")]
    pub all: bool,
    /// Project folder to search for build caches (default: the current folder).
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub json: bool,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-clean"))
        .mut_arg("suggest", |a| a.help(l.t("arg-clean-suggest")))
        .mut_arg("ids", |a| a.help(l.t("arg-clean-ids")))
        .mut_arg("all", |a| a.help(l.t("arg-clean-all")))
        .mut_arg("path", |a| a.help(l.t("arg-clean-path")))
        .mut_arg("yes", |a| a.help(l.t("arg-clean-yes")))
        .mut_arg("json", |a| a.help(l.t("arg-json")))
}

fn label(c: &clean::Candidate) -> String {
    c.path
        .as_ref()
        .map_or_else(|| "Docker".to_owned(), |p| p.display().to_string())
}

fn confirm(l: &Localizer, n: usize, total: u64) -> bool {
    eprint!(
        "{} ",
        l.t_args(
            "clean-confirm",
            &args!("count" => n, "size" => human(total))
        )
    );
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(
        line.trim().to_lowercase().as_str(),
        "y" | "yes" | "e" | "evet" | "o" | "oui"
    )
}

pub fn run(ctx: &Ctx, a: &CleanArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = match &a.path {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let scan = Scan::standard(kor_health::health::cache_roots(Some(&root)));
    if a.suggest || (a.ids.is_empty() && !a.all) {
        let list = clean::suggest(&scan);
        if a.json {
            println!("{}", serde_json::to_string_pretty(&list)?);
            return Ok(true);
        }
        if list.is_empty() {
            println!("{}", l.t("clean-nothing"));
            return Ok(true);
        }
        for c in &list {
            println!(
                "{}  {:>10}  {:<14} {}",
                c.id,
                format!("{}{}", if c.approx { "≥" } else { "" }, human(c.bytes)),
                c.kind,
                label(c)
            );
        }
        let total: u64 = list.iter().map(|c| c.bytes).sum();
        println!(
            "{}",
            l.t_args(
                "clean-total",
                &args!("size" => human(total), "count" => list.len())
            )
        );
        println!("{}", l.t("clean-how"));
        return Ok(true);
    }
    let list = clean::suggest(&scan);
    let chosen: Vec<&clean::Candidate> = if a.all {
        list.iter().collect()
    } else {
        list.iter().filter(|c| a.ids.contains(&c.id)).collect()
    };
    for id in &a.ids {
        if !list.iter().any(|c| &c.id == id) {
            eprintln!("{}", l.t_args("clean-unknown", &args!("id" => id.clone())));
        }
    }
    if chosen.is_empty() {
        println!("{}", l.t("clean-nothing"));
        return Ok(a.ids.is_empty());
    }
    for c in &chosen {
        println!("  {}  {}", human(c.bytes), label(c));
    }
    let total: u64 = chosen.iter().map(|c| c.bytes).sum();
    if !a.yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("{}", l.t("clean-needs-yes"));
            return Ok(false);
        }
        if !confirm(l, chosen.len(), total) {
            println!("{}", l.t("clean-aborted"));
            return Ok(false);
        }
    }
    let ids: Vec<String> = chosen.iter().map(|c| c.id.clone()).collect();
    let out = clean::clean(&scan, &ids);
    let mut ok = true;
    for o in &out {
        let what = o
            .path
            .as_ref()
            .map_or_else(|| "Docker".to_owned(), |p| p.display().to_string());
        match &o.error {
            None => println!(
                "{}",
                l.t_args(
                    "clean-freed",
                    &args!("path" => what, "size" => human(o.freed))
                )
            ),
            Some(e) => {
                ok = false;
                eprintln!(
                    "{}",
                    l.t_args("clean-failed", &args!("path" => what, "error" => e.clone()))
                );
            }
        }
    }
    println!(
        "{}",
        l.t_args(
            "clean-done",
            &args!("size" => human(out.iter().map(|o| o.freed).sum()))
        )
    );
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: CleanArgs,
    }

    #[test]
    fn parses() {
        let t = T::parse_from(["t", "--suggest"]);
        assert!(t.a.suggest && t.a.ids.is_empty());
        let t = T::parse_from(["t", "ab12cd34ef", "--yes"]);
        assert_eq!(t.a.ids, ["ab12cd34ef"]);
        assert!(T::try_parse_from(["t", "x", "--all"]).is_err());
        let _ = localize(
            <T as clap::CommandFactory>::command(),
            &Localizer::new(kor_i18n::Lang::En),
        );
    }
}
