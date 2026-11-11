//! `kor daemon start|stop|status|logs|run|install|uninstall` (§8.4, ADR 0039). Logic in
//! `kor_runtime::triggers::daemon`; this file prints.

use kor_i18n::{Localizer, args};
use kor_runtime::triggers::{self, daemon};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum DaemonCmd {
    /// Start the daemon in the background.
    Start {
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        allow_remote: bool,
    },
    /// Stop the daemon.
    Stop,
    /// Whether the daemon runs.
    Status,
    /// Show the daemon log.
    Logs {
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        #[arg(short, long)]
        follow: bool,
    },
    /// Run the daemon in this terminal (what `start` and the login entry run).
    Run {
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        allow_remote: bool,
    },
    /// Start the daemon at login.
    Install,
    /// Remove the login entry.
    Uninstall,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let addr = |s: clap::Command| {
        s.mut_arg("listen", |a| a.help(l.t("arg-daemon-listen")))
            .mut_arg("allow_remote", |a| a.help(l.t("arg-daemon-allow-remote")))
    };
    c.about(l.t("cmd-daemon"))
        .mut_subcommand("start", |s| addr(s.about(l.t("cmd-daemon-start"))))
        .mut_subcommand("stop", |s| s.about(l.t("cmd-daemon-stop")))
        .mut_subcommand("status", |s| s.about(l.t("cmd-daemon-status")))
        .mut_subcommand("logs", |s| s.about(l.t("cmd-daemon-logs")))
        .mut_subcommand("run", |s| addr(s.about(l.t("cmd-daemon-run"))))
        .mut_subcommand("install", |s| s.about(l.t("cmd-daemon-install")))
        .mut_subcommand("uninstall", |s| s.about(l.t("cmd-daemon-uninstall")))
}

fn dir() -> anyhow::Result<std::path::PathBuf> {
    triggers::dir().ok_or_else(|| anyhow::anyhow!("no Kor data folder"))
}

/// The project in the working directory starts being watched when it defines triggers.
pub(crate) fn register_cwd(dir: &std::path::Path) {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(root) = cwd
            .ancestors()
            .find(|d| triggers::project_config(d).is_file())
        && std::fs::read_to_string(triggers::project_config(root))
            .is_ok_and(|t| t.contains("[[triggers]]"))
    {
        let _ = triggers::register(dir, root);
    }
}

#[allow(clippy::too_many_lines)] // flat subcommand dispatch
pub fn run(ctx: &Ctx, cmd: DaemonCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let dir = dir()?;
    match cmd {
        DaemonCmd::Start {
            listen,
            allow_remote,
        } => {
            register_cwd(&dir);
            let mut extra = Vec::new();
            if let Some(a) = listen {
                // Checked here so a refused address is reported, not hidden in the log.
                daemon::listen_addr(l, &a, allow_remote).map_err(|e| anyhow::anyhow!(e))?;
                extra.extend(["--listen".to_owned(), a]);
            }
            if allow_remote {
                extra.push("--allow-remote".into());
            }
            let exe = std::env::current_exe()?;
            match daemon::spawn_background(&exe, &extra) {
                Ok(s) => {
                    println!("{}", l.t_args("daemon-started", &args!("addr" => s.addr)));
                    Ok(true)
                }
                Err(log) => {
                    eprintln!("{}", l.t_args("daemon-start-failed", &args!("log" => log)));
                    Ok(false)
                }
            }
        }
        DaemonCmd::Stop => {
            let rt = crate::providers::runtime()?;
            if daemon::state(&dir).is_none() {
                println!("{}", l.t("daemon-not-running"));
                return Ok(true);
            }
            let ok = rt.block_on(daemon::stop_daemon(&dir));
            println!(
                "{}",
                l.t(if ok {
                    "daemon-stopped"
                } else {
                    "daemon-not-running"
                })
            );
            Ok(ok)
        }
        DaemonCmd::Status => {
            let v = crate::providers::runtime()?.block_on(daemon::status(&dir));
            if v["running"] != true {
                println!("{}", l.t("daemon-not-running"));
                return Ok(true);
            }
            println!(
                "{}",
                l.t_args(
                    "daemon-status-running",
                    &args!(
                        "pid" => v["pid"].as_u64().unwrap_or(0),
                        "addr" => v["addr"].as_str().unwrap_or_default().to_owned(),
                        "triggers" => v["triggers"].as_u64().unwrap_or(0),
                        "queued" => v["queued"].as_u64().unwrap_or(0)
                    )
                )
            );
            for w in v["waiting"].as_array().into_iter().flatten() {
                println!(
                    "  ? {}  {}  (kor triggers approve {})",
                    w["job"].as_str().unwrap_or_default(),
                    w["summary"].as_str().unwrap_or_default(),
                    w["id"].as_str().unwrap_or_default()
                );
            }
            Ok(true)
        }
        DaemonCmd::Logs { lines, follow } => {
            let text = daemon::log_tail(&dir, lines.clamp(1, 10_000));
            if !text.is_empty() {
                println!("{text}");
            }
            if follow {
                follow_log(&daemon::log_path(&dir));
            }
            Ok(true)
        }
        DaemonCmd::Run {
            listen,
            allow_remote,
        } => {
            let cfg = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut notes = Vec::new();
            let opts = daemon::options(l, &cfg.config, listen.as_deref(), allow_remote, &mut notes)
                .map_err(|e| anyhow::anyhow!(e))?;
            for n in notes {
                eprintln!("{n}");
            }
            crate::providers::runtime()?.block_on(async {
                let running = daemon::start(opts).await.map_err(|e| anyhow::anyhow!(e))?;
                // §27.5 `~/Kor/` stays live while the daemon runs (views, Gelen, Görevler; ADR 0052).
                let _korfolder = kor_runtime::korfolder::watch::start_if_enabled(
                    l.lang(),
                    std::sync::Arc::new(|_: &kor_runtime::korfolder::watch::Pass| {}),
                );
                running.wait().await;
                Ok(true)
            })
        }
        DaemonCmd::Install => {
            let exe = std::env::current_exe()?;
            let at = kor_platform::service::install(&exe)?;
            println!("{}", l.t_args("daemon-installed", &args!("where" => at)));
            Ok(true)
        }
        DaemonCmd::Uninstall => {
            kor_platform::service::uninstall()?;
            println!("{}", l.t("daemon-uninstalled"));
            Ok(true)
        }
    }
}

fn follow_log(path: &std::path::Path) {
    use std::io::{Read as _, Seek as _};
    let mut pos = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(mut f) = std::fs::File::open(path) else {
            continue;
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < pos {
            pos = 0;
        }
        if len == pos || f.seek(std::io::SeekFrom::Start(pos)).is_err() {
            continue;
        }
        let mut buf = String::new();
        if f.take(len - pos).read_to_string(&mut buf).is_ok() {
            print!("{buf}");
            pos = len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        cmd: DaemonCmd,
    }

    #[test]
    fn parses() {
        let t = T::try_parse_from(["kor", "start", "--listen", "127.0.0.1:9000"]).unwrap();
        assert!(matches!(
            t.cmd,
            DaemonCmd::Start {
                listen: Some(_),
                allow_remote: false
            }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "logs", "-n", "5", "-f"])
                .unwrap()
                .cmd,
            DaemonCmd::Logs {
                lines: 5,
                follow: true
            }
        ));
        for c in ["stop", "status", "run", "install", "uninstall"] {
            T::try_parse_from(["kor", c]).unwrap();
        }
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<T as clap::CommandFactory>::command(), &l);
    }
}
