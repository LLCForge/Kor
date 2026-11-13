//! `kor catalog [model]` (§4.4): intelligence/coding index, speed, latency, price, context,
//! tool calling, vision and licence per model, each with its source. Keyless: AA scores come only
//! from the central catalog, stored locally and read offline (ADR-0003).

use kor_i18n::{Localizer, args};
use kor_providers::model_catalog::{self as mc, Endpoints, ModelData, Options};

use crate::Ctx;

/// Rows shown without a query, best intelligence index first.
const LIST_LIMIT: usize = 40;

pub fn cache_path() -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|d| d.join("kor").join("model-catalog.json"))
}

pub fn run(ctx: &Ctx, query: Option<&str>, refresh: bool, offline: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let privacy = ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    let cache = cache_path();
    // `[auto] catalog_refresh = false`: a stale cache is used as is until `--refresh` (§28.14).
    let no_auto = !refresh
        && ctx
            .config
            .as_ref()
            .is_ok_and(|c| !c.config.auto.catalog_refresh)
        && cache.as_deref().is_some_and(std::path::Path::is_file);
    let opts = Options {
        offline: offline || privacy || no_auto,
        refresh,
    };
    let loaded = crate::providers::runtime()?.block_on(mc::load(
        cache.as_deref(),
        &Endpoints::default(),
        opts,
    ));
    let loaded = match loaded {
        Ok(x) => x,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("catalog-error", &args!("error" => e.to_string()))
            );
            return Ok(false);
        }
    };
    for n in &loaded.notices {
        eprintln!(
            "{}",
            l.t_args("catalog-notice", &args!("text" => n.clone()))
        );
    }
    let d = &loaded.data;
    let hours = age_hours(d.fetched_at);
    println!(
        "{}",
        l.t_args(
            "catalog-header",
            &args!(
                "count" => d.catalog.models.len(),
                "origin" => match d.origin {
                    mc::Origin::Published => "catalog.json",
                    mc::Origin::Openrouter => "openrouter.ai",
                },
                "hours" => hours
            )
        )
    );
    let models = &d.catalog.models;
    if let Some(q) = query {
        let found = mc::search(models, q);
        match found.as_slice() {
            [] => {
                eprintln!(
                    "{}",
                    l.t_args("catalog-not-found", &args!("model" => q.to_owned()))
                );
                return Ok(false);
            }
            [m] => details(l, m),
            many => table(many),
        }
    } else {
        let mut all: Vec<&ModelData> = models.iter().collect();
        all.sort_by(|a, b| {
            b.aa_intelligence_index
                .unwrap_or(-1.0)
                .total_cmp(&a.aa_intelligence_index.unwrap_or(-1.0))
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        table(&all[..all.len().min(LIST_LIMIT)]);
        if all.len() > LIST_LIMIT {
            println!(
                "{}",
                l.t_args("catalog-more", &args!("count" => all.len() - LIST_LIMIT))
            );
        }
    }
    println!("{}", l.t("catalog-attribution"));
    Ok(true)
}

fn age_hours(fetched_at: u64) -> u64 {
    kor_providers::catalog::now().saturating_sub(fetched_at) / 3600
}

fn num(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), |x| format!("{x:.1}"))
}

fn price(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), |x| format!("${x:.2}"))
}

fn flag(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}

fn table(rows: &[&ModelData]) {
    println!(
        "{:<44} {:>6} {:>6} {:>7} {:>8} {:>8} {:>8} {:>9} {:>5} {:>6}",
        "model", "intel", "code", "tok/s", "ttft ms", "in/M", "out/M", "context", "tools", "vision"
    );
    for m in rows {
        println!(
            "{:<44} {:>6} {:>6} {:>7} {:>8} {:>8} {:>8} {:>9} {:>5} {:>6}",
            m.model_id,
            num(m.aa_intelligence_index),
            num(m.aa_coding_index),
            num(m.speed_tps),
            num(m.ttft_ms),
            price(m.input_price_per_m),
            price(m.output_price_per_m),
            m.context.map_or_else(|| "-".into(), |c| c.to_string()),
            flag(m.tool_calling),
            flag(m.vision),
        );
    }
}

fn details(l: &Localizer, m: &ModelData) {
    println!("{}  ({})", m.model_id, m.provider.as_deref().unwrap_or("-"));
    let rows: [(&str, &str, String); 16] = [
        (
            "catalog-f-intelligence",
            "aa_intelligence_index",
            num(m.aa_intelligence_index),
        ),
        (
            "catalog-f-coding",
            "aa_coding_index",
            num(m.aa_coding_index),
        ),
        ("catalog-f-math", "aa_math_index", num(m.aa_math_index)),
        ("catalog-f-speed", "speed_tps", num(m.speed_tps)),
        ("catalog-f-ttft", "ttft_ms", num(m.ttft_ms)),
        (
            "catalog-f-input",
            "input_price_per_m",
            price(m.input_price_per_m),
        ),
        (
            "catalog-f-output",
            "output_price_per_m",
            price(m.output_price_per_m),
        ),
        (
            "catalog-f-context",
            "context",
            m.context.map_or_else(|| "-".into(), |c| c.to_string()),
        ),
        (
            "catalog-f-tools",
            "tool_calling",
            flag(m.tool_calling).into(),
        ),
        ("catalog-f-vision", "vision", flag(m.vision).into()),
        (
            "catalog-f-license",
            "license",
            m.license.clone().unwrap_or_else(|| "-".into()),
        ),
        (
            "catalog-f-hf-leaderboard",
            "hf_leaderboard_avg",
            num(m.hf_leaderboard_avg),
        ),
        ("catalog-f-lmarena", "lmarena_rating", num(m.lmarena_rating)),
        ("catalog-f-livebench", "livebench_avg", num(m.livebench_avg)),
        (
            "catalog-f-updated",
            "updated_at",
            m.updated_at.clone().unwrap_or_else(|| "-".into()),
        ),
        (
            "catalog-f-aa-fetched",
            "aa_fetched",
            m.aa_fetched.clone().unwrap_or_else(|| "-".into()),
        ),
    ];
    for (key, field, value) in rows {
        let src = m.source.get(field).map_or("-", String::as_str);
        println!("  {:<28} {value:<14} [{src}]", l.t(key));
    }
}
