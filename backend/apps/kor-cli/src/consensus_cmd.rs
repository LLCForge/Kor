//! `kor consensus "<prompt>" [--models a,b,c] [--json]` and `kor consensus --shadow` (Master Build
//! Prompt §5.1): consensus and shadow modes of the cost router (logic: `kor_runtime::consensus`).

use std::sync::Arc;

use clap::Args;
use kor_i18n::{Localizer, args};
use kor_providers::Provider;

use crate::Ctx;

#[derive(Debug, Args)]
pub struct ConsensusArgs {
    /// What to ask (every model gets exactly this).
    #[arg(required_unless_present = "shadow")]
    pub prompt: Option<String>,
    /// 2–3 models (`provider/model`); default `[routing] consensus`.
    #[arg(long, value_delimiter = ',')]
    pub models: Vec<String>,
    /// Print the merged result as JSON.
    #[arg(long)]
    pub json: bool,
    /// Show shadow mode's agreement history instead (`[routing] shadow`).
    #[arg(long, conflicts_with_all = ["prompt", "models"])]
    pub shadow: bool,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-consensus"))
        .mut_arg("prompt", |a| a.help(l.t("arg-consensus-prompt")))
        .mut_arg("models", |a| a.help(l.t("arg-consensus-models")))
        .mut_arg("json", |a| a.help(l.t("arg-consensus-json")))
        .mut_arg("shadow", |a| a.help(l.t("arg-consensus-shadow")))
}

fn pct(v: f64) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0–100
    let p = (v * 100.0).round().clamp(0.0, 100.0) as u64;
    p
}

fn shadow(l: &Localizer) -> bool {
    let log = kor_runtime::kor_home().map(|h| kor_runtime::consensus::read_log(&h.join("shadow.jsonl"))).unwrap_or_default();
    let mut pairs: Vec<(String, String)> = log.iter().map(|e| (e.main.clone(), e.shadow.clone())).collect();
    pairs.sort();
    pairs.dedup();
    if pairs.is_empty() {
        println!("{}", l.t("shadow-empty"));
        return true;
    }
    for (main, sh) in pairs {
        if let Some((avg, n)) = kor_runtime::consensus::average(&log, &main, &sh) {
            println!("{}", l.t_args("shadow-row", &args!("main" => main, "cheap" => sh, "pct" => pct(avg), "count" => n)));
        }
    }
    true
}

pub fn run(ctx: &Ctx, a: &ConsensusArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if a.shadow {
        return Ok(shadow(l));
    }
    let prompt = a.prompt.clone().unwrap_or_default();
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e.to_string())));
            return Ok(false);
        }
    };
    let models: Vec<String> = if a.models.is_empty() { loaded.config.routing.consensus.clone() } else { a.models.clone() };
    let models: Vec<String> = models.into_iter().map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()).collect();
    if !(2..=3).contains(&models.len()) {
        eprintln!("{}", l.t("consensus-need-models"));
        return Ok(false);
    }
    let mut targets: Vec<(Arc<dyn Provider>, String)> = Vec::new();
    for m in &models {
        match kor_runtime::oneshot_provider(l, loaded, Some(m)) {
            Ok(t) => targets.push(t),
            Err(e) => {
                eprintln!("{m}: {e}");
                return Ok(false);
            }
        }
    }
    let rt = crate::providers::runtime()?;
    let c = rt.block_on(kor_runtime::consensus::run(&targets, &prompt));
    if a.json {
        println!("{}", serde_json::to_string_pretty(&c)?);
        return Ok(c.answers.len() >= 2);
    }
    for (m, e) in &c.failed {
        eprintln!("{}", l.t_args("consensus-failed", &args!("model" => m.clone(), "error" => e.clone())));
    }
    if c.answers.len() < 2 {
        eprintln!("{}", l.t("consensus-too-few"));
        return Ok(false);
    }
    println!("{}", l.t_args("consensus-head", &args!("count" => c.answers.len(), "pct" => pct(c.agreement))));
    if c.agreed.is_empty() {
        println!("  {}", l.t("consensus-none-agreed"));
    }
    for line in &c.agreed {
        println!("  {line}");
    }
    if !c.disputed.is_empty() {
        println!("\n{}", l.t("consensus-disputed"));
        for (i, v) in c.disputed.iter().enumerate() {
            println!("\n[{}] {}", i + 1, v.models.join(", "));
            for line in &v.lines {
                println!("  {line}");
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct T {
        #[command(flatten)]
        a: super::ConsensusArgs,
    }

    #[test]
    fn prompt_or_shadow() {
        assert!(T::try_parse_from(["kor"]).is_err());
        let t = T::try_parse_from(["kor", "why?", "--models", "a/x,b/y"]).unwrap();
        assert_eq!(t.a.models, ["a/x", "b/y"]);
        assert!(T::try_parse_from(["kor", "--shadow"]).unwrap().a.shadow);
        assert!(T::try_parse_from(["kor", "q", "--shadow"]).is_err());
        assert_eq!(super::pct(0.756), 76);
    }
}
