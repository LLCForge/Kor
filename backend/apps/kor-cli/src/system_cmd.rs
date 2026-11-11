//! `kor system [show|refresh|health]` (§19.2–19.3, ADR 0059): the local system profile
//! (`<kor home>/system.json`) the agent sees, and the health checks behind the desktop's alert
//! cards. Logic in `kor_health`.

use kor_health::health::Level;
use kor_health::system::{self, Profile, RealProbe};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum SystemCmd {
    /// The stored profile (collected now when there is none).
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Collect the profile again and report what changed.
    Refresh {
        #[arg(long)]
        json: bool,
    },
    /// Run the health checks now.
    Health {
        /// Also measure build caches under this folder, Docker and Kor's cache (slower).
        #[arg(long)]
        deep: bool,
        #[arg(long)]
        json: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let json = |s: clap::Command| s.mut_arg("json", |a| a.help(l.t("arg-json")));
    c.about(l.t("cmd-system"))
        .mut_subcommand("show", |s| json(s.about(l.t("cmd-system-show"))))
        .mut_subcommand("refresh", |s| json(s.about(l.t("cmd-system-refresh"))))
        .mut_subcommand("health", |s| {
            json(s.about(l.t("cmd-system-health"))).mut_arg("deep", |a| a.help(l.t("arg-system-deep")))
        })
}

fn home(l: &Localizer) -> anyhow::Result<std::path::PathBuf> {
    kor_health::kor_home().ok_or_else(|| anyhow::anyhow!(l.t("system-no-home")))
}

fn print_profile(l: &Localizer, p: &Profile) {
    let section = system::prompt_section(p, usize::MAX);
    for line in section.lines().skip_while(|s| !s.starts_with("OS:")) {
        println!("{line}");
    }
    println!("{}", l.t_args("system-collected", &args!("age" => system::now_secs().saturating_sub(p.collected_at) / 60)));
}

pub fn run(ctx: &Ctx, cmd: Option<SystemCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cfg = ctx.config.as_ref().map(|c| c.config.health.clone()).unwrap_or_default();
    match cmd.unwrap_or(SystemCmd::Show { json: false }) {
        SystemCmd::Show { json } => {
            let h = home(l)?;
            let p = match system::load(&h) {
                Some(p) => p,
                None => system::refresh(&h, &RealProbe)?.profile,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else {
                print_profile(l, &p);
            }
            Ok(true)
        }
        SystemCmd::Refresh { json } => {
            let r = system::refresh(&home(l)?, &RealProbe)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(true);
            }
            print_profile(l, &r.profile);
            if r.changed.is_empty() {
                println!("{}", l.t("system-unchanged"));
            } else {
                println!("{}", l.t_args("system-changed", &args!("keys" => r.changed.join(", "))));
            }
            Ok(true)
        }
        SystemCmd::Health { deep, json } => {
            let cwd = std::env::current_dir().ok();
            let alerts = kor_health::check_now(&cfg, cwd.as_deref(), deep, l);
            if json {
                println!("{}", serde_json::to_string_pretty(&alerts)?);
                return Ok(true);
            }
            if alerts.is_empty() {
                println!("{}", l.t("health-none"));
            }
            for a in &alerts {
                let tag = l.t(if a.level == Level::Critical { "health-critical" } else { "health-warn" });
                println!("{tag} {}", a.title);
                if !a.body.is_empty() {
                    println!("    {}", a.body);
                }
                if a.clean {
                    println!("    {}", l.t("health-clean-hint"));
                }
            }
            Ok(alerts.iter().all(|a| a.level != Level::Critical))
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
        c: SystemCmd,
    }

    #[test]
    fn parses() {
        assert!(matches!(T::parse_from(["t", "health", "--deep"]).c, SystemCmd::Health { deep: true, json: false }));
        assert!(matches!(T::parse_from(["t", "refresh", "--json"]).c, SystemCmd::Refresh { json: true }));
        let _ = localize(<T as clap::CommandFactory>::command(), &Localizer::new(kor_i18n::Lang::Fr));
    }
}
