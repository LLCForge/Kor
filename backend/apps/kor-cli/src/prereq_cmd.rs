//! `kor prereq list|install` (spec §24.3 Layer C, ADR 0064): system prerequisites only the OS can
//! install (Docker/Podman, WSL2, Xcode Command Line Tools, `WebView2`) and the macOS permissions.
//! The plan comes from `kor_platform::sysdeps`; `install` shows the exact command, asks (never
//! silent; `--yes` is the answer given in advance), and the OS asks for the password.
//! `--dry-run` prints the command and runs nothing. Declining keeps Kor working: only the feature
//! that needs the prerequisite stays off.

use clap::{Args, Subcommand};
use kor_i18n::{Localizer, args};
use kor_platform::sysdeps::{self, Action, Host, Manager, NoPlan, Prereq};

use crate::Ctx;
use crate::doctor::{prereq_describe as describe, prereq_state as state};

#[derive(Debug, Subcommand)]
pub enum PrereqCmd {
    /// Show every prerequisite for this OS, whether it is present and the install command.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Install one prerequisite with the system package manager, after asking.
    Install(InstallArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// docker, podman, wsl2, xcode-clt, webview2, accessibility, screen-recording, microphone
    #[arg(value_parser = parse_prereq)]
    pub id: Prereq,
    /// Package manager to use when several are installed (winget, choco, brew, apt, dnf, pacman, zypper, apk, nix).
    #[arg(long, value_parser = parse_manager)]
    pub manager: Option<Manager>,
    /// Print the exact command and run nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not ask before running the command.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

fn parse_prereq(s: &str) -> Result<Prereq, String> {
    Prereq::parse(s).ok_or_else(|| Prereq::ALL.map(Prereq::id).join(", "))
}

fn parse_manager(s: &str) -> Result<Manager, String> {
    Manager::parse(s).ok_or_else(|| Manager::ALL.map(Manager::id).join(", "))
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-prereq"))
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-prereq-list"))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
        .mut_subcommand("install", |s| {
            s.about(l.t("cmd-prereq-install"))
                .mut_arg("id", |a| a.help(l.t("arg-prereq-id")))
                .mut_arg("manager", |a| a.help(l.t("arg-prereq-manager")))
                .mut_arg("dry_run", |a| a.help(l.t("arg-prereq-dry-run")))
                .mut_arg("yes", |a| a.help(l.t("arg-yes")))
        })
}

pub fn run(ctx: &Ctx, cmd: &PrereqCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let host = Host::detect();
    match cmd {
        PrereqCmd::List { json } => {
            let rows = sysdeps::status(&host);
            if *json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(true);
            }
            println!("{}", l.t("prereq-title"));
            for r in &rows {
                println!(
                    "  {} {} — {}",
                    if r.installed == Some(true) {
                        "✓"
                    } else {
                        "·"
                    },
                    describe(l, r.prereq),
                    state(l, r.installed)
                );
                match &r.plan {
                    Ok(p) => println!(
                        "      {}",
                        l.t_args("prereq-command", &args!("command" => p.command_line(false)))
                    ),
                    Err(NoPlan::NoManager) => println!("      {}", l.t("prereq-no-manager")),
                    Err(NoPlan::NotNeeded) => {}
                }
            }
            Ok(true)
        }
        PrereqCmd::Install(a) => Ok(install(l, &host, a)),
    }
}

fn install(l: &Localizer, host: &Host, a: &InstallArgs) -> bool {
    let name = describe(l, a.id);
    let plan = match sysdeps::plan(a.id, host, a.manager) {
        Ok(p) => p,
        Err(NoPlan::NotNeeded) => {
            println!("{}", l.t_args("prereq-not-needed", &args!("name" => name)));
            return true;
        }
        Err(NoPlan::NoManager) => {
            eprintln!("{}", l.t("prereq-no-manager"));
            eprintln!("{}", l.t_args("prereq-declined", &args!("name" => name)));
            return false;
        }
    };
    if plan.action == Action::Install && sysdeps::installed(a.id) == Some(true) {
        println!("{}", l.t_args("prereq-already", &args!("name" => name)));
        return true;
    }
    println!("{name}");
    println!(
        "  {}",
        l.t_args(
            "prereq-command",
            &args!("command" => plan.command_line(false))
        )
    );
    if a.dry_run {
        println!("{}", l.t("prereq-dry-run"));
        return true;
    }
    if !a.yes && !crate::doctor::completions::confirm(&l.t("prereq-ask")) {
        println!("{}", l.t_args("prereq-declined", &args!("name" => name)));
        return false;
    }
    let status = sysdeps::command(&plan, false).status();
    match status {
        Ok(s) if s.success() => {
            let key = if plan.action == Action::OpenSettings {
                "prereq-settings-opened"
            } else {
                "prereq-done"
            };
            println!("{}", l.t_args(key, &args!("name" => name)));
            true
        }
        Ok(s) => {
            eprintln!(
                "{}",
                l.t_args(
                    "prereq-failed",
                    &args!("name" => name, "code" => s.code().unwrap_or(-1))
                )
            );
            false
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args(
                    "prereq-failed",
                    &args!("name" => name, "code" => e.to_string())
                )
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser, Subcommand};

    use super::*;

    #[derive(Parser)]
    #[command(name = "k")]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Subcommand)]
    enum Top {
        Prereq {
            #[command(subcommand)]
            cmd: PrereqCmd,
        },
    }

    #[test]
    fn parses_and_localizes() {
        let Top::Prereq {
            cmd: PrereqCmd::Install(a),
        } = Cli::try_parse_from([
            "k",
            "prereq",
            "install",
            "docker",
            "--manager",
            "apt",
            "--dry-run",
        ])
        .unwrap()
        .cmd
        else {
            panic!()
        };
        assert_eq!(
            (a.id, a.manager, a.dry_run, a.yes),
            (Prereq::Docker, Some(Manager::Apt), true, false)
        );
        assert!(Cli::try_parse_from(["k", "prereq", "install", "docker-compose"]).is_err());
        assert!(
            Cli::try_parse_from(["k", "prereq", "install", "podman", "--manager", "yum"]).is_err()
        );
        assert!(matches!(
            Cli::try_parse_from(["k", "prereq", "list", "--json"])
                .unwrap()
                .cmd,
            Top::Prereq {
                cmd: PrereqCmd::List { json: true }
            }
        ));
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            let l = Localizer::new(lang);
            let _ = localize(
                Cli::command().find_subcommand("prereq").unwrap().clone(),
                &l,
            );
            for p in Prereq::ALL {
                let d = describe(&l, p);
                assert!(!d.starts_with("prereq-"), "{lang:?} {p:?} untranslated");
            }
            for key in [
                "prereq-title",
                "prereq-ask",
                "prereq-dry-run",
                "prereq-no-manager",
                "prereq-command",
                "prereq-declined",
            ] {
                assert_ne!(l.t(key), key, "{lang:?}");
            }
        }
    }

    #[test]
    fn dry_run_runs_nothing() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let host = Host {
            os: kor_platform::Os::Linux,
            managers: vec![Manager::Apt],
            root: false,
        };
        let a = InstallArgs {
            id: Prereq::Podman,
            manager: None,
            dry_run: true,
            yes: true,
        };
        // `podman` is not invoked: the dry run returns before `sysdeps::command`.
        assert!(install(&l, &host, &a));
    }
}
