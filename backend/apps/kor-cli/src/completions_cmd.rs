//! `kor completions <bash|zsh|fish|powershell|elvish|nushell>` (Master Build Prompt §26.1, ADR 0050).
//!
//! Without flags the script goes to stdout (for package managers and dotfiles). `--install` puts
//! it where the shell loads it (`kor_platform::completions`); a shell that needs a profile line
//! gets one marked line, and only after the user said yes. `--uninstall` undoes exactly that.
//! No shell given with `--install` / `--uninstall`: every supported shell found on PATH.

use clap::Args;
use kor_i18n::{Localizer, args};
use kor_platform::completions::{self as comp, Shell};

use crate::Ctx;
pub use crate::doctor::completions::{detected, install_one, script};

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// bash, zsh, fish, powershell, elvish or nushell.
    #[arg(value_parser = parse_shell)]
    pub shell: Option<Shell>,
    /// Install for this shell (or every shell found) instead of printing.
    #[arg(long, conflicts_with = "uninstall")]
    pub install: bool,
    /// Remove what `--install` added.
    #[arg(long)]
    pub uninstall: bool,
    /// Do not ask before adding a profile line.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

fn parse_shell(s: &str) -> Result<Shell, String> {
    s.parse()
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-completions"))
        .mut_arg("shell", |a| a.help(l.t("arg-completions-shell")))
        .mut_arg("install", |a| a.help(l.t("arg-completions-install")))
        .mut_arg("uninstall", |a| a.help(l.t("arg-completions-uninstall")))
        .mut_arg("yes", |a| a.help(l.t("arg-yes")))
}

#[allow(clippy::unnecessary_wraps)] // the signature every command arm of `run_cmd` has
pub fn run(ctx: &Ctx, cmd: clap::Command, a: &CompletionsArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if !a.install && !a.uninstall {
        let Some(shell) = a.shell else {
            eprintln!("{}", l.t("completions-need-shell"));
            return Ok(false);
        };
        print!("{}", script(cmd, shell));
        return Ok(true);
    }
    let shells = a.shell.map_or_else(detected, |s| vec![s]);
    if shells.is_empty() {
        eprintln!("{}", l.t("completions-none-found"));
        return Ok(false);
    }
    let mut ok = true;
    for shell in shells {
        if a.install {
            ok &= install_one(l, cmd.clone(), shell, a.yes);
        } else {
            let Some(d) = comp::Dirs::current() else {
                return Ok(false);
            };
            match comp::remove(&comp::plan(shell, &d)) {
                Ok(()) => println!(
                    "{}",
                    l.t_args("completions-removed", &args!("shell" => shell.name()))
                ),
                Err(e) => {
                    ok = false;
                    eprintln!(
                        "{}",
                        l.t_args(
                            "completions-failed",
                            &args!("shell" => shell.name(), "error" => e.to_string())
                        )
                    );
                }
            }
        }
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use clap::{Parser, Subcommand};

    use super::*;

    #[derive(Parser)]
    #[command(name = "kor")]
    struct Cli {
        #[arg(long, value_parser = ["tr", "en"])]
        lang: Option<String>,
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Subcommand)]
    enum Top {
        Completions(CompletionsArgs),
        Sessions {
            #[arg(long)]
            tag: Option<String>,
        },
    }

    #[test]
    fn parses_and_generates_every_shell() {
        let Top::Completions(a) = Cli::try_parse_from(["kor", "completions", "fish"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        assert_eq!(a.shell, Some(Shell::Fish));
        assert!(Cli::try_parse_from(["kor", "completions", "tcsh"]).is_err());
        assert!(Cli::try_parse_from(["kor", "completions", "--install", "--uninstall"]).is_err());
        let Top::Completions(a) = Cli::try_parse_from(["kor", "completions", "--install", "-y"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        assert!(a.install && a.yes && a.shell.is_none());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(
            Cli::command()
                .find_subcommand("completions")
                .unwrap()
                .clone(),
            &l,
        );
        for shell in Shell::ALL {
            let s = script(Cli::command(), shell);
            assert!(s.contains("kor"), "{shell:?}");
            assert!(s.contains("sessions"), "{shell:?}: {s}");
        }
        let nu = script(Cli::command(), Shell::Nushell);
        assert!(nu.contains("export extern \"kor sessions\""), "{nu}");
        assert!(nu.contains("--tag: string"), "{nu}");
        assert!(
            nu.contains("def \"nu-complete kor lang\" [] { [\"tr\" \"en\"] }"),
            "{nu}"
        );
    }

    use clap::CommandFactory;
}
