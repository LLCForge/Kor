//! Handoff when the laptop closes (spec §31.1, ADR 0057). While a session runs on this machine,
//! `kor peer standby <peer>` (or the desktop's "devret" switch) mirrors its checkpoint — the
//! saved transcript and, when it changed, a project snapshot — to the peer every
//! [`INTERVAL`] under a lease of [`LEASE`]. A closed lid or a lost network stops the renewals;
//! when the lease runs out while the session was mid-task (the last message waits for the model
//! or for tool results), the peer starts a worker that resumes the transcript from that
//! checkpoint (the remote-serve worker of ADR 0044). The next renewal from the laptop learns the
//! task id, stops mirroring and can attach to it. No OS sleep hooks are needed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kor_agent::remote::wire::Command;
use kor_core::{ContentPart, Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::client::dial;
use super::snapshot::{self, Bundle};
use super::{Node, PeerEntry};

/// How often a running session is mirrored.
pub const INTERVAL: Duration = Duration::from_secs(30);
/// How long a mirror stays valid without a renewal.
pub const LEASE: Duration = Duration::from_secs(120);
const SCAN: Duration = Duration::from_secs(5);
const MAX_TRANSCRIPT: usize = 32 * 1024 * 1024;

/// The message the peer continues with.
pub const HANDOFF_PROMPT: &str = "[Handoff] The machine this session ran on went offline in the \
middle of the task, so it was handed off to this machine with the conversation so far and a copy \
of the project. Continue the task from where it stopped; do not redo finished steps.";

/// The session stopped mid-task: the model owes an answer or tool results are pending.
pub fn unfinished(messages: &[Message]) -> bool {
    match messages.last() {
        None => false,
        Some(m) if m.role == Role::Assistant => m
            .parts
            .iter()
            .any(|p| matches!(p, ContentPart::ToolUse { .. })),
        Some(m) => matches!(m.role, Role::User | Role::Tool),
    }
}

/// The peer's lease record (`<peer dir>/standby/<peer>/<session>/lease.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Lease {
    pub session: String,
    pub expires: u64,
    pub unfinished: bool,
    pub digest: String,
    /// The task the peer continued the session as.
    pub taken_over: Option<String>,
}

fn dir_of(node: &Node, peer: &str, session: &str) -> PathBuf {
    node.dir.join("standby").join(peer).join(session)
}

fn read_lease(d: &Path) -> Option<Lease> {
    serde_json::from_slice(&std::fs::read(d.join("lease.json")).ok()?).ok()
}

fn write_lease(d: &Path, l: &Lease) -> Result<(), String> {
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let tmp = d.join("lease.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(l).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, d.join("lease.json")).map_err(|e| e.to_string())
}

/// Peer side of one `standby` request: store the checkpoint and renew, or release.
///
/// # Errors
/// On a bad session id, a damaged frame or I/O errors.
pub fn serve(node: &Node, peer: &PeerEntry, open: &Value) -> Result<Value, String> {
    let session = open["session"].as_str().unwrap_or_default();
    if !crate::valid_session_id(session) {
        return Err("invalid session id".into());
    }
    let d = dir_of(node, &peer.name, session);
    if open["release"].as_bool() == Some(true) {
        let taken = read_lease(&d).and_then(|l| l.taken_over);
        if taken.is_none() {
            let _ = std::fs::remove_dir_all(&d);
        }
        return Ok(json!({ "type": "lease", "takenOver": taken }));
    }
    let mut lease = read_lease(&d).unwrap_or_default();
    if lease.taken_over.is_some() {
        return Ok(json!({ "type": "lease", "takenOver": lease.taken_over }));
    }
    let transcript = open["transcript"].as_str().unwrap_or_default();
    if transcript.len() > MAX_TRANSCRIPT {
        return Err("transcript too large".into());
    }
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(d.join("transcript.jsonl"), transcript).map_err(|e| e.to_string())?;
    let digest = open["digest"].as_str().unwrap_or_default().to_owned();
    if let Some(b) = open.get("bundle").filter(|b| !b.is_null()) {
        let bundle: Bundle = serde_json::from_value(b.clone()).map_err(|e| e.to_string())?;
        std::fs::write(
            d.join("bundle.json"),
            serde_json::to_vec(&bundle).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        lease.digest.clone_from(&digest);
    }
    let need_bundle = lease.digest != digest || !d.join("bundle.json").is_file();
    let lease_ms = open["leaseMs"]
        .as_u64()
        .unwrap_or_else(|| u64::try_from(LEASE.as_millis()).unwrap_or(u64::MAX))
        .clamp(10_000, 3_600_000);
    session.clone_into(&mut lease.session);
    lease.expires = super::now_ms() + lease_ms;
    lease.unfinished = open["unfinished"].as_bool().unwrap_or(false);
    write_lease(&d, &lease)?;
    Ok(json!({ "type": "lease", "takenOver": null, "needBundle": need_bundle, "expires": lease.expires }))
}

/// Continue every expired, unfinished mirror as a task. Returns the new task ids.
pub fn scan(node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(book) = node.book() else {
        return out;
    };
    let now = super::now_ms();
    for p in &book.peers {
        let base = node.dir.join("standby").join(&p.name);
        for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let d = e.path();
            let Some(mut lease) = read_lease(&d) else { continue };
            if lease.taken_over.is_some() || lease.expires > now || !lease.unfinished {
                continue;
            }
            match takeover(node, p, &d, &lease) {
                Ok(id) => {
                    lease.taken_over = Some(id.clone());
                    let _ = write_lease(&d, &lease);
                    out.push(id);
                }
                Err(err) => {
                    super::audit("handoff-failed", &p.name, json!({ "session": lease.session, "error": err }));
                    // Do not retry every scan: mark it as not continuable.
                    lease.unfinished = false;
                    let _ = write_lease(&d, &lease);
                }
            }
        }
    }
    out
}

fn takeover(node: &Node, peer: &PeerEntry, d: &Path, lease: &Lease) -> Result<String, String> {
    let bundle: Bundle = serde_json::from_slice(
        &std::fs::read(d.join("bundle.json")).map_err(|_| "no project snapshot was mirrored")?,
    )
    .map_err(|e| e.to_string())?;
    let transcript = std::fs::read(d.join("transcript.jsonl")).map_err(|e| e.to_string())?;
    let store = node.store(&peer.name);
    std::fs::create_dir_all(&store.dir).map_err(|e| e.to_string())?;
    let id = store.create("handoff", HANDOFF_PROMPT)?;
    let task_dir = store.task_dir(&id);
    let work = task_dir.join("work");
    snapshot::extract(&bundle, &work)?;
    std::fs::write(
        task_dir.join("snapshot.json"),
        serde_json::to_vec(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // The worker resumes the session whose id is the task id: the checkpoint goes there.
    std::fs::create_dir_all(&node.sessions).map_err(|e| e.to_string())?;
    std::fs::write(node.sessions.join(format!("{id}.jsonl")), transcript)
        .map_err(|e| e.to_string())?;
    let root = work
        .canonicalize()
        .map(|p| kor_tools::without_verbatim(&p))
        .map_err(|e| e.to_string())?;
    super::server::write_meta(
        &task_dir,
        &super::server::Meta {
            peer: peer.name.clone(),
            root,
            workspace: true,
            handoff: true,
        },
    )?;
    store.push(
        &id,
        &Command::Send {
            text: HANDOFF_PROMPT.to_owned(),
            task: None,
        },
    )?;
    (node.spawn)(&peer.name, &id)?;
    super::audit("handoff", &peer.name, json!({ "session": lease.session, "task": id }));
    Ok(id)
}

/// The listener's background scan.
pub async fn watch(node: Arc<Node>) {
    loop {
        tokio::time::sleep(SCAN).await;
        let n = node.clone();
        let _ = tokio::task::spawn_blocking(move || scan(&n)).await;
    }
}

/// A local session to mirror.
#[derive(Debug, Clone)]
pub struct Mirror {
    pub session: String,
    pub transcript: PathBuf,
    pub project: PathBuf,
    /// Digest of the snapshot the peer holds.
    pub sent_digest: Option<String>,
}

/// What a renewal learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewed {
    Held,
    /// The peer continued the session as this task.
    TakenOver(String),
}

/// One renewal (with the snapshot only when it changed or the peer lost it).
///
/// # Errors
/// When the transcript cannot be read or the peer cannot be reached.
pub async fn renew(node: &Node, peer: &PeerEntry, m: &mut Mirror) -> Result<Renewed, String> {
    let text = std::fs::read_to_string(&m.transcript).map_err(|e| e.to_string())?;
    let messages = kor_agent::session::load(&m.transcript).unwrap_or_default();
    let bundle = snapshot::collect(&m.project)?;
    let digest = snapshot::digest(&bundle);
    let send_bundle = m.sent_digest.as_deref() != Some(digest.as_str());
    let (mut link, _) = dial(node, peer).await?;
    link.send(json!({
        "type": "open", "channel": "standby", "session": m.session, "transcript": text,
        "unfinished": unfinished(&messages), "digest": digest,
        "bundle": if send_bundle { serde_json::to_value(&bundle).unwrap_or(Value::Null) } else { Value::Null },
        "leaseMs": u64::try_from(LEASE.as_millis()).unwrap_or(u64::MAX),
    }))?;
    let v = link.recv_within(Duration::from_secs(60)).await?;
    if v["type"] == "error" {
        return Err(v["message"].as_str().unwrap_or("peer error").to_owned());
    }
    if let Some(t) = v["takenOver"].as_str() {
        return Ok(Renewed::TakenOver(t.to_owned()));
    }
    m.sent_digest = if v["needBundle"].as_bool() == Some(true) && !send_bundle {
        None
    } else {
        Some(digest)
    };
    Ok(Renewed::Held)
}

/// Stop mirroring (the session ended here); `Some(task)` if the peer had already taken over.
///
/// # Errors
/// When the peer cannot be reached.
pub async fn release(node: &Node, peer: &PeerEntry, session: &str) -> Result<Option<String>, String> {
    let (mut link, _) = dial(node, peer).await?;
    link.send(json!({ "type": "open", "channel": "standby", "session": session, "release": true }))?;
    let v = link.recv_within(Duration::from_secs(30)).await?;
    Ok(v["takenOver"].as_str().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_means_waiting_for_the_model_or_tools() {
        let user = Message::text(Role::User, "fix it");
        let done = Message::text(Role::Assistant, "done");
        let calling = Message {
            role: Role::Assistant,
            parts: vec![ContentPart::ToolUse {
                id: "1".into(),
                name: "Bash".into(),
                input: json!({}),
            }],
        };
        assert!(!unfinished(&[]));
        assert!(unfinished(std::slice::from_ref(&user)));
        assert!(!unfinished(&[user.clone(), done]));
        assert!(unfinished(&[user, calling]));
    }
}
