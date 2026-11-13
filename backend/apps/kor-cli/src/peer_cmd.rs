//! `kor peer …` (spec §31.1, ADR 0057): the Kor-to-Kor bridge. Pair machines with a one-time
//! code, delegate tasks (`kor peer run <peer|auto> "<task>"`, also `kor task --on <peer>`),
//! follow and approve them, bring the result back as a diff, hand a running session off when this
//! machine closes, sync memory with a three-way merge and set what each peer may do here.
//! The logic is `kor_runtime::peer`; this file parses, prints and asks.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::time::Duration;

use kor_agent::remote::wire::Command;
use kor_i18n::{Localizer, args};
use kor_runtime::peer::{self, Node, PeerEntry, client, pair, snapshot, standby};
use serde_json::{Value, json};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum PeerCmd {
    /// Show a pairing code (and QR) for another machine, or join one with `--join`.
    Pair {
        /// The code shown on the other machine.
        #[arg(long)]
        join: Option<String>,
        /// Reach the other machine through SSH (`user@host[:port]` or a connections.toml name).
        #[arg(long)]
        ssh: Option<String>,
        /// Addresses to put in the code instead of the detected ones (`host:port`).
        #[arg(long = "addr")]
        addrs: Vec<String>,
    },
    /// Paired machines with their load and route.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Answer paired machines (TCP, or one session on stdin/stdout for SSH).
    Serve {
        /// `host:port` (default from peers.toml, `0.0.0.0:47321`).
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        stdio: bool,
        /// Start in the background and return.
        #[arg(long, conflicts_with = "stdio")]
        background: bool,
    },
    /// Run a task on a peer (`auto` = the least busy one); a copy of this project goes along.
    Run {
        peer: String,
        task: Vec<String>,
        /// Run in this directory on the peer (its grant must allow it) instead of a copy.
        #[arg(long)]
        dir: Option<String>,
        /// Apply the result without asking.
        #[arg(long)]
        yes: bool,
        /// Start and return; follow later with `kor peer attach`.
        #[arg(long)]
        detach: bool,
    },
    /// Follow a task on a peer again (no task: list them).
    Attach { peer: String, task: Option<String> },
    /// What a task changed, as a diff; `--apply` writes it here.
    Result {
        peer: String,
        task: String,
        #[arg(long)]
        apply: bool,
    },
    /// Sync memory with a peer (three-way merge, both directions).
    Sync { peer: String },
    /// Set what a peer may do on this machine (read-only by default).
    Allow {
        peer: String,
        /// A directory it may run tasks in (repeatable).
        #[arg(long = "dir")]
        dirs: Vec<String>,
        /// It may change files in those directories.
        #[arg(long)]
        write: bool,
        /// A command prefix it may run (repeatable; `*` = any).
        #[arg(long = "command")]
        commands: Vec<String>,
        /// An extra tool (`WebFetch`, `network`, `mcp__…`; repeatable).
        #[arg(long = "tool")]
        tools: Vec<String>,
        /// Its records are merged into this machine's memory.
        #[arg(long)]
        memory: bool,
        /// Back to read-only.
        #[arg(long)]
        reset: bool,
    },
    /// How to reach a peer: its SSH target and addresses.
    Route {
        peer: String,
        #[arg(long)]
        ssh: Option<String>,
        #[arg(long = "addr")]
        addrs: Vec<String>,
    },
    /// Forget a paired machine.
    Remove { peer: String },
    /// Mirror a running session to a peer; if this machine closes, the peer continues it.
    Standby {
        peer: String,
        /// Session id (default: the most recent one).
        #[arg(long)]
        session: Option<String>,
        /// Stop mirroring.
        #[arg(long)]
        release: bool,
        /// Mirror in the background and return.
        #[arg(long)]
        background: bool,
    },
    /// Internal: the worker of a delegated task.
    #[command(hide = true)]
    Worker { peer: String, id: String },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-peer"))
        .mut_subcommand("pair", |s| {
            s.about(l.t("cmd-peer-pair"))
                .mut_arg("join", |a| a.help(l.t("arg-peer-join")))
                .mut_arg("ssh", |a| a.help(l.t("arg-peer-ssh")))
                .mut_arg("addrs", |a| a.help(l.t("arg-peer-addr")))
        })
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-peer-list"))
                .mut_arg("json", |a| a.help(l.t("arg-peer-json")))
        })
        .mut_subcommand("serve", |s| {
            s.about(l.t("cmd-peer-serve"))
                .mut_arg("listen", |a| a.help(l.t("arg-peer-listen")))
                .mut_arg("stdio", |a| a.help(l.t("arg-peer-stdio")))
                .mut_arg("background", |a| a.help(l.t("arg-peer-background")))
        })
        .mut_subcommand("run", |s| {
            s.about(l.t("cmd-peer-run"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name-auto")))
                .mut_arg("task", |a| a.help(l.t("arg-peer-task")))
                .mut_arg("dir", |a| a.help(l.t("arg-peer-dir")))
                .mut_arg("yes", |a| a.help(l.t("arg-peer-yes")))
                .mut_arg("detach", |a| a.help(l.t("arg-peer-detach")))
        })
        .mut_subcommand("attach", |s| {
            s.about(l.t("cmd-peer-attach"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
                .mut_arg("task", |a| a.help(l.t("arg-peer-task-id")))
        })
        .mut_subcommand("result", |s| {
            s.about(l.t("cmd-peer-result"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
                .mut_arg("task", |a| a.help(l.t("arg-peer-task-id")))
                .mut_arg("apply", |a| a.help(l.t("arg-peer-apply")))
        })
        .mut_subcommand("sync", |s| {
            s.about(l.t("cmd-peer-sync"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
        })
        .mut_subcommand("allow", |s| {
            s.about(l.t("cmd-peer-allow"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
                .mut_arg("dirs", |a| a.help(l.t("arg-peer-allow-dir")))
                .mut_arg("write", |a| a.help(l.t("arg-peer-allow-write")))
                .mut_arg("commands", |a| a.help(l.t("arg-peer-allow-command")))
                .mut_arg("tools", |a| a.help(l.t("arg-peer-allow-tool")))
                .mut_arg("memory", |a| a.help(l.t("arg-peer-allow-memory")))
                .mut_arg("reset", |a| a.help(l.t("arg-peer-allow-reset")))
        })
        .mut_subcommand("route", |s| {
            s.about(l.t("cmd-peer-route"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
                .mut_arg("ssh", |a| a.help(l.t("arg-peer-ssh")))
                .mut_arg("addrs", |a| a.help(l.t("arg-peer-addr")))
        })
        .mut_subcommand("remove", |s| {
            s.about(l.t("cmd-peer-remove"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
        })
        .mut_subcommand("standby", |s| {
            s.about(l.t("cmd-peer-standby"))
                .mut_arg("peer", |a| a.help(l.t("arg-peer-name")))
                .mut_arg("session", |a| a.help(l.t("arg-peer-session")))
                .mut_arg("release", |a| a.help(l.t("arg-peer-release")))
                .mut_arg("background", |a| a.help(l.t("arg-peer-background")))
        })
}

/// Commands that hold the terminal (servers, follow loops, pairing wait): not run from the
/// interactive screen's slash commands.
pub fn blocks_tui(cmd: &PeerCmd) -> bool {
    match cmd {
        PeerCmd::Serve { background, .. } | PeerCmd::Standby { background, .. } => !background,
        PeerCmd::Pair { join, .. } => join.is_none(),
        PeerCmd::Run { detach, .. } => !detach,
        PeerCmd::Attach { task, .. } => task.is_some(),
        PeerCmd::Worker { .. } => true,
        _ => false,
    }
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

/// This machine's node: the key from the configured secret store (keychain or vault file), and
/// the team server relay when this machine joined a team (`kor server join`).
fn node(ctx: &Ctx) -> Result<Node, String> {
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.vault.clone())
        .unwrap_or_default();
    let store = kor_runtime::secure::vault_store(&cfg, None).map_err(|e| e.to_string())?;
    let n = peer::local_node(store.as_ref())?;
    Ok(match &ctx.config {
        Ok(c) => peer::with_team_relay(n, &c.config),
        Err(_) => n,
    })
}

fn entry(node: &Node, name: &str) -> Result<PeerEntry, String> {
    node.book()?
        .get(name)
        .cloned()
        .ok_or_else(|| format!("no paired machine named {name} (kor peer list)"))
}

fn audit_on() {
    if kor_agent::audit::path().is_none() {
        kor_agent::audit::set_path(kor_runtime::secure::audit_path());
    }
}

#[allow(clippy::too_many_lines)]
pub fn run(ctx: &Ctx, cmd: PeerCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    audit_on();
    if let Ok(c) = &ctx.config {
        kor_runtime::apply_network_policy(l, &c.config);
    }
    if let PeerCmd::Worker { peer: p, id } = &cmd {
        let Ok(loaded) = ctx.config.as_ref() else {
            return Ok(fail(l, "config"));
        };
        let dir = peer::home().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
        let rt = crate::providers::runtime()?;
        let r = rt.block_on(peer::worker::run(
            Localizer::new(l.lang()),
            loaded.clone(),
            &dir,
            p,
            id,
        ));
        return Ok(r.map_or_else(|e| fail(l, e), |()| true));
    }
    let node = match node(ctx) {
        Ok(n) => n,
        Err(e) => return Ok(fail(l, e)),
    };
    let rt = crate::providers::runtime()?;
    match cmd {
        PeerCmd::Worker { .. } => Ok(false),
        PeerCmd::Pair { join, ssh, addrs } => rt.block_on(pair_cmd(l, &node, join, ssh.as_deref(), addrs)),
        PeerCmd::List { json } => {
            let loads = rt.block_on(client::loads(&node));
            if json {
                let v: Vec<Value> = loads.iter().map(|(p, r)| peer_json(p, r)).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(true);
            }
            let book = node.book().map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args("peer-self", &args!("name" => book.name.clone(), "fingerprint" => node.identity.fingerprint()))
            );
            if loads.is_empty() {
                println!("{}", l.t("peer-none"));
            }
            for (p, r) in &loads {
                let state = match r {
                    Ok(ld) => l.t_args(
                        "peer-load",
                        &args!("tasks" => ld.tasks, "cpus" => ld.cpus,
                               "busy" => ld.busy.map_or_else(|| "-".to_owned(), |b| format!("{:.0}%", b * 100.0))),
                    ),
                    Err(e) => l.t_args("peer-unreachable", &args!("error" => e.clone())),
                };
                println!("{:<20} {}  {}", p.name, grant_line(l, &p.grant), state);
            }
            if let Some(best) = client::least_busy(&loads) {
                println!("{}", l.t_args("peer-least-busy", &args!("name" => best.name.clone())));
            }
            Ok(true)
        }
        PeerCmd::Serve { listen, background: true, .. } => {
            let mut args = vec!["peer".to_owned(), "serve".to_owned()];
            if let Some(a) = listen {
                args.extend(["--listen".to_owned(), a]);
            }
            Ok(detached(l, &args))
        }
        PeerCmd::Serve { listen, stdio, background: false } => {
            let node = std::sync::Arc::new(node);
            if stdio {
                return Ok(rt.block_on(peer::server::serve_stdio(&node)).is_ok());
            }
            let addr = listen.unwrap_or_else(|| node.book().map(|b| b.listen).unwrap_or_default());
            rt.block_on(async {
                let lst = match tokio::net::TcpListener::bind(&addr).await {
                    Ok(x) => x,
                    Err(e) => return Ok(fail(l, format!("{addr}: {e}"))),
                };
                eprintln!(
                    "{}",
                    l.t_args("peer-serving", &args!("addr" => addr.clone(), "fingerprint" => node.identity.fingerprint()))
                );
                // Also reachable through the team server, and remote browser extensions reach
                // this machine's Kor bridge the same way (§31.1, §18.4).
                if let Some(r) = node.relay.clone() {
                    let name = node.book().map(|b| b.name).unwrap_or_default();
                    eprintln!("{}", l.t_args("peer-relay-on", &args!("server" => r.server.clone(), "name" => name.clone())));
                    tokio::spawn(peer::relay::serve(node.clone(), r.server.clone(), r.material.clone()));
                    tokio::spawn(peer::relay::serve_extension(
                        r.server,
                        r.material,
                        name,
                        peer::relay::LOCAL_BRIDGE.to_owned(),
                    ));
                }
                Ok(peer::server::listen(node.clone(), lst).await.map_or_else(|e| fail(l, e), |()| true))
            })
        }
        PeerCmd::Run { peer: name, task, dir, yes, detach } => {
            let text = task.join(" ");
            rt.block_on(run_task(l, &node, &name, &text, dir, yes, detach))
        }
        PeerCmd::Attach { peer: name, task } => rt.block_on(async {
            let e = match entry(&node, &name) {
                Ok(e) => e,
                Err(err) => return Ok(fail(l, err)),
            };
            let link = match client::attach(&node, &e, task.as_deref(), 0).await {
                Ok(x) => x,
                Err(err) => return Ok(fail(l, err)),
            };
            follow(l, &node, &e, link, task.is_none(), false).await
        }),
        PeerCmd::Result { peer: name, task, apply } => rt.block_on(async {
            let e = match entry(&node, &name) {
                Ok(e) => e,
                Err(err) => return Ok(fail(l, err)),
            };
            Ok(show_result(l, &node, &e, &task, apply).await)
        }),
        PeerCmd::Sync { peer: name } => rt.block_on(async {
            let e = match entry(&node, &name) {
                Ok(e) => e,
                Err(err) => return Ok(fail(l, err)),
            };
            match client::sync(&node, &e).await {
                Ok(r) => {
                    println!(
                        "{}",
                        l.t_args(
                            "peer-synced",
                            &args!("name" => e.name.clone(), "records" => r.records, "added" => r.local.added,
                                   "updated" => r.local.updated, "removed" => r.local.removed,
                                   "conflicts" => r.conflicts.len())
                        )
                    );
                    if !r.remote_applied {
                        println!("{}", l.t_args("peer-sync-readonly", &args!("name" => e.name.clone())));
                    }
                    for c in &r.conflicts {
                        println!("  ! {} ({})", c.task, c.field);
                    }
                    Ok(true)
                }
                Err(err) => Ok(fail(l, err)),
            }
        }),
        PeerCmd::Allow { peer: name, dirs, write, commands, tools, memory, reset } => {
            let mut book = node.book().map_err(anyhow::Error::msg)?;
            let Some(p) = book.peers.iter_mut().find(|p| p.name == name) else {
                return Ok(fail(l, format!("no paired machine named {name}")));
            };
            if reset {
                p.grant = peer::Grant::default();
            }
            for d in dirs {
                let abs = std::path::Path::new(&d)
                    .canonicalize()
                    .map(|x| kor_tools::without_verbatim(&x).display().to_string())
                    .unwrap_or(d);
                if !p.grant.dirs.contains(&abs) {
                    p.grant.dirs.push(abs);
                }
            }
            p.grant.write |= write;
            p.grant.memory |= memory;
            for c in commands {
                if !p.grant.commands.contains(&c) {
                    p.grant.commands.push(c);
                }
            }
            for t in tools {
                if !p.grant.tools.contains(&t) {
                    p.grant.tools.push(t);
                }
            }
            let line = grant_line(l, &p.grant);
            peer::audit("grant", &name, serde_json::to_value(&p.grant).unwrap_or(Value::Null));
            book.save(&node.dir).map_err(anyhow::Error::msg)?;
            println!("{name}: {line}");
            Ok(true)
        }
        PeerCmd::Route { peer: name, ssh, addrs } => {
            let mut book = node.book().map_err(anyhow::Error::msg)?;
            let Some(p) = book.peers.iter_mut().find(|p| p.name == name) else {
                return Ok(fail(l, format!("no paired machine named {name}")));
            };
            if ssh.is_some() {
                p.ssh = ssh;
            }
            if !addrs.is_empty() {
                p.addrs = addrs;
            }
            let routes: Vec<String> = client::routes(p, true).iter().map(client::Route::label).collect();
            book.save(&node.dir).map_err(anyhow::Error::msg)?;
            println!("{name}: {}", routes.join(" → "));
            Ok(true)
        }
        PeerCmd::Remove { peer: name } => {
            let mut book = node.book().map_err(anyhow::Error::msg)?;
            if let Err(e) = book.remove(&name) {
                return Ok(fail(l, e));
            }
            book.save(&node.dir).map_err(anyhow::Error::msg)?;
            peer::audit("removed", &name, json!({}));
            println!("{}", l.t_args("peer-removed", &args!("name" => name)));
            Ok(true)
        }
        PeerCmd::Standby { peer: name, session, release: false, background: true } => {
            let mut args = vec!["peer".to_owned(), "standby".to_owned(), name];
            if let Some(s) = session {
                args.extend(["--session".to_owned(), s]);
            }
            Ok(detached(l, &args))
        }
        PeerCmd::Standby { peer: name, session, release, .. } => rt.block_on(standby_cmd(l, &node, &name, session, release)),
    }
}

/// `kor <args>` as a detached process (`--background`), output to `<peer dir>/<first arg>.log`.
fn detached(l: &Localizer, args: &[String]) -> bool {
    use kor_platform::Detach as _;
    let (Ok(exe), Some(dir)) = (std::env::current_exe(), peer::home()) else {
        return fail(l, "no data directory");
    };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(log) = std::fs::File::create(dir.join(format!(
        "{}.log",
        args.get(1).map_or("peer", String::as_str)
    ))) else {
        return fail(l, "log file");
    };
    let err = log.try_clone().ok();
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(log);
    if let Some(e) = err {
        cmd.stderr(e);
    }
    match cmd.detach().spawn() {
        Ok(c) => {
            println!("{}", l.t_args("peer-background", &args!("pid" => c.id())));
            true
        }
        Err(e) => fail(l, e),
    }
}

fn grant_line(l: &Localizer, g: &peer::Grant) -> String {
    if g == &peer::Grant::default() {
        return l.t("peer-grant-readonly");
    }
    let mut parts = Vec::new();
    if !g.dirs.is_empty() {
        parts.push(format!(
            "{}{}",
            g.dirs.join(","),
            if g.write { " (rw)" } else { " (ro)" }
        ));
    }
    if !g.commands.is_empty() {
        parts.push(format!("cmd: {}", g.commands.join(",")));
    }
    if !g.tools.is_empty() {
        parts.push(format!("tools: {}", g.tools.join(",")));
    }
    if g.memory {
        parts.push("memory".into());
    }
    parts.join("; ")
}

fn peer_json(p: &PeerEntry, r: &Result<client::Load, String>) -> Value {
    json!({
        "name": p.name, "fingerprint": p.key_bytes().map(|k| peer::fingerprint(&k)),
        "addrs": p.addrs, "ssh": p.ssh, "grant": p.grant,
        "load": r.as_ref().ok(), "error": r.as_ref().err(),
    })
}

async fn pair_cmd(
    l: &Localizer,
    node: &Node,
    join: Option<String>,
    ssh: Option<&str>,
    addrs: Vec<String>,
) -> anyhow::Result<bool> {
    if let Some(code) = join {
        let code = match pair::Code::parse(&code) {
            Ok(c) => c,
            Err(e) => return Ok(fail(l, e)),
        };
        return Ok(match pair::join(node, &code, ssh).await {
            Ok(e) => {
                println!(
                    "{}",
                    l.t_args("peer-paired", &args!("name" => e.name.clone(),
                        "fingerprint" => e.key_bytes().map(|k| peer::fingerprint(&k)).unwrap_or_default()))
                );
                true
            }
            Err(e) => fail(l, e),
        });
    }
    let book = node.book().map_err(anyhow::Error::msg)?;
    let addrs = if addrs.is_empty() {
        client::advertised_addrs(&book.listen)
    } else {
        addrs
    };
    let code = match pair::create(&node.dir, addrs) {
        Ok(c) => c,
        Err(e) => return Ok(fail(l, e)),
    };
    // No `kor peer serve` running: listen here until the code is used or expires.
    let own = tokio::net::TcpListener::bind(&book.listen).await.ok();
    println!("{}", pair::qr_text(&code));
    println!(
        "{}",
        l.t_args(
            "peer-pair-code",
            &args!("code" => code.render(), "fingerprint" => node.identity.fingerprint())
        )
    );
    let n = std::sync::Arc::new(node.clone());
    if let Some(lst) = own {
        let n2 = n.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = lst.accept().await {
                let n3 = n2.clone();
                tokio::spawn(async move {
                    let _ = peer::server::serve_conn(&n3, s).await;
                });
            }
        });
    }
    let deadline = std::time::Instant::now() + pair::TTL;
    while pair::is_pending(&node.dir, code.id) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = node.book().map_err(anyhow::Error::msg)?;
    if after.peers != book.peers {
        let newest = after.peers.iter().max_by_key(|p| p.paired_at).cloned();
        if let Some(p) = newest {
            println!(
                "{}",
                l.t_args("peer-paired", &args!("name" => p.name.clone(),
                    "fingerprint" => p.key_bytes().map(|k| peer::fingerprint(&k)).unwrap_or_default()))
            );
        }
        return Ok(true);
    }
    Ok(fail(l, l.t("peer-pair-expired")))
}

/// `kor task --on <peer> "<goal>"` (spec wording `kor task run --on <peer>`).
pub fn run_on(ctx: &Ctx, name: &str, goal: &str) -> anyhow::Result<bool> {
    let goal = goal.strip_prefix("run ").unwrap_or(goal);
    run(
        ctx,
        PeerCmd::Run {
            peer: name.to_owned(),
            task: vec![goal.to_owned()],
            dir: None,
            yes: false,
            detach: false,
        },
    )
}

async fn run_task(
    l: &Localizer,
    node: &Node,
    name: &str,
    text: &str,
    dir: Option<String>,
    yes: bool,
    detach: bool,
) -> anyhow::Result<bool> {
    if text.trim().is_empty() {
        return Ok(fail(l, l.t("peer-empty-task")));
    }
    let e = if name == "auto" {
        let loads = client::loads(node).await;
        match client::least_busy(&loads) {
            Some(p) => {
                eprintln!(
                    "{}",
                    l.t_args("peer-least-busy", &args!("name" => p.name.clone()))
                );
                p.clone()
            }
            None => return Ok(fail(l, l.t("peer-none-reachable"))),
        }
    } else {
        match entry(node, name) {
            Ok(e) => e,
            Err(err) => return Ok(fail(l, err)),
        }
    };
    let ws = if let Some(path) = dir {
        client::Workspace::Dir { path }
    } else {
        let root = std::env::current_dir()?;
        match snapshot::collect(&root) {
            Ok(b) => {
                if !b.skipped.is_empty() {
                    eprintln!(
                        "{}",
                        l.t_args("peer-skipped", &args!("count" => b.skipped.len()))
                    );
                }
                client::Workspace::Snapshot { bundle: b }
            }
            Err(err) => return Ok(fail(l, err)),
        }
    };
    let link = match client::run(node, &e, text, &ws).await {
        Ok(x) => x,
        Err(err) => return Ok(fail(l, err)),
    };
    if detach {
        let mut link = link;
        while let Ok(v) = link.recv_within(Duration::from_secs(60)).await {
            if v["type"] == "error" {
                return Ok(fail(l, v["message"].as_str().unwrap_or_default()));
            }
            if let Some(t) = v["task"].as_str().filter(|_| v["type"] == "attached") {
                println!(
                    "{}",
                    l.t_args(
                        "peer-detached",
                        &args!("task" => t.to_owned(), "name" => e.name.clone())
                    )
                );
                return Ok(true);
            }
        }
        return Ok(false);
    }
    follow(l, node, &e, link, false, yes).await
}

/// Stream a task's events, ask the approvals here, then bring the result back.
async fn follow(
    l: &Localizer,
    node: &Node,
    e: &PeerEntry,
    mut link: peer::noise::Link,
    list_only: bool,
    yes: bool,
) -> anyhow::Result<bool> {
    let interactive = std::io::stdin().is_terminal();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for input in std::io::stdin().lock().lines() {
            let Ok(input) = input else { break };
            if tx.send(input).is_err() {
                break;
            }
        }
    });
    let mut asking: Option<String> = None;
    let mut task: Option<String> = None;
    loop {
        tokio::select! {
            v = link.recv() => {
                let Some(v) = v else {
                    eprintln!("{}", l.t_args("peer-lost", &args!("task" => task.clone().unwrap_or_default(), "name" => e.name.clone())));
                    return Ok(false);
                };
                if v["type"] == "tasks" {
                    crate::remote_cmd::print_tasks(l, &v["tasks"]);
                    if list_only {
                        return Ok(true);
                    }
                }
                if v["type"] == "error" && task.is_none() {
                    return Ok(fail(l, v["message"].as_str().unwrap_or_default()));
                }
                if let Some(t) = v["task"].as_str() {
                    task = Some(t.to_owned());
                }
                if let Some(id) = crate::remote_cmd::show(l, &v) {
                    if interactive {
                        asking = Some(id);
                    } else {
                        let _ = link.send(serde_json::to_value(Command::Approve { id, approval: "no".into(), files: None })?);
                    }
                }
                if v["type"] == "done" {
                    let Some(t) = task.clone() else { return Ok(true) };
                    drop(link);
                    return Ok(show_result(l, node, e, &t, yes).await);
                }
            }
            input = rx.recv() => {
                let Some(input) = input else {
                    eprintln!("{}", l.t_args("peer-detached", &args!("task" => task.clone().unwrap_or_default(), "name" => e.name.clone())));
                    return Ok(true);
                };
                if let Some(id) = asking.take() {
                    let _ = link.send(serde_json::to_value(Command::Approve { id, approval: crate::remote_cmd::answer(&input).into(), files: None })?);
                } else if input.trim() == "/abort" {
                    let _ = link.send(serde_json::to_value(Command::Abort)?);
                } else if input.trim() == "/exit" {
                    eprintln!("{}", l.t_args("peer-detached", &args!("task" => task.clone().unwrap_or_default(), "name" => e.name.clone())));
                    return Ok(true);
                } else if let Some(t) = task.clone().filter(|_| !input.trim().is_empty()) {
                    let _ = link.send(serde_json::to_value(Command::Send { text: input, task: Some(t) })?);
                }
            }
        }
    }
}

async fn show_result(l: &Localizer, node: &Node, e: &PeerEntry, task: &str, apply: bool) -> bool {
    let r = match client::result(node, e, task).await {
        Ok(r) => r,
        Err(err) => return fail(l, err),
    };
    if r.changes.is_empty() {
        println!("{}", l.t("peer-no-changes"));
        return true;
    }
    print!("{}", r.diff);
    let _ = std::io::stdout().flush();
    let go = apply
        || (std::io::stdin().is_terminal() && {
            eprint!(
                "{}",
                l.t_args("peer-apply-ask", &args!("count" => r.changes.len()))
            );
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
            crate::remote_cmd::answer(&s) != "no"
        });
    if !go {
        println!(
            "{}",
            l.t_args(
                "peer-apply-later",
                &args!("name" => e.name.clone(), "task" => task.to_owned())
            )
        );
        return true;
    }
    let root = std::env::current_dir().unwrap_or_default();
    let res = snapshot::apply(&root, &r.changes, None);
    let mut ok = true;
    for a in &res {
        let line = l.t_args(
            &format!("peer-applied-{}", a.status),
            &args!("path" => a.path.clone()),
        );
        if a.status == "conflict" || a.status == "failed" {
            ok = false;
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    peer::audit(
        "apply",
        &e.name,
        json!({ "task": task, "files": res.len(), "ok": ok }),
    );
    ok
}

fn latest_session() -> Option<(String, PathBuf)> {
    let dir = kor_runtime::sessions_dir()?;
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| {
            (
                e.path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                e.path(),
            )
        })
}

async fn standby_cmd(
    l: &Localizer,
    node: &Node,
    name: &str,
    session: Option<String>,
    release: bool,
) -> anyhow::Result<bool> {
    let e = match entry(node, name) {
        Ok(e) => e,
        Err(err) => return Ok(fail(l, err)),
    };
    let (sid, path) = match session {
        Some(s) if kor_runtime::valid_session_id(&s) => {
            let p = kor_runtime::sessions_dir()
                .unwrap_or_default()
                .join(format!("{s}.jsonl"));
            (s, p)
        }
        Some(_) => return Ok(fail(l, "invalid session id")),
        None => match latest_session() {
            Some(x) => x,
            None => return Ok(fail(l, l.t("peer-no-session"))),
        },
    };
    if release {
        return Ok(match standby::release(node, &e, &sid).await {
            Ok(Some(t)) => {
                println!(
                    "{}",
                    l.t_args(
                        "peer-taken-over",
                        &args!("task" => t, "name" => e.name.clone())
                    )
                );
                true
            }
            Ok(None) => {
                println!("{}", l.t("peer-standby-stopped"));
                true
            }
            Err(err) => fail(l, err),
        });
    }
    let mut m = standby::Mirror {
        session: sid.clone(),
        transcript: path,
        project: std::env::current_dir()?,
        sent_digest: None,
    };
    println!(
        "{}",
        l.t_args(
            "peer-standby-on",
            &args!("session" => sid, "name" => e.name.clone())
        )
    );
    loop {
        match standby::renew(node, &e, &mut m).await {
            Ok(standby::Renewed::TakenOver(t)) => {
                println!(
                    "{}",
                    l.t_args(
                        "peer-taken-over",
                        &args!("task" => t, "name" => e.name.clone())
                    )
                );
                return Ok(true);
            }
            Ok(standby::Renewed::Held) => {}
            // Unreachable for a moment: keep trying; the lease covers short gaps.
            Err(err) => eprintln!("{}", l.t_args("error-generic", &args!("error" => err))),
        }
        tokio::time::sleep(standby::INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: PeerCmd,
    }

    #[test]
    fn args_parse() {
        let t = T::try_parse_from(["peer", "run", "auto", "fix", "the", "tests", "--yes"]).unwrap();
        assert!(
            matches!(t.c, PeerCmd::Run { ref peer, ref task, yes: true, .. } if peer == "auto" && task.len() == 3)
        );
        let t = T::try_parse_from([
            "peer",
            "allow",
            "laptop",
            "--dir",
            "/srv",
            "--command",
            "cargo test",
            "--memory",
        ])
        .unwrap();
        assert!(matches!(t.c, PeerCmd::Allow { ref dirs, memory: true, .. } if dirs.len() == 1));
        let t = T::try_parse_from(["peer", "pair", "--join", "KOR1-AAAA@x:1", "--ssh", "me@srv"])
            .unwrap();
        assert!(matches!(
            t.c,
            PeerCmd::Pair {
                join: Some(_),
                ssh: Some(_),
                ..
            }
        ));
        assert!(T::try_parse_from(["peer", "serve", "--stdio"]).is_ok());
        assert!(T::try_parse_from(["peer", "standby", "srv", "--release"]).is_ok());
        assert!(T::try_parse_from(["peer", "worker", "srv", "r-1"]).is_ok());
        assert!(blocks_tui(&T::try_parse_from(["peer", "serve"]).unwrap().c));
        assert!(!blocks_tui(
            &T::try_parse_from(["peer", "serve", "--background"])
                .unwrap()
                .c
        ));
        assert!(!blocks_tui(
            &T::try_parse_from(["peer", "run", "srv", "x", "--detach"])
                .unwrap()
                .c
        ));
        assert!(T::try_parse_from(["peer", "serve", "--stdio", "--background"]).is_err());
    }
}
