//! `kor update`: check, install a signed release, switch channel, roll back (§15.17).

use kor_i18n::args;
use kor_runtime::update::{self, Channel};

use crate::Ctx;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, clap::Subcommand)]
pub enum UpdateCmd {
    /// Only look for a newer release.
    Check,
    /// Switch channel: stable, beta or nightly.
    Channel { name: String },
    /// Go back to the version that ran before the last update.
    Rollback,
}

fn channel(ctx: &Ctx) -> Channel {
    match ctx.config.as_ref().map(|c| c.config.update.channel) {
        Ok(kor_config::UpdateChannel::Beta) => Channel::Beta,
        Ok(kor_config::UpdateChannel::Nightly) => Channel::Nightly,
        _ => Channel::Stable,
    }
}

fn privacy(ctx: &Ctx) -> bool {
    ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode)
}

/// Release notes, first lines only.
fn summary(body: Option<&str>) -> String {
    body.unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn run(ctx: &Ctx, from: Option<&std::path::Path>, cmd: Option<UpdateCmd>) -> bool {
    let l = &ctx.l;
    let fail = |e: String| {
        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
        false
    };
    if let Some(file) = from {
        return match update::install_from(file) {
            Ok(_) => {
                println!("{}", l.t("update-installed-file"));
                true
            }
            Err(e) => fail(e),
        };
    }
    match cmd {
        Some(UpdateCmd::Channel { name }) => {
            if Channel::parse(&name).is_none() {
                return fail(l.t_args("update-bad-channel", &args!("name" => name)));
            }
            let Some(file) = ctx.paths.user.clone() else {
                return fail(l.t("config-set-no-user-dir"));
            };
            match kor_config::set_scalar(&file, "update.channel", &name) {
                Ok(()) => {
                    println!("{}", l.t_args("update-channel-set", &args!("name" => name)));
                    true
                }
                Err(e) => fail(e.to_string()),
            }
        }
        Some(UpdateCmd::Rollback) => match update::rollback() {
            Ok(_) => {
                println!("{}", l.t("update-rolled-back"));
                true
            }
            Err(e) => fail(e),
        },
        cmd @ (Some(UpdateCmd::Check) | None) => {
            if privacy(ctx) {
                return fail(l.t("update-privacy"));
            }
            let install = cmd.is_none();
            let channel = channel(ctx);
            let rt = match crate::providers::runtime() {
                Ok(rt) => rt,
                Err(e) => return fail(e.to_string()),
            };
            rt.block_on(async {
                let found = match update::check(VERSION, channel).await {
                    Ok(c) => c.available,
                    Err(e) => return fail(e),
                };
                let Some(r) = found else {
                    println!(
                        "{}",
                        l.t_args(
                            "update-none",
                            &args!("version" => VERSION, "channel" => channel.as_str())
                        )
                    );
                    return true;
                };
                let tag = r.tag_name.trim_start_matches('v').to_owned();
                println!(
                    "{}",
                    l.t_args(
                        "update-available",
                        &args!("version" => tag.clone(), "current" => VERSION)
                    )
                );
                let notes = summary(r.body.as_deref());
                if !notes.is_empty() {
                    println!("{notes}");
                }
                if !install {
                    return true;
                }
                if update::PUBLIC_KEY.is_none() {
                    println!(
                        "{}",
                        l.t_args("update-unsigned-build", &args!("url" => r.html_url.clone()))
                    );
                    return true;
                }
                match update::install(&r).await {
                    Ok(_) => {
                        println!("{}", l.t_args("update-installed", &args!("version" => tag)));
                        true
                    }
                    Err(e) => fail(e),
                }
            })
        }
    }
}

/// At the start of an interactive session: show the last background check's note, start today's.
pub fn on_interactive_start(ctx: &Ctx) {
    if let Some(note) = update::take_notice() {
        let mut parts = note.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("installed"), Some(v), _) => {
                println!(
                    "{}",
                    ctx.l
                        .t_args("update-installed", &args!("version" => v.to_owned()))
                );
            }
            (Some("available"), Some(v), url) => println!(
                "{}",
                ctx.l.t_args(
                    "update-available-note",
                    &args!("version" => v.to_owned(), "url" => url.unwrap_or_default().to_owned())
                )
            ),
            _ => {}
        }
    }
    let auto = ctx.config.as_ref().is_ok_and(|c| c.config.update.auto);
    if !privacy(ctx) {
        update::spawn_background_check(VERSION, channel(ctx), auto);
    }
    crate::integrate_cmd::first_run(ctx);
}

#[cfg(test)]
mod tests {
    #[test]
    fn summary_keeps_the_first_lines() {
        let body = "## Changes\n\n- a\n- b\n";
        assert_eq!(super::summary(Some(body)), "## Changes\n- a\n- b");
        assert_eq!(super::summary(None), "");
    }
}
