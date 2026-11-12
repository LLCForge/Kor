//! `kor bench` (§5.1, ADR 0015): race models on the project's own test suite. All logic is in
//! `kor_runtime::bench`; this module parses flags, guards the budget and prints.
//! `kor bench --ui --cli --memory` (§32, ADR 0062) measures the performance budgets instead
//! (`perf`, `desktop`); `kor doctor --perf` reads `perf.log`.

mod desktop;
mod perf;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::anyhow;
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use kor_runtime::bench::{self, Report, Spec};

use crate::Ctx;

#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // clap flags, one per option
pub struct BenchArgs {
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(short, long = "model")]
    models: Vec<String>,
    #[arg(long, default_value_t = bench::DEFAULT_MAX_COST_USD)]
    max_cost: f64,
    #[arg(short, long)]
    yes: bool,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    baseline: Option<String>,
    #[arg(long = "route", requires = "baseline")]
    routes: Vec<String>,
    /// Graph tools off vs on (spec §7.5): `on` / `off` run one arm, bare `--graph` both.
    #[arg(long, value_parser = ["on", "off", "both"], num_args = 0..=1,
          default_missing_value = "both", conflicts_with = "baseline")]
    graph: Option<String>,
    /// §32 performance budgets: TUI + desktop.
    #[arg(long)]
    ui: bool,
    #[arg(long)]
    cli: bool,
    /// §31.3.1 knowledge base budgets.
    #[arg(long)]
    memory: bool,
    #[arg(long, default_value = "bench/history.json")]
    history: PathBuf,
    #[arg(long)]
    record: bool,
    #[arg(long)]
    check: bool,
    #[arg(long)]
    update_baseline: bool,
    #[arg(long)]
    weak: bool,
    #[arg(long)]
    desktop: Option<PathBuf>,
    #[arg(long, default_value_t = 1_000_000)]
    records: u64,
    #[arg(long)]
    performance_md: Option<PathBuf>,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-bench"))
        .mut_arg("file", |a| a.help(l.t("arg-bench-file")))
        .mut_arg("models", |a| a.help(l.t("arg-bench-model")))
        .mut_arg("max_cost", |a| a.help(l.t("arg-bench-max-cost")))
        .mut_arg("yes", |a| a.help(l.t("arg-bench-yes")))
        .mut_arg("json", |a| a.help(l.t("arg-bench-json")))
        .mut_arg("baseline", |a| a.help(l.t("arg-bench-baseline")))
        .mut_arg("routes", |a| a.help(l.t("arg-bench-route")))
        .mut_arg("graph", |a| a.help(l.t("arg-bench-graph")))
        .mut_arg("ui", |a| a.help(l.t("arg-bench-ui")))
        .mut_arg("cli", |a| a.help(l.t("arg-bench-cli")))
        .mut_arg("memory", |a| a.help(l.t("arg-bench-memory")))
        .mut_arg("history", |a| a.help(l.t("arg-bench-history")))
        .mut_arg("record", |a| a.help(l.t("arg-bench-record")))
        .mut_arg("check", |a| a.help(l.t("arg-bench-check")))
        .mut_arg("update_baseline", |a| {
            a.help(l.t("arg-bench-update-baseline"))
        })
        .mut_arg("weak", |a| a.help(l.t("arg-bench-weak")))
        .mut_arg("desktop", |a| a.help(l.t("arg-bench-desktop")))
        .mut_arg("records", |a| a.help(l.t("arg-bench-records")))
        .mut_arg("performance_md", |a| {
            a.help(l.t("arg-bench-performance-md"))
        })
}

/// `kor doctor --perf` (§32.3): where the time went, from the local `perf.log`.
#[allow(clippy::unnecessary_wraps)] // same signature as the other command runners
pub fn doctor_perf(ctx: &Ctx) -> anyhow::Result<bool> {
    use kor_runtime::budgets;
    let l = &ctx.l;
    let on = ctx.config.as_ref().is_ok_and(|c| c.config.perf.log);
    let entries = budgets::perf_log_path()
        .map(|p| budgets::read_log(&p))
        .unwrap_or_default();
    if entries.is_empty() {
        println!(
            "{}",
            l.t(if on {
                "doctor-perf-empty"
            } else {
                "doctor-perf-off"
            })
        );
        return Ok(true);
    }
    println!(
        "{}",
        l.t_args("doctor-perf-head", &args!("n" => entries.len()))
    );
    for c in budgets::causes(&entries).iter().take(15) {
        println!(
            "{}",
            l.t_args(
                "doctor-perf-row",
                &args!("source" => c.source.clone(), "kind" => c.kind.clone(), "what" => c.what.clone(),
                       "count" => c.count, "p95" => format!("{:.0}", c.p95_ms), "max" => format!("{:.0}", c.max_ms))
            )
        );
    }
    if !on {
        println!("{}", l.t("doctor-perf-off"));
    }
    Ok(true)
}

/// Draw one TUI frame; a frame over 50 ms goes to `perf.log` (§32.3) with the docked panel as
/// its cause.
pub fn draw_logged<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    screen: &crate::tui::Screen,
    l: &Localizer,
    config: Option<&kor_config::Config>,
) -> std::io::Result<()> {
    let t = std::time::Instant::now();
    terminal.draw(|frame| crate::tui::draw(frame, screen, l))?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    if ms > kor_runtime::budgets::SLOW_FRAME_MS
        && let Some(c) = config
    {
        kor_runtime::budgets::log(
            c,
            &kor_runtime::budgets::PerfEntry::now(
                "tui",
                "frame",
                &format!("panel:{:?}", screen.dock),
                ms,
                Some(format!("{} lines", screen.lines.len())),
            ),
        );
    }
    Ok(())
}

/// `category=provider/model` pairs.
fn parse_routes(raw: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    raw.iter()
        .map(|r| {
            r.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                .filter(|(k, v)| !k.is_empty() && v.contains('/'))
                .ok_or_else(|| anyhow!("--route {r}: category=provider/model expected"))
        })
        .collect()
}

#[allow(clippy::too_many_lines)] // load, guard, run, print: one linear flow
pub fn run(ctx: &Ctx, a: &BenchArgs) -> anyhow::Result<bool> {
    if a.ui || a.cli || a.memory || a.performance_md.is_some() {
        return perf::run(
            ctx,
            &perf::Opts {
                ui: a.ui,
                cli: a.cli,
                memory: a.memory,
                history: a.history.clone(),
                record: a.record,
                check: a.check,
                update_baseline: a.update_baseline,
                weak: a.weak,
                desktop: a.desktop.clone(),
                records: a.records,
                performance_md: a.performance_md.clone(),
            },
        );
    }
    let l = &ctx.l;
    let loaded = ctx.config.as_ref().map_err(|e| anyhow!(e.to_string()))?;
    let root = kor_runtime::project_root()?;
    let file = a
        .file
        .clone()
        .unwrap_or_else(|| root.join(bench::SPEC_FILE));
    // The spec's commands (and the model's code under test) run on this machine.
    if !kor_runtime::is_trusted(&root) {
        eprintln!(
            "{}",
            l.t_args(
                "bench-untrusted",
                &args!("file" => file.display().to_string())
            )
        );
        return Ok(false);
    }
    let spec = match Spec::load(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", l.t_args("bench-spec-error", &args!("error" => e)));
            return Ok(false);
        }
    };
    let jobs = if let Some(base) = &a.baseline {
        let mut routes = parse_routes(&a.routes)?;
        if routes.is_empty() {
            routes.clone_from(&spec.routing);
        }
        if routes.is_empty() {
            eprintln!("{}", l.t("bench-no-routes"));
            return Ok(false);
        }
        bench::router_jobs(&spec, base, &routes)
    } else {
        let models = if a.models.is_empty() {
            spec.models.clone()
        } else {
            a.models.clone()
        };
        if models.is_empty() {
            eprintln!("{}", l.t("bench-no-models"));
            return Ok(false);
        }
        match a.graph.as_deref() {
            Some(g) => bench::graph_jobs(&spec, &models, g != "on", g != "off"),
            None => bench::race_jobs(&spec, &models),
        }
    };

    let prices = bench::Prices::load(l, crate::catalog_cmd::cache_path().as_deref());
    let est = bench::estimate(&jobs, &prices);
    println!(
        "{}",
        l.t_args(
            "bench-estimate",
            &args!("jobs" => jobs.len(), "usd" => format!("${:.2}", est.usd),
                   "max" => format!("${:.2}", a.max_cost))
        )
    );
    if !est.unknown.is_empty() {
        println!(
            "{}",
            l.t_args(
                "bench-unknown-price",
                &args!("models" => est.unknown.join(", "))
            )
        );
    }
    if !bench::within_budget(&est, a.max_cost) && !a.yes {
        eprintln!("{}", l.t("bench-over-budget"));
        return Ok(false);
    }

    let make = bench::runtime_agent(l, loaded);
    let progress = |p: bench::Progress| match p {
        bench::Progress::Start {
            index,
            total,
            task,
            model,
            arm,
        } => eprintln!(
            "{}",
            l.t_args(
                "bench-start",
                &args!("n" => index + 1, "total" => total, "task" => task, "model" => model,
                       "arm" => arm.as_str())
            )
        ),
        bench::Progress::Done { index, total, row } => {
            let result = if row.passed {
                l.t("bench-pass")
            } else {
                l.t("bench-fail")
            };
            let tail = row.error.map(|e| format!(" ({e})")).unwrap_or_default();
            eprintln!(
                "{}{tail}",
                l.t_args(
                    "bench-done",
                    &args!("n" => index + 1, "total" => total, "result" => result,
                           "secs" => format!("{:.1}", secs(row.wall_ms)),
                           "cost" => bench::money(row.cost_usd, row.cost_estimated))
                )
            );
        }
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let rows = crate::providers::runtime()?.block_on(async {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                stop.cancel();
            }
        });
        bench::run(l, &root, &spec, &jobs, &make, &prices, &progress, &cancel).await
    });
    let report = bench::report(rows);
    print_report(l, &report);
    if let Some(path) = &a.json {
        let text = serde_json::to_string_pretty(&report)?;
        if path.as_os_str() == "-" {
            println!("{text}");
        } else {
            std::fs::write(path, text)?;
            eprintln!(
                "{}",
                l.t_args(
                    "bench-json-written",
                    &args!("path" => path.display().to_string())
                )
            );
        }
    }
    Ok(!cancel.is_cancelled())
}

#[allow(clippy::cast_precision_loss)] // display only
fn secs(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

fn print_report(l: &Localizer, r: &Report) {
    println!("{}", l.t("bench-head"));
    for s in &r.summary {
        let calls = s.tool_calls_ok + s.tool_calls_repaired + s.tool_calls_failed;
        println!(
            "  {:<36} {:<8} {}/{} ({:.0}%) {:>8.1} {:>10}↑ {:>8}↓ {:>10} {}/{} {}",
            s.model,
            s.arm.map_or("", bench::Arm::as_str),
            s.passed,
            s.tasks,
            s.pass_rate * 100.0,
            secs(s.wall_ms),
            s.input_tokens,
            s.output_tokens,
            bench::money(s.cost_usd, s.cost_estimated),
            s.tool_calls_ok + s.tool_calls_repaired,
            calls,
            s.emulated,
        );
    }
    if let Some(g) = &r.graph {
        let pct = |d: Option<f64>| d.map_or_else(|| "?".to_owned(), |d| format!("{d:+.1}%"));
        println!(
            "{}",
            l.t_args(
                "bench-graph-compare",
                &args!("tokens" => pct(g.token_delta_pct), "cost" => pct(g.cost_delta_pct),
                       "pass" => format!("{:+.1}", g.pass_delta_pp))
            )
        );
        println!(
            "{}",
            l.t(match g.meets_acceptance {
                Some(true) => "bench-graph-accept-yes",
                Some(false) => "bench-graph-accept-no",
                None => "bench-graph-accept-unknown",
            })
        );
    }
    if r.routing.is_empty() {
        if r.comparison.is_none() && r.graph.is_none() {
            println!("{}", l.t("bench-no-routing"));
        }
    } else {
        println!("{}", l.t("bench-routing"));
        print!("{}", r.routing_toml);
    }
    if let Some(c) = &r.comparison {
        println!(
            "{}",
            l.t_args(
                "bench-compare",
                &args!("cost" => c.cost_delta_pct.map_or_else(|| "?".to_owned(), |d| format!("{d:+.1}%")),
                       "pass" => format!("{:+.1}", c.pass_delta_pp))
            )
        );
        println!(
            "{}",
            l.t(match c.meets_acceptance {
                Some(true) => "bench-accept-yes",
                Some(false) => "bench-accept-no",
                None => "bench-accept-unknown",
            })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_flags_parse() {
        use clap::{CommandFactory as _, FromArgMatches as _, Parser};
        #[derive(Parser)]
        struct T {
            #[command(flatten)]
            b: BenchArgs,
        }
        let m = T::command()
            .try_get_matches_from([
                "kor",
                "--ui",
                "--cli",
                "--memory",
                "--check",
                "--record",
                "--weak",
                "--records",
                "1000",
                "--history",
                "h.json",
                "--performance-md",
                "P.md",
            ])
            .unwrap();
        let t = T::from_arg_matches(&m).unwrap();
        assert!(t.b.ui && t.b.cli && t.b.memory && t.b.check && t.b.record && t.b.weak);
        assert!(!t.b.update_baseline);
        assert_eq!(t.b.records, 1000);
        assert_eq!(t.b.history, PathBuf::from("h.json"));
        assert_eq!(t.b.performance_md, Some(PathBuf::from("P.md")));
        let d = T::try_parse_from(["kor"]).unwrap();
        assert_eq!(d.b.history, PathBuf::from("bench/history.json"));
        assert_eq!(d.b.records, 1_000_000);
    }

    #[test]
    fn routes_parse() {
        let r = parse_routes(&["docs=local/qwen".into(), " review = a/b ".into()]).unwrap();
        assert_eq!(r["docs"], "local/qwen");
        assert_eq!(r["review"], "a/b");
        assert!(parse_routes(&["docs".into()]).is_err());
        assert!(parse_routes(&["=a/b".into()]).is_err());
        assert!(parse_routes(&["docs=nomodel".into()]).is_err());
    }
}
