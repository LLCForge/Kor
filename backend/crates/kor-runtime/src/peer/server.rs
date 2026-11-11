//! `kor peer serve` (ADR 0057): answers paired peers on a TCP listener or, for the SSH route,
//! on stdin/stdout. A session starts with one `open` frame naming the channel:
//!
//! - `status` → `{ type: "status", load }` (CPUs, CPU share in use, delegated tasks running);
//! - `tasks` with `new: { text, workspace }` → a task in the peer's own store, then the
//!   remote-serve stream of ADR 0044 (`hello`, `attached`, events; commands `approve`, `abort`,
//!   `send` with `task`, `attach`, `tasks`, `detach`); with `attach` → follow an existing task;
//! - `result` → the files the task changed in its snapshot workspace, with a diff;
//! - `sync` → the three-way merged knowledge records;
//! - `standby` → the handoff lease ([`super::standby`]).
//!
//! Unknown keys never get past the handshake; every peer sees only its own tasks.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kor_agent::remote::wire::Command;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use super::client::{Load, TaskResult, Workspace};
use super::noise::{self, Link, Mode};
use super::snapshot::{self, Bundle};
use super::{Grant, Node, PeerEntry, merge, pair, standby};

const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PROMPT: usize = 100_000;

/// What the worker of a delegated task needs (`<task>/peer.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub peer: String,
    /// Where the task runs.
    pub root: PathBuf,
    /// `root` is the task's own snapshot copy (`<task>/work`); `snapshot.json` holds the original.
    pub workspace: bool,
    /// Continued from a handed-off checkpoint.
    #[serde(default)]
    pub handoff: bool,
}

/// # Errors
/// On I/O errors.
pub fn write_meta(task_dir: &Path, m: &Meta) -> Result<(), String> {
    std::fs::write(
        task_dir.join("peer.json"),
        serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn read_meta(task_dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(task_dir.join("peer.json")).ok()?).ok()
}

/// Accept connections until the listener fails; the standby watcher runs alongside.
///
/// # Errors
/// When accepting fails.
pub async fn listen(node: Arc<Node>, listener: TcpListener) -> Result<(), String> {
    tokio::spawn(standby::watch(node.clone()));
    loop {
        let (s, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let _ = s.set_nodelay(true);
        let n = node.clone();
        tokio::spawn(async move {
            let _ = serve_conn(&n, s).await;
        });
    }
}

/// One connection, pairing or session.
///
/// # Errors
/// On a failed handshake or an unknown peer (the connection is simply closed).
pub async fn serve_conn<S>(node: &Node, mut io: S) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    match noise::read_prelude(&mut io).await? {
        Mode::Pair { id } => pair::accept(node, id, io).await.map(|_| ()),
        Mode::Session => {
            let book = node.book()?;
            let link = noise::respond(io, &node.identity, None, &node.hello()?, |k, _| {
                if book.by_key(k).is_some() {
                    Ok(())
                } else {
                    Err("unknown peer".into())
                }
            })
            .await?;
            let peer = book.by_key(&link.remote).cloned().ok_or("unknown peer")?;
            handle(node, peer, link).await
        }
    }
}

fn error(link: &Link, msg: impl Into<String>) {
    let _ = link.send(json!({ "type": "error", "message": msg.into() }));
}

async fn handle(node: &Node, peer: PeerEntry, mut link: Link) -> Result<(), String> {
    let open = link.recv_within(OPEN_TIMEOUT).await?;
    if open["type"] != "open" {
        error(&link, "expected an open frame");
        return Ok(());
    }
    let channel = open["channel"].as_str().unwrap_or_default().to_owned();
    super::audit("connect", &peer.name, json!({ "channel": channel }));
    let r = match channel.as_str() {
        "status" => {
            let load = load(node).await;
            link.send(json!({ "type": "status", "load": load }))
        }
        "tasks" => return tasks(node, &peer, link, &open).await,
        "result" => result(node, &peer, &open).and_then(|r| link.send(json!({ "type": "result", "result": r }))),
        "sync" => sync(node, &peer, &open).and_then(|v| link.send(v)),
        "standby" => standby::serve(node, &peer, &open).and_then(|v| link.send(v)),
        other => Err(format!("unknown channel {other}")),
    };
    if let Err(e) = r {
        error(&link, e);
    }
    // Let the pump flush the answer before the link is dropped.
    let _ = tokio::time::timeout(Duration::from_secs(5), link.recv()).await;
    Ok(())
}

/// Delegated tasks running or waiting for an answer, every peer together.
fn running_tasks(node: &Node) -> u32 {
    let mut n = 0u32;
    for d in std::fs::read_dir(node.dir.join("tasks")).into_iter().flatten().flatten() {
        let store = crate::remote::TaskStore { dir: d.path() };
        n += u32::try_from(
            store
                .list()
                .iter()
                .filter(|t| t.state == "running" || t.state == "waiting")
                .count(),
        )
        .unwrap_or(u32::MAX);
    }
    n
}

/// This machine's load.
pub async fn load(node: &Node) -> Load {
    let busy = tokio::task::spawn_blocking(kor_platform::sysload::busy)
        .await
        .ok()
        .flatten();
    Load {
        name: node.book().map(|b| b.name).unwrap_or_default(),
        cpus: u32::try_from(kor_platform::sysload::cpus()).unwrap_or(u32::MAX),
        busy,
        tasks: running_tasks(node),
        version: kor_core::VERSION.to_owned(),
    }
}

/// `~/x` → the home directory's `x`.
fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")), dirs::home_dir()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if p == "~" => dirs::home_dir().unwrap_or_default(),
        _ => PathBuf::from(p),
    }
}

/// `dir` (on this machine) is inside one of the granted directories.
fn granted_dir(grant: &Grant, dir: &str) -> Result<PathBuf, String> {
    let want = Path::new(dir)
        .canonicalize()
        .map_err(|e| format!("{dir}: {e}"))?;
    for g in &grant.dirs {
        let g = expand_home(g);
        if let Ok(gc) = g.canonicalize()
            && want.starts_with(&gc)
        {
            return Ok(kor_tools::without_verbatim(&want));
        }
    }
    Err(format!("the peer policy of this machine does not grant {dir}"))
}

/// Create a delegated task: its workspace, metadata, first message and worker.
///
/// # Errors
/// On a refused directory, a bad snapshot or I/O errors.
pub fn create_task(
    node: &Node,
    peer: &PeerEntry,
    text: &str,
    ws: &Workspace,
) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > MAX_PROMPT {
        return Err("empty or too long task".into());
    }
    let store = node.store(&peer.name);
    std::fs::create_dir_all(&store.dir).map_err(|e| e.to_string())?;
    let (root, workspace) = match ws {
        Workspace::Dir { path } => (Some(granted_dir(&peer.grant, path)?), false),
        Workspace::Snapshot { .. } => (None, true),
    };
    let id = store.create(
        &root
            .as_ref()
            .map_or_else(|| "snapshot".to_owned(), |r| r.display().to_string()),
        text,
    )?;
    let task_dir = store.task_dir(&id);
    let root = match (root, ws) {
        (Some(r), _) => r,
        (None, Workspace::Snapshot { bundle }) => {
            let work = task_dir.join("work");
            snapshot::extract(bundle, &work)?;
            std::fs::write(
                task_dir.join("snapshot.json"),
                serde_json::to_vec(bundle).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            work.canonicalize()
                .map(|p| kor_tools::without_verbatim(&p))
                .map_err(|e| e.to_string())?
        }
        (None, Workspace::Dir { .. }) => unreachable!("a dir workspace has a root"),
    };
    write_meta(
        &task_dir,
        &Meta {
            peer: peer.name.clone(),
            root,
            workspace,
            handoff: false,
        },
    )?;
    store.push(
        &id,
        &Command::Send {
            text: text.to_owned(),
            task: None,
        },
    )?;
    (node.spawn)(&peer.name, &id)?;
    super::audit(
        "task",
        &peer.name,
        json!({ "task": id, "prompt": kor_agent::audit::clip(text), "workspace": workspace }),
    );
    Ok(id)
}

async fn tasks(node: &Node, peer: &PeerEntry, link: Link, open: &Value) -> Result<(), String> {
    let first = if let Some(new) = open.get("new") {
        let text = new["text"].as_str().unwrap_or_default();
        let ws: Workspace = match serde_json::from_value(new["workspace"].clone()) {
            Ok(w) => w,
            Err(e) => {
                error(&link, format!("bad workspace: {e}"));
                return Ok(());
            }
        };
        match create_task(node, peer, text, &ws) {
            Ok(id) => vec![Command::Attach { task: id, from: 0 }],
            Err(e) => {
                super::audit("denied", &peer.name, json!({ "reason": e }));
                error(&link, e);
                return Ok(());
            }
        }
    } else if let Some(t) = open["attach"].as_str() {
        vec![Command::Attach {
            task: t.to_owned(),
            from: open["from"].as_u64().unwrap_or(0),
        }]
    } else {
        vec![Command::Tasks]
    };
    relay(node, peer, link, first).await
}

/// The remote-serve relay (ADR 0044) over the Noise link, scoped to this peer's tasks.
async fn relay(node: &Node, peer: &PeerEntry, link: Link, first: Vec<Command>) -> Result<(), String> {
    let store = node.store(&peer.name);
    let (mut in_w, in_r) = tokio::io::duplex(1 << 20);
    let (out_w, out_r) = tokio::io::duplex(1 << 20);
    let Link { tx, mut rx, .. } = link;
    let name = peer.name.clone();
    let spawn_fn = node.spawn.clone();
    let spawn = move |id: &str| spawn_fn(&name, id);
    let err_tx = tx.clone();
    let feeder = async move {
        for c in first {
            if in_w.write_all(c.frame().as_bytes()).await.is_err() {
                return;
            }
        }
        while let Some(v) = rx.recv().await {
            match serde_json::from_value::<Command>(v) {
                // New tasks come with a workspace through `open`, never as a bare send.
                Ok(Command::Send { task: None, .. }) => {
                    let _ = err_tx.send(json!({ "type": "error", "message": "start tasks with kor peer run" }));
                }
                Ok(c) => {
                    if in_w.write_all(c.frame().as_bytes()).await.is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = err_tx.send(json!({ "type": "error", "message": format!("bad command: {e}") }));
                }
            }
        }
        // Client gone: closing the input ends the relay; the task keeps running.
    };
    let pump = async move {
        let mut lines = BufReader::new(out_r).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if let Ok(v) = serde_json::from_str::<Value>(&l)
                && tx.send(v).is_err()
            {
                break;
            }
        }
    };
    let project = node.dir.clone();
    let serve = async move {
        let r = crate::remote::serve(BufReader::new(in_r), out_w, &store, &project, &spawn).await;
        drop(store);
        r
    };
    let ((), (), r) = tokio::join!(feeder, pump, serve);
    r
}

fn result(node: &Node, peer: &PeerEntry, open: &Value) -> Result<TaskResult, String> {
    let task = open["task"].as_str().unwrap_or_default();
    let store = node.store(&peer.name);
    if !store.exists(task) {
        return Err(format!("no task {task}"));
    }
    let dir = store.task_dir(task);
    let meta = read_meta(&dir).ok_or("task metadata missing")?;
    let state = store.state(task).map(|s| s.state).unwrap_or_default();
    if !meta.workspace {
        return Ok(TaskResult {
            task: task.to_owned(),
            state,
            ..TaskResult::default()
        });
    }
    let bundle: Bundle = serde_json::from_slice(
        &std::fs::read(dir.join("snapshot.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let changes = snapshot::changes(&bundle, &meta.root)?;
    let diff = snapshot::diff(&changes);
    super::audit("result", &peer.name, json!({ "task": task, "files": changes.len() }));
    Ok(TaskResult {
        task: task.to_owned(),
        state,
        changes,
        diff,
    })
}

fn sync(node: &Node, peer: &PeerEntry, open: &Value) -> Result<Value, String> {
    let home = node.knowledge_home.as_deref().ok_or("no knowledge base here")?;
    let theirs: Vec<kor_memory::Record> =
        serde_json::from_value(open["records"].clone()).map_err(|e| format!("bad records: {e}"))?;
    let ours = merge::snapshot(merge::local_records(home)?);
    let base = merge::load_base(&node.dir, &peer.name);
    let out = merge::merge(&base, &ours, &merge::snapshot(theirs));
    // Read-only by default: the peer gets our records, ours change only if we allow it.
    let applied = peer.grant.memory;
    if applied {
        let a = merge::apply(home, &out.merged)?;
        merge::save_base(&node.dir, &peer.name, &out.merged)?;
        super::audit(
            "sync",
            &peer.name,
            json!({ "added": a.added, "updated": a.updated, "removed": a.removed, "conflicts": out.conflicts.len() }),
        );
    } else {
        super::audit("sync", &peer.name, json!({ "read_only": true }));
    }
    Ok(json!({ "type": "synced", "merged": out.merged, "conflicts": out.conflicts, "applied": applied }))
}

/// `kor peer serve --stdio`: one session on this process's stdin/stdout (the SSH route).
///
/// # Errors
/// As [`serve_conn`].
pub async fn serve_stdio(node: &Node) -> Result<(), String> {
    let io = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    serve_conn(node, io).await
}
