//! SSH remote mode (spec §15.3, ADR 0044): `kor --remote user@host[:port] [--project /dir]`,
//! `kor --remote … attach [task]` and the remote side `kor remote-serve`. The agent runs on the
//! server (`kor_runtime::remote`); this terminal only shows its events and answers its questions.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;

use kor_agent::remote::wire::Command;
use kor_i18n::{Localizer, args};
use kor_runtime::remote::{self as rm, BinarySource, Client, TaskStore};
use serde_json::Value;

use crate::Ctx;

/// Localized help for `remote-serve` and `attach` (for `localized_command`).
pub fn localize_serve(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-remote-serve"))
        .mut_arg("project", |a| a.help(l.t("arg-remote-project")))
        .mut_arg("worker", |a| a.help(l.t("arg-remote-worker")))
}

pub fn localize_attach(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-attach"))
        .mut_arg("task", |a| a.help(l.t("arg-remote-task")))
}

fn privacy(ctx: &Ctx) -> bool {
    kor_runtime::privacy_mode() || ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode)
}

fn connections() -> Option<PathBuf> {
    kor_config::user_dir().map(|d| d.join("connections.toml"))
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

/// What to do once connected.
pub enum Mode {
    /// Interactive; with `Some(prompt)` send it first.
    Chat(Option<String>),
    /// `-p`: send, print, exit when the turn is done.
    Print(String),
    /// `attach <task>` (follow) or `attach` (list tasks).
    Attach(Option<String>),
}

/// `kor --remote <target> …`.
pub fn run(ctx: &Ctx, target: &str, project: Option<&str>, mode: Mode) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if privacy(ctx) {
        return Ok(fail(l, l.t("remote-privacy")));
    }
    let h = match kor_agent::remote::resolve_target(target, connections().as_deref()) {
        Ok(h) => h,
        Err(e) => return Ok(fail(l, e)),
    };
    let shown = kor_agent::remote::display_target(&h);
    let rt = crate::providers::runtime()?;
    rt.block_on(async {
        eprintln!("{}", l.t_args("remote-checking", &args!("host" => shown.clone())));
        let src = BinarySource::standard(crate::update::VERSION, std::env::current_exe().ok());
        match rm::ensure(&kor_agent::remote::SshShell::new(&h), &src).await {
            Ok(e) => {
                if let Some(old) = e.replaced {
                    let key = if old.is_empty() { "remote-installed" } else { "remote-updated" };
                    eprintln!(
                        "{}",
                        l.t_args(key, &args!("version" => src.version.clone(), "old" => old, "target" => e.info.target.unwrap_or_default()))
                    );
                }
            }
            Err(e) => return Ok(fail(l, l.t_args("remote-install-failed", &args!("error" => e)))),
        }
        let mut c = match Client::connect(&h, project).await {
            Ok(c) => c,
            Err(e) => return Ok(fail(l, e)),
        };
        eprintln!(
            "{}",
            l.t_args(
                "remote-connected",
                &args!("host" => shown.clone(), "project" => c.hello["project"].as_str().unwrap_or_default().to_owned())
            )
        );
        session(l, &mut c, &h, &shown, mode).await
    })
}

pub(crate) fn print_tasks(l: &Localizer, v: &Value) {
    let list = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if list.is_empty() {
        println!("{}", l.t("remote-no-tasks"));
    }
    for t in list {
        println!(
            "{}  {:<8}  {}",
            t["id"].as_str().unwrap_or_default(),
            t["state"].as_str().unwrap_or_default(),
            t["prompt"]
                .as_str()
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
        );
    }
}

/// Print one event; returns `Some(id)` for a permission card.
pub(crate) fn show(l: &Localizer, v: &Value) -> Option<String> {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
    match v["type"].as_str().unwrap_or_default() {
        "text" => {
            print!("{}", s("delta"));
            let _ = std::io::stdout().flush();
        }
        "tool" => println!("\n> {} {}", s("name"), s("summary")),
        "toolResult" => {
            let r = s("result");
            let first = r.lines().next().unwrap_or_default();
            println!("  {}", first.chars().take(200).collect::<String>());
        }
        "notice" => eprintln!("{}", s("text")),
        "error" => eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => s("message")))
        ),
        "attached" => eprintln!(
            "{}",
            l.t_args("remote-attached", &args!("task" => s("task")))
        ),
        "done" => println!(),
        "confirm" => {
            let danger = v["destructive"].is_string();
            eprintln!(
                "\n{}",
                l.t_args(
                    if danger {
                        "remote-confirm-danger"
                    } else {
                        "remote-confirm"
                    },
                    &args!("tool" => s("name"), "summary" => s("summary"))
                )
            );
            return Some(s("id"));
        }
        _ => {}
    }
    None
}

pub(crate) fn answer(line: &str) -> &'static str {
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" | "e" | "evet" | "o" | "oui" => "yes",
        "a" | "always" | "her" | "t" | "toujours" => "always",
        _ => "no",
    }
}

#[allow(clippy::too_many_lines)] // one select loop dispatching events and REPL commands
async fn session(
    l: &Localizer,
    c: &mut Client,
    h: &kor_agent::remote::SshHost,
    shown: &str,
    mode: Mode,
) -> anyhow::Result<bool> {
    let interactive = std::io::stdin().is_terminal();
    let mut task: Option<String> = None;
    let mut exit_on_done = false;
    match mode {
        Mode::Attach(None) => {
            print_tasks(l, &c.hello["tasks"]);
            return Ok(true);
        }
        Mode::Attach(Some(id)) => {
            c.send(&Command::Attach { task: id, from: 0 })
                .await
                .map_err(anyhow::Error::msg)?;
        }
        Mode::Print(p) => {
            c.send(&Command::Send {
                text: p,
                task: None,
            })
            .await
            .map_err(anyhow::Error::msg)?;
            exit_on_done = true;
        }
        Mode::Chat(first) => {
            if let Some(p) = first {
                c.send(&Command::Send {
                    text: p,
                    task: None,
                })
                .await
                .map_err(anyhow::Error::msg)?;
            }
        }
    }
    // stdin on its own thread: lines arrive while events stream.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    if interactive {
        eprintln!("{}", l.t("remote-help"));
    }
    let mut asking: Option<String> = None;
    let mut ok = true;
    loop {
        tokio::select! {
            ev = c.next() => {
                let Some(v) = ev else {
                    eprintln!("{}", l.t_args("remote-lost", &args!("task" => task.clone().unwrap_or_default(), "host" => shown.to_owned())));
                    return Ok(false);
                };
                if let Some(t) = v["task"].as_str() {
                    task = Some(t.to_owned());
                }
                if v["type"] == "tasks" {
                    print_tasks(l, &v["tasks"]);
                }
                if v["type"] == "error" {
                    ok = false;
                }
                if let Some(id) = show(l, &v) {
                    if interactive {
                        asking = Some(id);
                    } else {
                        // Nobody to ask: the safe answer.
                        c.send(&Command::Approve { id, approval: "no".into(), files: None }).await.map_err(anyhow::Error::msg)?;
                    }
                }
                if v["type"] == "done" && exit_on_done {
                    return Ok(ok);
                }
            }
            line = rx.recv() => {
                let Some(line) = line else {
                    // stdin closed: leave the task running.
                    if !exit_on_done {
                        detach_note(l, task.as_deref(), shown);
                        return Ok(true);
                    }
                    continue;
                };
                if let Some(id) = asking.take() {
                    c.send(&Command::Approve { id, approval: answer(&line).into(), files: None }).await.map_err(anyhow::Error::msg)?;
                    continue;
                }
                let t = line.trim();
                let cmd = match t.split_once(' ').map_or((t, ""), |(a, b)| (a, b.trim())) {
                    ("", _) => continue,
                    ("/exit" | "/quit" | "/detach", _) => {
                        let _ = c.send(&Command::Detach).await;
                        detach_note(l, task.as_deref(), shown);
                        return Ok(true);
                    }
                    ("/tasks", _) => Command::Tasks,
                    ("/abort", _) => Command::Abort,
                    ("/attach", id) if !id.is_empty() => Command::Attach { task: id.to_owned(), from: 0 },
                    ("/discover", _) => {
                        let target = kor_agent::remote::display_target(h);
                        // stdin belongs to the session reader: no sudo questions here
                        // (`kor discover <host> --sudo …` asks for them).
                        let _ = crate::discover_cmd::scan(l, h, &target, &[], false).await;
                        continue;
                    }
                    _ => Command::Send { text: line.clone(), task: task.clone() },
                };
                c.send(&cmd).await.map_err(anyhow::Error::msg)?;
            }
        }
    }
}

fn detach_note(l: &Localizer, task: Option<&str>, host: &str) {
    if let Some(t) = task {
        eprintln!(
            "{}",
            l.t_args(
                "remote-detached",
                &args!("task" => t.to_owned(), "host" => host.to_owned())
            )
        );
    }
}

/// `kor remote-serve [--project dir] [--worker id]`: runs on the server.
pub fn serve(ctx: &Ctx, project: Option<&str>, worker: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = match project {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?,
    };
    let Ok(root) = std::fs::canonicalize(&root).map(|r| kor_tools::without_verbatim(&r)) else {
        return Ok(fail(
            l,
            l.t_args(
                "remote-no-project",
                &args!("path" => root.display().to_string()),
            ),
        ));
    };
    let Some(store) = TaskStore::standard() else {
        return Ok(fail(l, "no Kor data directory"));
    };
    let rt = crate::providers::runtime()?;
    if let Some(id) = worker {
        if !kor_agent::remote::wire::valid_task_id(id) {
            return Ok(fail(l, format!("invalid task id {id}")));
        }
        let loaded = match &ctx.config {
            Ok(c) => c.clone(),
            Err(e) => return Ok(fail(l, e)),
        };
        let mut turns = rm::AgentTurns::new(Localizer::new(l.lang()), loaded, root, id);
        return Ok(match rt.block_on(rm::run_worker(&store, id, &mut turns)) {
            Ok(()) => true,
            Err(e) => fail(l, e),
        });
    }
    let spawn = |id: &str| rm::spawn_worker(&store, id, &root);
    let r = rt.block_on(rm::serve(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        &store,
        &root,
        &spawn,
    ));
    Ok(match r {
        Ok(()) => true,
        Err(e) => fail(l, e),
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[arg(long, global = true)]
        remote: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[command(subcommand)]
        cmd: Option<C>,
    }

    #[derive(Debug, clap::Subcommand)]
    enum C {
        RemoteServe {
            #[arg(long)]
            project: Option<String>,
            #[arg(long, hide = true)]
            worker: Option<String>,
        },
        Attach {
            task: Option<String>,
        },
    }

    #[test]
    fn flags_parse_like_the_main_cli() {
        let t = T::try_parse_from([
            "kor",
            "--remote",
            "deploy@web1:2222",
            "--project",
            "/srv/app",
            "attach",
            "r-1",
        ])
        .unwrap();
        assert_eq!(t.remote.as_deref(), Some("deploy@web1:2222"));
        assert!(matches!(t.cmd, Some(C::Attach { task: Some(ref x) }) if x == "r-1"));
        assert_eq!(t.project.as_deref(), Some("/srv/app"));
        let t = T::try_parse_from(["kor", "attach", "--remote", "web1"]).unwrap();
        assert_eq!(t.remote.as_deref(), Some("web1"));
        let t = T::try_parse_from(["kor", "remote-serve", "--project", "/p", "--worker", "r-2"])
            .unwrap();
        assert!(matches!(
            t.cmd,
            Some(C::RemoteServe {
                worker: Some(_),
                project: Some(_)
            })
        ));
        assert_eq!(super::answer(" Evet "), "yes");
        assert_eq!(super::answer("a"), "always");
        assert_eq!(super::answer("whatever"), "no");
    }
}
