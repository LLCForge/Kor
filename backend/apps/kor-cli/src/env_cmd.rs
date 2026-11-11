//! `kor env status` (§15.7, ADR 0043): the project's environment (devcontainer, compose, nix,
//! mise/asdf), how agent commands run in it, and expected vs found tool versions (probed exactly
//! as a session's first command). Logic: `kor_runtime::devenv`, `kor_tools::devenv`.

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_runtime::devenv::Status;
use kor_tools::devenv::OnMismatch;

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum EnvCmd {
    /// Show what was detected and the expected vs found versions.
    Status,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-env"))
        .mut_subcommand("status", |s| s.about(l.t("cmd-env-status")))
}

pub fn run(ctx: &Ctx, cmd: &EnvCmd) -> anyhow::Result<bool> {
    let EnvCmd::Status = cmd;
    let config = ctx
        .config
        .as_ref()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .config
        .clone();
    let root = crate::graph_cmd::root()?;
    let st = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(kor_runtime::devenv::status(&config, &root));
    print(&ctx.l, &st);
    Ok(st.problems.is_none())
}

fn print(l: &Localizer, st: &Status) {
    if !st.enabled {
        println!("{}", l.t("env-status-off"));
        return;
    }
    let d = &st.detected;
    if d.is_empty() {
        println!("{}", l.t("env-status-none"));
        return;
    }
    println!(
        "{}",
        l.t_args("env-status-files", &args!("files" => d.files.join(", ")))
    );
    if d.dockerfile_dev {
        println!("{}", l.t("env-dockerfile-only"));
    }
    if let Some(k) = &d.kind {
        println!(
            "{}",
            l.t_args("env-status-kind", &args!("kind" => k.name()))
        );
        if st.tool_missing {
            println!("  ! {}", st.reason.clone().unwrap_or_default());
        }
    }
    if let Some(e) = &st.example {
        println!(
            "{}",
            l.t_args("env-status-example", &args!("example" => e.clone()))
        );
    }
    let key = match st.sandbox {
        "inside" => "env-status-sandbox-inside",
        "asks" => "env-status-sandbox-asks",
        "image" => "env-status-sandbox-image",
        _ => "env-status-sandbox-none",
    };
    println!("{}", l.t_args(key, &args!("backend" => st.sandbox_backend)));
    let mode = match st.on_mismatch {
        OnMismatch::Stop => "stop",
        OnMismatch::Warn => "warn",
    };
    println!("{}", l.t_args("env-status-mode", &args!("mode" => mode)));
    if !st.versions.is_empty() {
        println!("{}", l.t("env-status-versions"));
    }
    for v in &st.versions {
        let state = if v.bad() {
            l.t("env-state-bad")
        } else if v.ok == Some(true) {
            l.t("env-state-ok")
        } else {
            l.t("env-state-unknown")
        };
        println!(
            "  {}",
            l.t_args(
                "env-status-version",
                &args!(
                    "tool" => v.tool.as_str(),
                    "expected" => v.expected.clone(),
                    "source" => v.source.clone(),
                    "found" => v.found.clone().unwrap_or_else(|| l.t("env-found-none")),
                    "state" => state
                )
            )
        );
        if v.bad() {
            println!(
                "    {}",
                l.t_args(
                    "env-status-install",
                    &args!("hint" => v.tool.install_hint(&v.expected))
                )
            );
        }
    }
    if let Some(p) = &st.problems {
        println!("{}", l.t("env-status-problems"));
        for line in p.lines() {
            println!("  {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Subcommand)]
    enum Top {
        Env {
            #[command(subcommand)]
            command: EnvCmd,
        },
    }

    #[test]
    fn parses_env_status() {
        let Top::Env { command } = Cli::try_parse_from(["kor", "env", "status"]).unwrap().cmd;
        assert!(matches!(command, EnvCmd::Status));
        assert!(Cli::try_parse_from(["kor", "env"]).is_err());
        let _ = localize(
            clap::Command::new("env").subcommand(clap::Command::new("status")),
            &Localizer::new(kor_i18n::Lang::Fr),
        );
    }
}
