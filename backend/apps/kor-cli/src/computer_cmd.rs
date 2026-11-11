//! `kor computer status|on|off|allow <app> <tier>|reset <app>` (§19.1, ADR 0059): desktop control
//! switch and per-app tiers. Logic in `kor_computer::policy`; tiers live in the user config
//! (`computer.apps.<app>`).

use kor_computer::policy::{self, Class, Policy, Tier};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum ComputerCmd {
    /// Whether desktop control is on, the app in front and every app's tier.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Turn desktop control on.
    On,
    /// Turn desktop control off.
    Off,
    /// Set an app's tier: blocked, read, click, ask or full.
    Allow { app: String, tier: String },
    /// Back to the built-in tier for an app.
    Reset { app: String },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-computer"))
        .mut_subcommand("status", |s| {
            s.about(l.t("cmd-computer-status"))
                .mut_arg("json", |a| a.help(l.t("arg-json")))
        })
        .mut_subcommand("on", |s| s.about(l.t("cmd-computer-on")))
        .mut_subcommand("off", |s| s.about(l.t("cmd-computer-off")))
        .mut_subcommand("allow", |s| {
            s.about(l.t("cmd-computer-allow"))
                .mut_arg("app", |a| a.help(l.t("arg-computer-app")))
                .mut_arg("tier", |a| a.help(l.t("arg-computer-tier")))
        })
        .mut_subcommand("reset", |s| {
            s.about(l.t("cmd-computer-reset"))
                .mut_arg("app", |a| a.help(l.t("arg-computer-app")))
        })
}

fn tier_label(l: &Localizer, t: Tier) -> String {
    l.t(&format!("computer-tier-{}", t.key()))
}

fn user_file(l: &Localizer, ctx: &Ctx) -> anyhow::Result<std::path::PathBuf> {
    ctx.paths
        .user
        .clone()
        .ok_or_else(|| anyhow::anyhow!(l.t("config-set-no-user-dir")))
}

pub fn run(ctx: &Ctx, cmd: ComputerCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let config = ctx
        .config
        .as_ref()
        .map(|c| c.config.computer.clone())
        .unwrap_or_default();
    match cmd {
        ComputerCmd::Status { json } => {
            let p = Policy::from_config(&config);
            let front = kor_platform::desktop_probe::foreground_app();
            let enabled = match std::env::var("KOR_COMPUTER_USE").ok().as_deref() {
                Some("1" | "true") => true,
                Some("0" | "false") => false,
                _ => config.enabled,
            };
            let watching = kor_platform::desktop_probe::idle_ms().is_some();
            if json {
                let v = serde_json::json!({
                    "enabled": enabled,
                    "front": front.as_ref().map(|a| serde_json::json!({"app": a, "tier": p.tier(a).0, "class": policy::classify(a)})),
                    "watching": watching,
                    "apps": policy::rows(&p),
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(true);
            }
            println!(
                "{}",
                l.t(if enabled {
                    "computer-status-on"
                } else {
                    "computer-status-off"
                })
            );
            if let Some(a) = &front {
                let (t, _) = p.tier(a);
                println!(
                    "{}",
                    l.t_args(
                        "computer-front",
                        &args!("app" => a.clone(), "tier" => tier_label(l, t))
                    )
                );
            }
            println!(
                "{}",
                l.t(if watching {
                    "computer-watch-on"
                } else {
                    "computer-watch-off"
                })
            );
            println!("{}", l.t("computer-defaults"));
            for c in [
                Class::Kor,
                Class::Terminal,
                Class::PasswordManager,
                Class::Banking,
                Class::Browser,
                Class::Ide,
                Class::Other,
            ] {
                println!(
                    "  {}",
                    l.t_args(
                        "computer-default-row",
                        &args!("class" => l.t(&format!("computer-class-{}", c.key())), "tier" => tier_label(l, c.default_tier()))
                    )
                );
            }
            if !p.apps.is_empty() {
                println!("{}", l.t("computer-overrides"));
                for r in policy::rows(&p) {
                    println!("  {}  {}", r.app, tier_label(l, r.tier));
                }
            }
            Ok(true)
        }
        ComputerCmd::On | ComputerCmd::Off => {
            let on = matches!(cmd, ComputerCmd::On);
            kor_config::set_scalar(
                &user_file(l, ctx)?,
                "computer.enabled",
                if on { "true" } else { "false" },
            )?;
            println!(
                "{}",
                l.t(if on {
                    "computer-status-on"
                } else {
                    "computer-status-off"
                })
            );
            Ok(true)
        }
        ComputerCmd::Allow { app, tier } => {
            let Some(t) = Tier::parse(&tier.to_ascii_lowercase()) else {
                eprintln!("{}", l.t_args("computer-bad-tier", &args!("tier" => tier)));
                return Ok(false);
            };
            let key = policy::config_key(&app).map_err(|e| anyhow::anyhow!(e))?;
            kor_config::set_string(&user_file(l, ctx)?, &key, t.key())?;
            println!(
                "{}",
                l.t_args("computer-tier-set", &args!("app" => kor_platform::desktop_probe::app_name(&app), "tier" => tier_label(l, t)))
            );
            Ok(true)
        }
        ComputerCmd::Reset { app } => {
            let key = policy::config_key(&app).map_err(|e| anyhow::anyhow!(e))?;
            kor_config::unset(&user_file(l, ctx)?, &key)?;
            let name = kor_platform::desktop_probe::app_name(&app);
            let t = policy::classify(&name).default_tier();
            println!(
                "{}",
                l.t_args(
                    "computer-tier-reset",
                    &args!("app" => name, "tier" => tier_label(l, t))
                )
            );
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: ComputerCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(
            T::parse_from(["t", "status", "--json"]).c,
            ComputerCmd::Status { json: true }
        ));
        assert!(matches!(
            T::parse_from(["t", "allow", "chrome", "full"]).c,
            ComputerCmd::Allow { .. }
        ));
        assert!(matches!(
            T::parse_from(["t", "reset", "chrome"]).c,
            ComputerCmd::Reset { .. }
        ));
        assert!(matches!(T::parse_from(["t", "on"]).c, ComputerCmd::On));
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let _ = localize(<T as clap::CommandFactory>::command(), &l);
    }
}
