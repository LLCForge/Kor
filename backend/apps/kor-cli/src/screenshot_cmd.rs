//! `kor screenshot <png>` (spec §10, ADR 0048): UI image → component tree → components. The work is
//! `kor_runtime::visual`; this module picks the model, shows what is sent and reviews every
//! proposed file (accept / edit / reject, ADR 0030) before anything is written. `kor diagram`
//! shares [`run_image`].

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use kor_agent::Proposal;
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use kor_runtime::visual::{self, GenFile, Kind, Picture, SystemOcr, Target};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct ScreenshotArgs {
    image: PathBuf,
    #[arg(long, default_value = "react", value_parser = ["react", "svelte", "tauri", "html"])]
    target: String,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(short, long)]
    model: Option<String>,
    #[arg(long, short)]
    yes: bool,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-screenshot"))
        .mut_arg("image", |a| a.help(l.t("arg-screenshot-image")))
        .mut_arg("target", |a| a.help(l.t("arg-screenshot-target")))
        .mut_arg("out", |a| a.help(l.t("arg-visual-out")))
        .mut_arg("model", |a| a.help(l.t("arg-visual-model")))
        .mut_arg("yes", |a| a.help(l.t("arg-visual-yes")))
}

fn fail(l: &Localizer, e: impl Into<String>) -> bool {
    eprintln!("{}", l.t_args("error-generic", &args!("error" => e.into())));
    false
}

pub fn run(ctx: &Ctx, a: &ScreenshotArgs) -> anyhow::Result<bool> {
    let target: Target = match a.target.parse() {
        Ok(t) => t,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    run_image(
        ctx,
        &a.image,
        Kind::Screenshot(target),
        a.out.as_deref(),
        a.model.as_deref(),
        a.yes,
    )
}

/// Image → model (image, or local OCR text without vision) → files → review → write.
pub fn run_image(
    ctx: &Ctx,
    image: &Path,
    kind: Kind,
    out: Option<&Path>,
    model: Option<&str>,
    yes: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let pic = match Picture::read(image) {
        Ok(p) => p,
        Err(e) => return Ok(fail(l, e)),
    };
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => return Ok(fail(l, e.to_string())),
    };
    let (provider, model) = match kor_runtime::oneshot_provider(l, loaded, model) {
        Ok(p) => p,
        Err(e) => return Ok(fail(l, e)),
    };
    let name = provider.manifest().name.clone();
    let key = if provider.manifest().vision {
        "visual-sending-image"
    } else {
        "visual-ocr-fallback"
    };
    eprintln!("{}", l.t_args(key, &args!("provider" => name)));
    let rt = crate::providers::runtime()?;
    let generated = rt.block_on(visual::generate(
        &provider,
        &model,
        &pic,
        kind,
        &SystemOcr,
        &CancellationToken::new(),
    ));
    let g = match generated {
        Ok(g) => g,
        Err(e) => return Ok(fail(l, e)),
    };
    if let Some(d) = &g.description {
        eprintln!("\n{}\n{d}", l.t("visual-description"));
    }
    // The component tree: the reply before the first file.
    let tree = g.reply.split("```").next().unwrap_or("").trim();
    if !tree.is_empty() {
        println!("\n{tree}\n");
    }
    let dir = match out {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir()?,
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Ok(fail(l, format!("{}: {e}", dir.display())));
    }
    println!(
        "{}",
        l.t_args("visual-files", &args!("count" => g.files.len()))
    );
    for f in &g.files {
        println!("  {}", f.path);
    }
    let shown = kor_tools::display_path(&dir);
    if kind == Kind::Diagram
        && !crate::shell_cmd::confirm(
            l,
            &l.t_args("visual-diagram-implement", &args!("dir" => shown.clone())),
            yes,
        )
    {
        println!("{}", l.t("visual-nothing-written"));
        return Ok(true);
    }
    let accepted = if yes {
        g.files
    } else if !std::io::stdin().is_terminal() {
        eprintln!("{}", l.t("visual-review-needs-terminal"));
        return Ok(false);
    } else {
        review(l, &visual::proposals(&dir, &g.files))
    };
    if accepted.is_empty() {
        println!("{}", l.t("visual-nothing-written"));
        return Ok(true);
    }
    match rt.block_on(visual::write(&dir, &accepted)) {
        Ok(written) => {
            println!(
                "{}",
                l.t_args(
                    "visual-written",
                    &args!("count" => written.len(), "dir" => shown)
                )
            );
            Ok(true)
        }
        Err(e) => Ok(fail(l, e)),
    }
}

/// `/paste [question]` in the interactive session: the clipboard image (an error screenshot, say)
/// read with local OCR and added to the message as text (spec §10, LLM-free). `None` after printing
/// why not.
pub async fn paste_prompt(l: &Localizer, question: &str) -> Option<String> {
    let pic = match visual::clipboard_image() {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!("{}", l.t("paste-no-image"));
            return None;
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("paste-clipboard-failed", &args!("error" => e))
            );
            return None;
        }
    };
    eprintln!("{}", l.t("paste-reading"));
    match visual::image_context(&[pic], &SystemOcr, &CancellationToken::new()).await {
        Ok(text) => {
            eprintln!(
                "{}",
                l.t_args("paste-attached", &args!("chars" => text.chars().count()))
            );
            let q = if question.is_empty() {
                l.t("paste-default-question")
            } else {
                question.to_owned()
            };
            Some(format!("{q}{text}"))
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            None
        }
    }
}

fn read_answer() -> String {
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_lowercase()
}

/// File by file: the diff, then `a` accept, `e` edit in `$EDITOR`, anything else rejects.
fn review(l: &Localizer, props: &[Proposal]) -> Vec<GenFile> {
    let mut keep = Vec::new();
    for p in props {
        println!("\n{}", crate::tui::review::diff(p));
        print!(
            "{} ",
            l.t_args("perm-file", &args!("path" => p.path.clone()))
        );
        let _ = std::io::stdout().flush();
        let after = p.after.clone().unwrap_or_default();
        match read_answer().as_str() {
            "a" => keep.push(GenFile {
                path: p.path.clone(),
                content: after,
            }),
            "e" => match crate::tui::review::edit_in_editor(&p.path, &after) {
                Ok(content) => keep.push(GenFile {
                    path: p.path.clone(),
                    content,
                }),
                Err(error) => {
                    eprintln!(
                        "{}",
                        l.t_args("perm-editor-failed", &args!("error" => error))
                    );
                }
            },
            _ => {}
        }
    }
    keep
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
        Screenshot(ScreenshotArgs),
    }

    #[test]
    fn parses_targets_and_rejects_unknown_ones() {
        let Sub::Screenshot(a) = Cli::try_parse_from([
            "kor",
            "screenshot",
            "ui.png",
            "--target",
            "svelte",
            "--out",
            "web",
            "-y",
        ])
        .unwrap()
        .cmd;
        assert_eq!((a.target.as_str(), a.yes), ("svelte", true));
        assert_eq!(a.out.as_deref(), Some(Path::new("web")));
        let Sub::Screenshot(a) = Cli::try_parse_from(["kor", "screenshot", "ui.png"])
            .unwrap()
            .cmd;
        assert_eq!(a.target, "react");
        assert!(Cli::try_parse_from(["kor", "screenshot", "ui.png", "--target", "vue"]).is_err());
        assert!(Cli::try_parse_from(["kor", "screenshot"]).is_err());
    }
}
