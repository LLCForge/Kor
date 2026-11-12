//! `kor cost [why] [--export csv|json]` and `kor quota` (Master Build Prompt §5.1, §5.2, §15.10).
//! All logic lives in `kor_runtime::cost` and `kor_runtime::quota`; this only prints.

use kor_i18n::args;

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Export {
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy, clap::Subcommand)]
pub enum CostCmd {
    /// Why each model was chosen: the router's recent decisions with their reasons.
    Why {
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
}

/// `kor cost`: the dashboard, an export of it, or the routing decisions.
pub fn cost(ctx: &Ctx, days: u32, export: Option<Export>, cmd: Option<CostCmd>) -> bool {
    if let Some(CostCmd::Why { last }) = cmd {
        return why(ctx, last);
    }
    match export {
        None => crate::extra::cost(ctx, days),
        Some(Export::Csv) => {
            print!(
                "{}",
                kor_runtime::cost::to_csv(&crate::extra::cost_report(ctx, days))
            );
            true
        }
        Some(Export::Json) => {
            match serde_json::to_string_pretty(&crate::extra::cost_report(ctx, days)) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!(
                        "{}",
                        ctx.l
                            .t_args("error-generic", &args!("error" => e.to_string()))
                    );
                    return false;
                }
            }
            true
        }
    }
}

/// `kor cost why`: the last `last` routing decisions, oldest first.
fn why(ctx: &Ctx, last: usize) -> bool {
    let l = &ctx.l;
    let all = kor_runtime::cost::decisions_file()
        .map(|p| kor_runtime::cost::decisions(&p))
        .unwrap_or_default();
    if all.is_empty() {
        println!("{}", l.t("cost-why-none"));
        return true;
    }
    println!("{}", l.t("cost-why-head"));
    for d in &all[all.len().saturating_sub(last.max(1))..] {
        let outcome = match d.outcome {
            Some(true) => l.t("cost-why-ok"),
            Some(false) => l.t("cost-why-failed"),
            None => String::new(),
        };
        let cat = d.category.map_or("-", kor_runtime::cost::Category::key);
        println!(
            "  {}  {cat:<16} {}  {outcome}\n      {}\n      \"{}\"",
            kor_runtime::cost::fmt_time(d.at),
            d.model,
            d.reason,
            d.task
        );
    }
    true
}

/// `kor quota`: remaining quota per key, last 24 hours, `OpenRouter` credits, failover order,
/// and the local `kor proxy` counters when it is running.
pub fn quota(ctx: &Ctx) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let config = ctx
        .config
        .as_ref()
        .map(|c| c.config.clone())
        .unwrap_or_default();
    let rt = crate::providers::runtime()?;
    let report = rt.block_on(kor_runtime::quota::report(l, &config));
    for line in kor_runtime::quota::lines(l, &report) {
        println!("{line}");
    }
    // The local gateway is on this machine, so privacy mode allows asking it.
    if let Ok(Some(key)) = kor_proxy::access_key(&kor_secrets::KeychainStore, false) {
        let port = kor_proxy::default_config_path()
            .and_then(|p| kor_proxy::config::load(&p).ok())
            .unwrap_or_default()
            .port;
        let base = format!("http://127.0.0.1:{port}");
        if let Ok(Some(r)) = rt.block_on(kor_proxy::remote_usage(&base, Some(&key))) {
            let u = &r["usage"];
            let n = |x: &serde_json::Value| x.as_u64().unwrap_or(0);
            println!(
                "{}",
                l.t_args(
                    "proxy-status-head",
                    &args!("url" => base, "requests" => n(&u["total_requests"]),
                           "failed" => n(&u["failure_count"]), "tokens" => n(&u["total_tokens"]))
                )
            );
            // §4.3, §15.10: each subscription account or key of the gateway.
            for a in kor_proxy::usage::account_rows(&r) {
                println!(
                    "  {}",
                    l.t_args(
                        "quota-proxy-account",
                        &args!("account" => a.account, "requests" => a.requests, "failed" => a.failed,
                               "limited" => a.rate_limited, "tokens" => a.tokens)
                    )
                );
            }
        }
    }
    Ok(true)
}
