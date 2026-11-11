//! `kor know "<query>" | digest | stale` (Master Build Prompt §27.15, ADR 0063): search the
//! personal knowledge base directly (outdated notes marked), the weekly digest of the most
//! referenced notes, and the notes whose files or symbols changed or vanished. Logic:
//! `kor_memory::kb` and `kor_memory::digest`.

use clap::{Args, Subcommand};
use kor_i18n::{Localizer, args};
use kor_memory::digest;

use crate::Ctx;

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct KnowArgs {
    #[command(subcommand)]
    pub command: Option<KnowSub>,
    /// What to look for ("docker build cache").
    pub query: Vec<String>,
    /// Only this project's notes.
    #[arg(long)]
    pub here: bool,
    #[arg(short = 'n', long, default_value_t = 10)]
    pub limit: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum KnowSub {
    /// The weekly digest: most referenced notes, new notes, outdated ones.
    Digest {
        /// Week as 2026-W45 (default: the last complete week).
        #[arg(long)]
        week: Option<String>,
        /// Print it instead of writing `<kor home>/knowledge/digest/<week>.md`.
        #[arg(long)]
        print: bool,
    },
    /// Notes that refer to files or symbols that changed or are gone.
    Stale {
        /// Only this project's notes.
        #[arg(long)]
        here: bool,
        #[arg(short = 'n', long, default_value_t = 500)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-know"))
        .mut_arg("query", |a| a.help(l.t("arg-know-query")))
        .mut_arg("here", |a| a.help(l.t("arg-know-here")))
        .mut_arg("limit", |a| a.help(l.t("arg-know-limit")))
        .mut_arg("json", |a| a.help(l.t("arg-json")))
        .mut_subcommand("digest", |s| {
            s.about(l.t("cmd-know-digest"))
                .mut_arg("week", |a| a.help(l.t("arg-know-week")))
                .mut_arg("print", |a| a.help(l.t("arg-know-print")))
        })
        .mut_subcommand("stale", |s| {
            s.about(l.t("cmd-know-stale"))
                .mut_arg("here", |a| a.help(l.t("arg-know-here")))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("memory-error", &args!("error" => e.to_string()))
    );
    false
}

/// Monday 00:00 UTC of `2026-W45`.
pub fn parse_week(s: &str) -> Option<i64> {
    let (y, w) = s.split_once("-W")?;
    let d =
        chrono::NaiveDate::from_isoywd_opt(y.parse().ok()?, w.parse().ok()?, chrono::Weekday::Mon)?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

fn here() -> String {
    kor_memory::project_key(&kor_runtime::project_root().unwrap_or_default())
}

pub fn run(ctx: &Ctx, a: &KnowArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(home) = kor_runtime::kor_home() else {
        eprintln!("{}", l.t("memory-no-home"));
        return Ok(false);
    };
    let kb = match kor_memory::open_knowledge(&home) {
        Ok(k) => k,
        Err(e) => return Ok(fail(l, e)),
    };
    match &a.command {
        Some(KnowSub::Digest { week, print }) => {
            let from = match week {
                Some(w) => match parse_week(w) {
                    Some(f) => f,
                    None => {
                        return Ok(fail(
                            l,
                            l.t_args("know-bad-week", &args!("week" => w.clone())),
                        ));
                    }
                },
                None => digest::week_start(chrono::Utc::now().timestamp()) - 7 * 86_400,
            };
            if *print {
                let d = digest::digest(&kb, &home, from).map_err(anyhow::Error::msg)?;
                println!("{}", digest::markdown(&d));
            } else {
                let p = digest::write_week(&kb, &home, from).map_err(anyhow::Error::msg)?;
                println!(
                    "{}",
                    l.t_args(
                        "know-digest-written",
                        &args!("path" => kor_tools::display_path(&p))
                    )
                );
            }
            Ok(true)
        }
        Some(KnowSub::Stale {
            here: only,
            limit,
            json,
        }) => {
            let project = only.then(here);
            let s = digest::stale(&kb, project.as_deref(), (*limit).clamp(1, 10_000))
                .map_err(anyhow::Error::msg)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else if s.is_empty() {
                println!("{}", l.t("know-stale-none"));
            } else {
                for x in &s {
                    println!("#{}  {}", x.id, x.task);
                    for r in &x.reasons {
                        println!("    ~ {}", digest::reason_text(r));
                    }
                }
            }
            Ok(true)
        }
        None => {
            let query = a.query.join(" ");
            if query.trim().is_empty() {
                eprintln!("{}", l.t("know-empty-query"));
                return Ok(false);
            }
            let me = here();
            let only = a.here.then_some(me.as_str());
            let hits = kb
                .recall(&query, only, Some(&me), a.limit.clamp(1, 100))
                .map_err(anyhow::Error::msg)?;
            let _ =
                digest::record_uses(&home, &hits.iter().map(|h| h.record.id).collect::<Vec<_>>());
            if a.json {
                let rows: Vec<serde_json::Value> = hits
                    .iter()
                    .map(|h| serde_json::json!({ "hit": h, "stale": digest::staleness(&h.record) }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(true);
            }
            if hits.is_empty() {
                println!("{}", l.t("recall-none"));
            }
            for h in &hits {
                let r = &h.record;
                let project = r.project.rsplit('/').next().unwrap_or(&r.project);
                let stale = digest::staleness(r)
                    .map(|s| {
                        format!(
                            "  ({})",
                            l.t_args(
                                "know-stale-mark",
                                &args!("why" => digest::reason_text(&s.reasons[0]))
                            )
                        )
                    })
                    .unwrap_or_default();
                println!(
                    "#{}  {}  {project}  {}{stale}",
                    r.id,
                    kor_memory::kb::date(r.ts),
                    r.task
                );
                for line in r.summary.lines().take(3) {
                    println!("    {line}");
                }
                for e in r.errors.iter().take(2) {
                    println!("    ! {e}");
                }
                for f in r.files.iter().take(3) {
                    println!("    @ {f}");
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
        #[command(flatten)]
        a: KnowArgs,
    }

    #[test]
    fn parses_query_and_subcommands() {
        let c = Cli::try_parse_from(["know", "docker", "build", "cache", "--here"]).unwrap();
        assert_eq!(c.a.query.join(" "), "docker build cache");
        assert!(c.a.here);
        let c = Cli::try_parse_from(["know", "digest", "--week", "2026-W45"]).unwrap();
        assert!(
            matches!(c.a.command, Some(KnowSub::Digest { ref week, .. }) if week.as_deref() == Some("2026-W45"))
        );
        assert!(matches!(
            Cli::try_parse_from(["know", "stale", "--json"])
                .unwrap()
                .a
                .command,
            Some(KnowSub::Stale { json: true, .. })
        ));
        let monday = parse_week("2026-W45").unwrap();
        assert_eq!(digest::week_start(monday + 3600), monday);
        assert!(parse_week("2026-45").is_none());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
