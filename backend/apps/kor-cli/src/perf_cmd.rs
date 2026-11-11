//! `kor perf <command|test|url> [--profiler] [--duration] [--requests] [--concurrency] [--pid]
//! [--cmd] [--allow-host] [--optimize] [--out]`, `kor perf report <file>`, `kor perf list`
//! (§15.8, ADR 0049). Profiling, parsing, flamegraphs and graph mapping live in
//! `kor_runtime::perf`; `--optimize` runs the agent in the normal REPL (every write reviewed per
//! file, ADR 0030) and then measures again the same way.

use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_runtime::perf::{self, Report, run::CompareRow, run::Row};

use crate::Ctx;

#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct PerfArgs {
    #[command(subcommand)]
    pub command: Option<PerfCmd>,
    /// A command (quoted), `test`, or an http(s) URL on this computer.
    #[arg(required = true)]
    pub target: Option<String>,
    #[arg(long)]
    pub profiler: Option<String>,
    #[arg(long)]
    pub duration: Option<u64>,
    #[arg(long, default_value_t = 200)]
    pub requests: u32,
    #[arg(long, default_value_t = 10)]
    pub concurrency: u32,
    #[arg(long)]
    pub pid: Option<u32>,
    #[arg(long = "cmd")]
    pub server_cmd: Option<String>,
    #[arg(long = "allow-host")]
    pub allow_host: Vec<String>,
    #[arg(long)]
    pub optimize: bool,
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, clap::Subcommand)]
pub enum PerfCmd {
    /// Read an existing profile file (pprof, speedscope, cpuprofile, samply, collapsed, perf script, sample).
    Report {
        file: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Earlier runs of this project.
    List,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-perf"))
        .mut_arg("target", |a| a.help(l.t("arg-perf-target")))
        .mut_arg("profiler", |a| a.help(l.t("arg-perf-profiler")))
        .mut_arg("duration", |a| a.help(l.t("arg-perf-duration")))
        .mut_arg("requests", |a| a.help(l.t("arg-perf-requests")))
        .mut_arg("concurrency", |a| a.help(l.t("arg-perf-concurrency")))
        .mut_arg("pid", |a| a.help(l.t("arg-perf-pid")))
        .mut_arg("server_cmd", |a| a.help(l.t("arg-perf-cmd")))
        .mut_arg("allow_host", |a| a.help(l.t("arg-perf-allow-host")))
        .mut_arg("optimize", |a| a.help(l.t("arg-perf-optimize")))
        .mut_arg("out", |a| a.help(l.t("arg-perf-out")))
        .mut_arg("yes", |a| a.help(l.t("arg-perf-yes")))
        .mut_subcommand("report", |s| {
            s.about(l.t("cmd-perf-report"))
                .mut_arg("file", |a| a.help(l.t("arg-perf-file")))
                .mut_arg("out", |a| a.help(l.t("arg-perf-out")))
        })
        .mut_subcommand("list", |s| s.about(l.t("cmd-perf-list")))
}

pub fn run(ctx: &Ctx, a: &PerfArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    match &a.command {
        Some(PerfCmd::List) => {
            let runs = perf::list(&root);
            if runs.is_empty() {
                println!("{}", l.t("perf-list-empty"));
            }
            let mut rows = vec![l.t("perf-list-columns").split('|').map(str::to_owned).collect::<Vec<_>>()];
            for s in runs {
                rows.push(vec![
                    s.id,
                    s.profiler,
                    s.wall_ms.map(|w| format!("{w:.0} ms")).unwrap_or_default(),
                    s.hottest.map(|h| format!("{h} {:.1}%", s.hottest_pct.unwrap_or(0.0))).unwrap_or_default(),
                    s.target,
                ]);
            }
            if rows.len() > 1 {
                crate::providers::table(&rows);
            }
            Ok(true)
        }
        Some(PerfCmd::Report { file, out }) => match perf::report_file(&root, file, l.lang()) {
            Ok(r) => {
                print_report(l, &r);
                copy_out(l, &root, &r, out.as_deref());
                Ok(true)
            }
            Err(e) => {
                eprintln!("{e}");
                Ok(false)
            }
        },
        None => profile(ctx, &root, a),
    }
}

fn options(ctx: &Ctx, root: &std::path::Path, a: &PerfArgs, baseline: Option<String>) -> perf::Options {
    perf::Options {
        root: root.to_owned(),
        target: a.target.clone().unwrap_or_default(),
        profiler: a.profiler.clone(),
        duration: a.duration,
        requests: a.requests,
        concurrency: a.concurrency,
        pid: a.pid,
        server_cmd: a.server_cmd.clone(),
        allow_hosts: a.allow_host.clone(),
        baseline,
        lang: ctx.l.lang(),
    }
}

/// Plan, ask when the run leaves the sandbox, run.
fn measure(ctx: &Ctx, config: &kor_config::Config, opts: &perf::Options, yes: bool) -> anyhow::Result<Option<Report>> {
    let l = &ctx.l;
    let plan = match perf::plan(opts, config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Ok(None);
        }
    };
    println!(
        "{}",
        l.t_args(
            "perf-running",
            &args!(
                "profiler" => plan.profiler.as_str(),
                "lang" => plan.lang.as_str(),
                "target" => plan.command.clone().or_else(|| plan.url.clone()).unwrap_or_default(),
                "sandbox" => plan.sandbox.as_str()
            )
        )
    );
    if let Some(q) = &plan.ask
        && !crate::shell_cmd::confirm(l, q, yes)
    {
        eprintln!("{}", l.t("perf-denied"));
        return Ok(None);
    }
    let rt = crate::providers::runtime()?;
    match rt.block_on(perf::execute(plan)) {
        Ok(r) => Ok(Some(r)),
        Err(e) => {
            eprintln!("{e}");
            Ok(None)
        }
    }
}

fn profile(ctx: &Ctx, root: &std::path::Path, a: &PerfArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let config = ctx.config.as_ref().map_err(|e| anyhow::anyhow!(e.to_string()))?.config.clone();
    kor_runtime::apply_network_policy(l, &config);
    let Some(before) = measure(ctx, &config, &options(ctx, root, a, None), a.yes)? else {
        return Ok(false);
    };
    print_report(l, &before);
    copy_out(l, root, &before, a.out.as_deref());
    if !a.optimize {
        return Ok(true);
    }
    let Some(prompt) = perf::optimize_prompt(&before) else {
        println!("{}", l.t("perf-optimize-none"));
        return Ok(true);
    };
    if let Some(h) = &before.hottest {
        println!("\n{}", l.t_args("perf-optimize-target", &args!("name" => h.name.clone(), "pct" => format!("{:.2}", h.self_pct))));
    }
    if !crate::shell_cmd::confirm(l, &l.t("perf-optimize-ask"), a.yes) {
        return Ok(true);
    }
    let ok = crate::repl::run(
        ctx,
        &crate::repl::RunArgs {
            prompt: Some(prompt),
            ..crate::repl::RunArgs::default()
        },
    )?;
    if !ok {
        return Ok(false);
    }
    println!("\n{}", l.t("perf-remeasure"));
    // The same target, profiler and load; the earlier run is the baseline.
    let mut again = options(ctx, root, a, Some(before.id.clone()));
    again.profiler = Some(before.profiler.clone());
    let Some(after) = measure(ctx, &config, &again, true)? else {
        return Ok(false);
    };
    print_report(l, &after);
    print_compare(l, &after.compare);
    Ok(true)
}

fn location(r: &Row) -> String {
    match (&r.file, r.line) {
        (Some(f), Some(n)) => format!("{f}:{n}"),
        (Some(f), None) => f.clone(),
        _ => String::new(),
    }
}

fn human(v: u64, unit: perf::Unit) -> String {
    #[allow(clippy::cast_precision_loss)]
    let f = v as f64;
    match unit {
        perf::Unit::Bytes if v >= 1 << 20 => format!("{:.1} MiB", f / f64::from(1u32 << 20)),
        perf::Unit::Bytes if v >= 1 << 10 => format!("{:.1} KiB", f / 1024.0),
        perf::Unit::Nanoseconds => format!("{:.1} ms", f / 1e6),
        perf::Unit::Microseconds => format!("{:.1} ms", f / 1e3),
        u => format!("{v} {}", u.as_str()),
    }
}

fn table(l: &Localizer, rows: &[Row], n: usize) {
    let mut t = vec![l.t("perf-columns").split('|').map(str::to_owned).collect::<Vec<_>>()];
    for r in rows.iter().take(n) {
        t.push(vec![
            format!("{:.2}", r.self_pct),
            format!("{:.2}", r.total_pct),
            r.name.chars().take(60).collect(),
            location(r),
            r.node.as_ref().map(|n| format!("{} ({})", n.id, n.by)).unwrap_or_default(),
        ]);
    }
    crate::providers::table(&t);
}

fn print_report(l: &Localizer, r: &Report) {
    println!();
    let mut head = vec![human(r.total, r.unit)];
    if let Some(w) = r.wall_ms {
        head.insert(0, l.t_args("perf-wall", &args!("ms" => format!("{w:.0}"))));
    }
    if let Some(c) = r.exit_code {
        head.push(l.t_args("perf-exit", &args!("code" => c)));
    }
    println!("{}", head.join(" · "));
    if let Some(s) = &r.load {
        println!(
            "{}",
            l.t_args(
                "perf-load",
                &args!(
                    "requests" => s.requests, "ok" => s.ok, "errors" => s.errors,
                    "rps" => format!("{:.1}", s.rps), "p50" => format!("{:.1}", s.p50_ms), "p95" => format!("{:.1}", s.p95_ms)
                )
            )
        );
        if let Some(e) = &s.first_error {
            println!("  ! {e}");
        }
    }
    if let Some(h) = &r.hottest
        && let Some(n) = &h.node
    {
        println!(
            "{}",
            l.t_args(
                "perf-hottest",
                &args!("name" => n.name.clone(), "pct" => format!("{:.2}", h.self_pct), "total" => format!("{:.2}", h.total_pct), "location" => format!("{}:{}", n.file, n.line.unwrap_or(0)))
            )
        );
    }
    println!("\n{}", l.t("perf-top"));
    table(l, &r.top, 15);
    match &r.memory {
        Some(m) => {
            let count = m.count.map(|c| l.t_args("perf-alloc-count", &args!("count" => c))).unwrap_or_default();
            println!("\n{} {count}", l.t_args("perf-memory", &args!("total" => human(m.total, m.unit))));
            table(l, &m.top, 8);
        }
        None => {
            if let Some(n) = &r.memory_note {
                println!("\n{n}");
            }
        }
    }
    for n in &r.notes {
        println!("  · {n}");
    }
    println!("\n{}", l.t_args("perf-flamegraph", &args!("path" => r.flamegraph.clone())));
    if let Some(m) = &r.memory_flamegraph {
        println!("{}", l.t_args("perf-flamegraph-memory", &args!("path" => m.clone())));
    }
    if !r.compare.is_empty() {
        print_compare(l, &r.compare);
    }
}

fn print_compare(l: &Localizer, rows: &[CompareRow]) {
    println!("\n{}", l.t("perf-compare"));
    let mut t = vec![l.t("perf-compare-columns").split('|').map(str::to_owned).collect::<Vec<_>>()];
    for c in rows {
        let label = l.t(&format!("perf-metric-{}", c.metric.replace('_', "-")));
        let label = c.subject.as_ref().map_or(label.clone(), |s| format!("{label} ({s})"));
        t.push(vec![
            label,
            format!("{:.2}", c.before),
            c.after.map_or_else(|| "-".into(), |a| format!("{a:.2}")),
            c.change_pct.map_or_else(String::new, |p| format!("{p:+.2}%")),
        ]);
    }
    crate::providers::table(&t);
}

fn copy_out(l: &Localizer, root: &std::path::Path, r: &Report, out: Option<&std::path::Path>) {
    let Some(out) = out else { return };
    match std::fs::copy(root.join(&r.flamegraph), out) {
        Ok(_) => println!("{}", l.t_args("perf-flamegraph", &args!("path" => out.display().to_string()))),
        Err(e) => eprintln!("{}: {e}", out.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: PerfArgs,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["perf", "cargo test --release", "--profiler", "samply", "--optimize"]).unwrap();
        assert_eq!(t.a.target.as_deref(), Some("cargo test --release"));
        assert!(t.a.optimize && t.a.command.is_none());
        let t = T::try_parse_from(["perf", "http://127.0.0.1:3000/api", "--requests", "500", "--concurrency", "20", "--cmd", "node server.js"]).unwrap();
        assert_eq!((t.a.requests, t.a.concurrency, t.a.server_cmd.as_deref()), (500, 20, Some("node server.js")));
        let t = T::try_parse_from(["perf", "report", "cpu.pprof", "--out", "f.svg"]).unwrap();
        assert!(matches!(t.a.command, Some(PerfCmd::Report { .. })));
        let t = T::try_parse_from(["perf", "list"]).unwrap();
        assert!(matches!(t.a.command, Some(PerfCmd::List)));
        assert!(T::try_parse_from(["perf"]).is_err());
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            localize(T::command(), &Localizer::new(lang)).debug_assert();
        }
    }
}
