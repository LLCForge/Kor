//! `kor diagram <png>` (spec §10, ADR 0048): architecture diagram → services/modules skeleton
//! (interfaces + empty implementations). Same image path as `kor screenshot`; asks "implement?"
//! before any file is reviewed or written. `kor diagram sync|watch|check|list|add` (§27.14,
//! ADR 0063) keep marked diagrams in the docs in sync with the graph (`diagsync_cmd`).

use std::path::PathBuf;

use kor_i18n::Localizer;
use kor_runtime::visual::Kind;

use crate::Ctx;

/// `kor diagram sync|watch|check|list|add` (§27.14).
#[path = "diagsync_cmd.rs"]
pub mod sync;

#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct DiagramArgs {
    #[command(subcommand)]
    command: Option<sync::DiagramSub>,
    #[arg(required = true)]
    image: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(short, long)]
    model: Option<String>,
    #[arg(long, short)]
    yes: bool,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let c = c
        .about(l.t("cmd-diagram"))
        .mut_arg("image", |a| a.help(l.t("arg-diagram-image")))
        .mut_arg("out", |a| a.help(l.t("arg-visual-out")))
        .mut_arg("model", |a| a.help(l.t("arg-visual-model")))
        .mut_arg("yes", |a| a.help(l.t("arg-visual-yes")));
    sync::localize(c, l)
}

pub fn run(ctx: &Ctx, a: &DiagramArgs) -> anyhow::Result<bool> {
    if let Some(sub) = &a.command {
        return sync::run(ctx, sub);
    }
    let Some(image) = &a.image else {
        return Ok(false);
    };
    crate::screenshot_cmd::run_image(
        ctx,
        image,
        Kind::Diagram,
        a.out.as_deref(),
        a.model.as_deref(),
        a.yes,
    )
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
        Diagram(DiagramArgs),
    }

    #[test]
    fn parses_image_and_options() {
        let Sub::Diagram(a) =
            Cli::try_parse_from(["kor", "diagram", "arch.png", "--out", "svc", "-m", "x"])
                .unwrap()
                .cmd;
        assert_eq!(a.image, Some(PathBuf::from("arch.png")));
        assert_eq!(a.model.as_deref(), Some("x"));
        assert!(!a.yes);
        assert!(Cli::try_parse_from(["kor", "diagram"]).is_err());
        let Sub::Diagram(a) = Cli::try_parse_from(["kor", "diagram", "check"])
            .unwrap()
            .cmd;
        assert!(a.command.is_some() && a.image.is_none());
    }
}
