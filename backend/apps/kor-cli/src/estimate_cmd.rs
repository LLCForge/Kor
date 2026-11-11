//! `kor estimate "<task>"` (Master Build Prompt §27.16, ADR 0063): time and cost before a task
//! runs, from the task's category, similar finished tasks on this machine, the model's catalog
//! price and speed, and the context size. The same estimate is shown when a task starts (spinner
//! line, `--json` `estimate` event, desktop composer). Logic: `kor_runtime::estimate`.

use clap::Args;
use kor_i18n::{Localizer, args};
use kor_runtime::estimate;

use crate::Ctx;

#[derive(Debug, Args)]
pub struct EstimateArgs {
    #[arg(required = true, num_args = 1..)]
    pub task: Vec<String>,
    /// Model to estimate for (default: the configured model).
    #[arg(short, long)]
    pub model: Option<String>,
    /// Continue this saved session (its context counts).
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub json: bool,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-estimate"))
        .mut_arg("task", |a| a.help(l.t("arg-estimate-task")))
        .mut_arg("model", |a| a.help(l.t("arg-estimate-model")))
        .mut_arg("session", |a| a.help(l.t("arg-estimate-session")))
        .mut_arg("json", |a| a.help(l.t("arg-json")))
}

pub fn run(ctx: &Ctx, a: &EstimateArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let model = a.model.clone().or_else(|| {
        ctx.config
            .as_ref()
            .ok()
            .map(|c| c.config.agent.model.trim().to_owned())
            .filter(|m| !m.is_empty())
    });
    let Some(model) = model else {
        eprintln!("{}", l.t("no-model"));
        return Ok(false);
    };
    let context = match &a.session {
        Some(id) if kor_runtime::session_tree::valid_id(id) => kor_runtime::sessions_dir()
            .map(|d| d.join(format!("{id}.jsonl")))
            .filter(|p| p.is_file())
            .map_or(estimate::BASE_CONTEXT, |p| estimate::session_context(&p)),
        Some(id) => {
            eprintln!(
                "{}",
                l.t_args(
                    "error-generic",
                    &args!("error" => format!("unknown session `{id}`"))
                )
            );
            return Ok(false);
        }
        None => estimate::BASE_CONTEXT,
    };
    let runs = estimate::file()
        .map(|p| estimate::read(&p))
        .unwrap_or_default();
    let e = estimate::estimate(
        &a.task.join(" "),
        &model,
        context,
        &kor_runtime::cost::Prices::load(),
        &runs,
    );
    if a.json {
        println!("{}", serde_json::to_string_pretty(&e)?);
        return Ok(true);
    }
    println!("{}", estimate::line(l, &e));
    println!("  {}", estimate::method_line(l, &e));
    println!(
        "  {}",
        l.t_args(
            "estimate-detail",
            &args!("model" => e.model.clone(), "turns" => e.turns, "input" => e.prompt_tokens, "output" => e.completion_tokens)
        )
    );
    if let Ok(c) = &ctx.config
        && let Some(w) = estimate::budget_warning(l, &e, &c.config.budget, 0.0, 0.0)
    {
        println!("  {w}");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        a: EstimateArgs,
    }

    #[test]
    fn parses() {
        let c = Cli::try_parse_from([
            "e",
            "refactor",
            "the",
            "parser",
            "-m",
            "anthropic/claude",
            "--json",
        ])
        .unwrap();
        assert_eq!(c.a.task.join(" "), "refactor the parser");
        assert_eq!(c.a.model.as_deref(), Some("anthropic/claude"));
        assert!(Cli::try_parse_from(["e"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
