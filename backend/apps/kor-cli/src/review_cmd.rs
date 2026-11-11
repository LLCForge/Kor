//! `kor review` (§8.8, ADR 0040): review a local diff (or a PR's) through a persona. Logic lives
//! in `kor_runtime::review`; `--pr N --post` hands over to `kor pr review`.

use clap::{Args, ValueEnum};
use kor_agent::gitflow;
use kor_i18n::{Localizer, args};
use kor_runtime::forge::{self, Forge};
use kor_runtime::review::{self, DiffSource};

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Markdown,
    Json,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // independent command-line flags
pub struct ReviewArgs {
    /// security, performance, rust-idiomatic, junior-friendly, general, or one from
    /// `.kor/review/personas/*.md`.
    #[arg(long)]
    pub persona: Option<String>,
    /// Review the staged changes (default: uncommitted changes against HEAD).
    #[arg(long, conflicts_with_all = ["range", "pr"])]
    pub staged: bool,
    /// Review a commit range `A..B`.
    #[arg(long, conflicts_with = "pr")]
    pub range: Option<String>,
    /// Review a pull request (number or URL) from the forge.
    #[arg(long)]
    pub pr: Option<String>,
    /// With --pr: post the review as PR comments after confirmation.
    #[arg(long, requires = "pr")]
    pub post: bool,
    #[arg(long, short)]
    pub yes: bool,
    #[arg(long, value_enum, default_value = "markdown")]
    pub format: Format,
    /// List the available personas.
    #[arg(long)]
    pub list_personas: bool,
    #[arg(short, long)]
    pub model: Option<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-review"))
}

fn flow_fail(l: &Localizer, e: gitflow::FlowError) -> bool {
    eprintln!("{}", forge::error_text(l, &e.into()));
    false
}

pub fn run(ctx: &Ctx, a: &ReviewArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(root) = crate::pr_cmd::repo(l)? else {
        return Ok(false);
    };
    if a.list_personas {
        for p in review::personas(&root) {
            let first = p
                .prompt
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches('#')
                .trim()
                .to_owned();
            println!("{:<18} {:<8} {first}", p.name, p.source);
        }
        return Ok(true);
    }
    let rt = crate::providers::runtime()?;
    if a.post {
        let target = a.pr.as_deref().unwrap_or_default();
        return Ok(crate::pr_cmd::pr_review(
            ctx,
            &rt,
            &root,
            target,
            a.persona.as_deref(),
            a.yes,
            a.model.as_deref(),
        ));
    }
    let name = a.persona.as_deref().unwrap_or(review::DEFAULT_PERSONA);
    let Some(persona) = review::persona(&root, name) else {
        let all: Vec<String> = review::personas(&root)
            .into_iter()
            .map(|p| p.name)
            .collect();
        eprintln!(
            "{}",
            l.t_args(
                "review-unknown-persona",
                &args!("name" => name.to_owned(), "list" => all.join(", "))
            )
        );
        return Ok(false);
    };
    let diff = if let Some(target) = &a.pr {
        let Some(num) = forge::pr_number(target) else {
            eprintln!(
                "{}",
                l.t_args("pr-bad-target", &args!("target" => target.clone()))
            );
            return Ok(false);
        };
        match Forge::open(&root) {
            Ok(client) => match rt.block_on(client.diff(num)) {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("{}", forge::error_text(l, &e));
                    return Ok(false);
                }
            },
            Err(e) => {
                eprintln!("{}", forge::error_text(l, &e));
                return Ok(false);
            }
        }
    } else {
        let src = match (&a.range, a.staged) {
            (Some(range), _) => DiffSource::Range(range.clone()),
            (None, true) => DiffSource::Staged,
            (None, false) => DiffSource::Working,
        };
        match review::local_diff(&root, &src) {
            Ok(d) => d,
            Err(e) => return Ok(flow_fail(l, e)),
        }
    };
    if diff.trim().is_empty() {
        println!("{}", l.t("review-empty-diff"));
        return Ok(true);
    }
    let provider = crate::pr_cmd::model(ctx, a.model.as_deref());
    let report = rt.block_on(review::review(
        l,
        provider.as_ref().map(|(prov, id)| (prov, id.as_str())),
        &persona,
        &diff,
    ));
    match a.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Format::Markdown => print!("{}", review::markdown(l, &report)),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Format, ReviewArgs};

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        a: ReviewArgs,
    }

    #[test]
    fn parses() {
        let c = Cli::try_parse_from([
            "review",
            "--persona",
            "security",
            "--staged",
            "--format",
            "json",
        ])
        .unwrap();
        assert!(c.a.staged && c.a.format == Format::Json);
        assert!(Cli::try_parse_from(["review", "--range", "a..b"]).is_ok());
        assert!(Cli::try_parse_from(["review", "--pr", "4", "--post", "-y"]).is_ok());
        assert!(Cli::try_parse_from(["review", "--post"]).is_err());
        assert!(Cli::try_parse_from(["review", "--staged", "--pr", "4"]).is_err());
    }
}
