//! Reaching a peer (ADR 0057) and the requests one machine makes of another.
//!
//! Routes, in order: the peer's Tailscale addresses when this machine is on a tailnet (an
//! existing encrypted overlay), its direct addresses, then SSH (`ssh <host> kor peer serve
//! --stdio`, the user's own keys and known hosts). Every route carries the same Noise session,
//! so the peer's key is checked whichever way the bytes travel. Privacy and offline mode allow
//! loopback only (`kor_core::net`). A team-server relay is the Faz 8 path (spec §11), not here.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpStream;

use super::noise::{self, Link, Mode};
use super::snapshot::{Bundle, Change};
use super::{Node, PeerEntry};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// A byte stream to a peer.
pub trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static> Stream for T {}

/// This machine's Tailscale IPv4 addresses (`tailscale ip -4`); empty without Tailscale.
pub fn tailscale_ips() -> Vec<String> {
    use kor_platform::NoWindow as _;
    let Ok(out) = std::process::Command::new("tailscale")
        .args(["ip", "-4"])
        .no_window()
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| l.parse::<std::net::Ipv4Addr>().is_ok())
        .map(str::to_owned)
        .collect()
}

/// The LAN address the OS would use for outside traffic (no packet is sent).
fn lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // TEST-NET-1: never routed; `connect` on UDP only picks the interface.
    s.connect(("192.0.2.1", 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| ip.to_string())
}

/// Addresses to put in a pairing code / hello for a listener on `listen`.
pub fn advertised_addrs(listen: &str) -> Vec<String> {
    let Some((host, port)) = listen.rsplit_once(':') else {
        return Vec::new();
    };
    if host != "0.0.0.0" && host != "[::]" && host != "::" {
        return vec![listen.to_owned()];
    }
    let mut out: Vec<String> = tailscale_ips()
        .into_iter()
        .map(|ip| format!("ts:{ip}:{port}"))
        .collect();
    if let Some(ip) = lan_ip() {
        out.push(format!("{ip}:{port}"));
    }
    out
}

/// How a peer can be reached, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `kind` is `tailscale` or `direct`.
    Tcp { addr: String, kind: &'static str },
    Ssh { target: String },
}

impl Route {
    pub fn label(&self) -> String {
        match self {
            Self::Tcp { addr, kind } => format!("{kind} {addr}"),
            Self::Ssh { target } => format!("ssh {target}"),
        }
    }
}

pub fn routes(e: &PeerEntry, on_tailnet: bool) -> Vec<Route> {
    let mut out = Vec::new();
    if on_tailnet {
        for a in e.addrs.iter().filter_map(|a| a.strip_prefix("ts:")) {
            out.push(Route::Tcp {
                addr: a.to_owned(),
                kind: "tailscale",
            });
        }
    }
    for a in e.addrs.iter().filter(|a| !a.starts_with("ts:")) {
        out.push(Route::Tcp {
            addr: a.clone(),
            kind: "direct",
        });
    }
    if let Some(s) = &e.ssh {
        out.push(Route::Ssh { target: s.clone() });
    }
    out
}

/// TCP to `host:port` within 4 s, after the network guard.
///
/// # Errors
/// Refused by privacy/offline mode, or no connection.
pub async fn tcp(addr: &str) -> Result<TcpStream, String> {
    let host = addr
        .rsplit_once(':')
        .map_or(addr, |(h, _)| h)
        .trim_start_matches('[')
        .trim_end_matches(']');
    kor_core::net::check_host(host).map_err(|b| b.message)?;
    let s = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let _ = s.set_nodelay(true);
    Ok(s)
}

/// `ssh <target> kor peer serve --stdio` as one byte stream.
///
/// # Errors
/// Refused by privacy/offline mode, an unknown target or ssh missing.
pub fn ssh_stream(target: &str) -> Result<impl Stream, String> {
    if let Some(b) = kor_core::net::refusal(target) {
        return Err(b.message);
    }
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let h = kor_agent::remote::resolve_target(target, file.as_deref())?;
    let mut child = kor_agent::remote::ssh_command(&h, "kor peer serve --stdio")
        .spawn()
        .map_err(|e| format!("could not start ssh ({e}); install the OpenSSH client"))?;
    let out = child.stdout.take().ok_or("no stdout")?;
    let inp = child.stdin.take().ok_or("no stdin")?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(tokio::io::join(out, inp))
}

/// A Noise session with `e` over the first route that answers; the route's label.
///
/// # Errors
/// When every route fails (the message lists each).
pub async fn dial(node: &Node, e: &PeerEntry) -> Result<(Link, String), String> {
    let key = e.key_bytes().ok_or("damaged peer key")?;
    let hello = node.hello()?;
    let on_tailnet = !tailscale_ips().is_empty();
    let mut errors = Vec::new();
    for r in routes(e, on_tailnet) {
        let io: Box<dyn StreamBox> = match &r {
            Route::Tcp { addr, .. } => match tcp(addr).await {
                Ok(s) => Box::new(s),
                Err(err) => {
                    errors.push(format!("{}: {err}", r.label()));
                    continue;
                }
            },
            Route::Ssh { target } => match ssh_stream(target) {
                Ok(s) => Box::new(s),
                Err(err) => {
                    errors.push(format!("{}: {err}", r.label()));
                    continue;
                }
            },
        };
        match noise::initiate(io, &node.identity, Mode::Session, None, &hello).await {
            Ok(link) if link.remote == key => return Ok((link, r.label())),
            // Somebody else answered on that address: never talk to it.
            Ok(_) => errors.push(format!("{}: a different key answered", r.label())),
            Err(err) => errors.push(format!("{}: {err}", r.label())),
        }
    }
    if errors.is_empty() {
        return Err(format!("{} has no address; pair again or set its ssh target", e.name));
    }
    Err(format!("could not reach {} ({})", e.name, errors.join("; ")))
}

/// Object-safe stream for routes of different types.
pub trait StreamBox: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> StreamBox for T {}

fn check(v: &Value) -> Result<(), String> {
    if v["type"] == "error" {
        return Err(v["message"].as_str().unwrap_or("peer error").to_owned());
    }
    Ok(())
}

async fn request(node: &Node, e: &PeerEntry, open: Value) -> Result<Value, String> {
    let (mut link, _) = dial(node, e).await?;
    link.send(open)?;
    let v = link.recv_within(ANSWER_TIMEOUT).await?;
    check(&v)?;
    Ok(v)
}

/// How busy a peer is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    pub name: String,
    pub cpus: u32,
    /// Share of CPUs in use (load average ÷ CPUs, or Windows busy time); `None` = not reported.
    pub busy: Option<f64>,
    /// Delegated tasks running or waiting on that machine.
    pub tasks: u32,
    pub version: String,
}

impl Load {
    /// Lower is less busy: running tasks per CPU plus the CPU share in use (0 when unreported).
    pub fn score(&self) -> f64 {
        f64::from(self.tasks) / f64::from(self.cpus.max(1)) + self.busy.unwrap_or(0.0)
    }
}

/// # Errors
/// When the peer cannot be reached.
pub async fn status(node: &Node, e: &PeerEntry) -> Result<Load, String> {
    let v = request(node, e, json!({ "type": "open", "channel": "status" })).await?;
    serde_json::from_value(v["load"].clone()).map_err(|err| err.to_string())
}

/// Every peer's load (unreachable ones with their error), concurrently.
pub async fn loads(node: &Node) -> Vec<(PeerEntry, Result<Load, String>)> {
    let Ok(book) = node.book() else {
        return Vec::new();
    };
    let futs = book.peers.into_iter().map(|p| async move {
        let r = status(node, &p).await;
        (p, r)
    });
    futures::future::join_all(futs).await
}

/// The reachable peer with the lowest [`Load::score`].
pub fn least_busy(loads: &[(PeerEntry, Result<Load, String>)]) -> Option<&PeerEntry> {
    loads
        .iter()
        .filter_map(|(p, l)| l.as_ref().ok().map(|l| (p, l.score())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p)
}

/// Where a delegated task runs on the peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Workspace {
    /// A copy of this project, sent along.
    Snapshot { bundle: Bundle },
    /// A directory on the peer its grant allows.
    Dir { path: String },
}

/// Start a task on the peer; the returned link streams the task's events (remote-serve frames:
/// `hello`, `attached`, `text`, `tool`, `confirm`, `done` …) and takes
/// [`kor_agent::remote::wire::Command`]s (`approve`, `abort`, `detach`, `send` with `task`).
///
/// # Errors
/// When the peer cannot be reached or refuses the task.
pub async fn run(node: &Node, e: &PeerEntry, text: &str, ws: &Workspace) -> Result<Link, String> {
    let (link, route) = dial(node, e).await?;
    super::audit(
        "delegate",
        &e.name,
        json!({ "prompt": kor_agent::audit::clip(text), "route": route,
                "workspace": match ws { Workspace::Snapshot { bundle } => format!("snapshot ({} files)", bundle.files.len()), Workspace::Dir { path } => path.clone() } }),
    );
    link.send(json!({ "type": "open", "channel": "tasks", "new": { "text": text, "workspace": ws } }))?;
    Ok(link)
}

/// Follow an existing task from event `from`.
///
/// # Errors
/// When the peer cannot be reached.
pub async fn attach(node: &Node, e: &PeerEntry, task: Option<&str>, from: u64) -> Result<Link, String> {
    let (link, _) = dial(node, e).await?;
    link.send(json!({ "type": "open", "channel": "tasks", "attach": task, "from": from }))?;
    Ok(link)
}

/// What a finished task changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub task: String,
    pub state: String,
    pub changes: Vec<Change>,
    pub diff: String,
}

/// # Errors
/// When the peer cannot be reached or has no such task.
pub async fn result(node: &Node, e: &PeerEntry, task: &str) -> Result<TaskResult, String> {
    let v = request(node, e, json!({ "type": "open", "channel": "result", "task": task })).await?;
    serde_json::from_value(v["result"].clone()).map_err(|err| err.to_string())
}

/// What a memory sync did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub local: super::merge::Applied,
    /// The peer merged into its own base (its grant for us has `memory`).
    pub remote_applied: bool,
    pub conflicts: Vec<super::merge::Conflict>,
    pub records: usize,
}

/// Three-way merge of both knowledge bases; the peer computes the merge against its stored
/// base for us, both sides apply it (the peer only if it allows our records) and keep it as
/// the next base.
///
/// # Errors
/// When the peer cannot be reached or a knowledge base cannot be read.
pub async fn sync(node: &Node, e: &PeerEntry) -> Result<SyncReport, String> {
    let home = node.knowledge_home.as_deref().ok_or("no knowledge base")?;
    let ours = super::merge::snapshot(super::merge::local_records(home)?);
    let v = request(
        node,
        e,
        json!({ "type": "open", "channel": "sync", "records": ours.values().collect::<Vec<_>>() }),
    )
    .await?;
    let merged: super::merge::Snapshot =
        serde_json::from_value(v["merged"].clone()).map_err(|err| err.to_string())?;
    let conflicts: Vec<super::merge::Conflict> =
        serde_json::from_value(v["conflicts"].clone()).unwrap_or_default();
    let local = super::merge::apply(home, &merged)?;
    super::merge::save_base(&node.dir, &e.name, &merged)?;
    super::audit(
        "sync",
        &e.name,
        json!({ "added": local.added, "updated": local.updated, "removed": local.removed,
                "conflicts": conflicts.len(), "remote_applied": v["applied"].as_bool().unwrap_or(false) }),
    );
    Ok(SyncReport {
        local,
        remote_applied: v["applied"].as_bool().unwrap_or(false),
        records: merged.len(),
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_prefer_tailscale_then_direct_then_ssh() {
        let e = PeerEntry {
            name: "srv".into(),
            key: "00".repeat(32),
            addrs: vec!["10.0.0.2:47321".into(), "ts:100.64.0.9:47321".into()],
            ssh: Some("me@srv".into()),
            grant: super::super::Grant::default(),
            paired_at: 0,
        };
        let r = routes(&e, true);
        assert_eq!(
            r.iter().map(Route::label).collect::<Vec<_>>(),
            ["tailscale 100.64.0.9:47321", "direct 10.0.0.2:47321", "ssh me@srv"]
        );
        assert_eq!(routes(&e, false).len(), 2);
        let l = |tasks, busy| Load {
            name: String::new(),
            cpus: 4,
            busy,
            tasks,
            version: String::new(),
        };
        assert!(l(0, Some(0.1)).score() < l(2, Some(0.0)).score());
        assert!(advertised_addrs("10.1.2.3:5").contains(&"10.1.2.3:5".to_owned()));
    }
}
