//! SSH remote mode (spec §15.3, ADR 0044): the agent, index, graph and sandbox run on the remote
//! machine; the local CLI or desktop is only the UI.
//!
//! - **Install** ([`ensure`]): `uname` picks the release target; a missing or different
//!   `~/.kor/bin/kor` is replaced by a verified binary (this machine's own `kor` when the target
//!   matches, else a minisign-verified release file from the local cache or GitHub), streamed over
//!   SSH, checked by SHA-256 on the remote, then renamed into place. Nothing unverified runs.
//! - **Transport**: the user's own `ssh` (keys/agent, known hosts); `kor remote-serve` speaks
//!   newline-delimited JSON on the channel's stdio with the desktop's event schema
//!   ([`kor_agent::remote::wire`]).
//! - **Tasks survive disconnects**: each task is a detached worker process writing
//!   `<kor home>/remote/tasks/<id>/events.jsonl` and reading commands from `inbox.jsonl`;
//!   `remote-serve` only relays. Reattach replays the log (`kor --remote … attach <task>`). The
//!   task id is also the agent session id, so the transcript is checkpointed after every turn.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::remote::wire::{self, Command, MAX_FRAME, PROTOCOL};
use kor_agent::remote::{Shell, SshHost};
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest, Review, StopReason};
use kor_providers::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// The remote binary, for the remote POSIX shell.
pub const REMOTE_BIN: &str = "\"$HOME/.kor/bin/kor\"";
const MAX_BINARY: usize = 256 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(100);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn sha256_hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(b).iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

// ---------------------------------------------------------------------------------------------
// Install / version check
// ---------------------------------------------------------------------------------------------

/// What `uname` and the installed binary say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteInfo {
    pub os: String,
    pub arch: String,
    pub target: Option<&'static str>,
    /// Version of `~/.kor/bin/kor`, if installed.
    pub version: Option<String>,
}

/// Release target for `uname -s` / `uname -m` (Windows servers are not supported: no POSIX sh).
pub fn remote_target(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os.trim(), arch.trim()) {
        ("Linux", "x86_64" | "amd64") => "x86_64-unknown-linux-musl",
        ("Linux", "aarch64" | "arm64") => "aarch64-unknown-linux-gnu",
        ("Darwin", "x86_64") => "x86_64-apple-darwin",
        ("Darwin", "arm64" | "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    })
}

/// `uname` and the installed version.
///
/// # Errors
/// When SSH fails.
pub async fn probe(sh: &dyn Shell) -> Result<RemoteInfo, String> {
    let cmd = format!("uname -s; uname -m; {REMOTE_BIN} --version 2>/dev/null || true");
    let out = sh.run(&cmd, None, Duration::from_secs(30)).await?;
    let text = out.text();
    let mut lines = text.lines();
    let os = lines.next().unwrap_or_default().trim().to_owned();
    let arch = lines.next().unwrap_or_default().trim().to_owned();
    let version = lines
        .next()
        .and_then(|l| l.split_whitespace().last())
        .map(str::to_owned);
    if os.is_empty() {
        return Err(format!("uname gave no answer: {}", out.error()));
    }
    Ok(RemoteInfo {
        target: remote_target(&os, &arch),
        os,
        arch,
        version,
    })
}

/// Where a verified `kor` binary for a target comes from.
#[derive(Debug, Clone)]
pub struct BinarySource {
    /// Version the remote must run (the local CLI's).
    pub version: String,
    /// A local `kor` of exactly `version` and its target (this machine's own binary).
    pub local: Option<(String, PathBuf)>,
    /// `<cache>/<version>/kor-<target>` + `.minisig`.
    pub cache: Option<PathBuf>,
    /// Minisign public key (base64); `None`: only `local` can be used.
    pub key: Option<String>,
    pub allow_download: bool,
}

impl BinarySource {
    /// The running CLI, Kor's cache, the built-in release key; downloads unless privacy mode.
    pub fn standard(version: &str, local_exe: Option<PathBuf>) -> Self {
        Self {
            version: version.to_owned(),
            local: crate::update::target()
                .zip(local_exe)
                .map(|(t, p)| (t.to_owned(), p)),
            cache: crate::kor_home().map(|h| h.join("remote").join("cache")),
            key: crate::update::PUBLIC_KEY.map(str::to_owned),
            allow_download: !crate::privacy_mode(),
        }
    }

    fn verify(&self, data: &[u8], sig: &str) -> Result<(), String> {
        let key = self
            .key
            .as_deref()
            .ok_or("this Kor build has no release key; it cannot verify a binary for the server")?;
        crate::update::verify_with(key, data, sig)
    }

    /// Verified bytes of `kor` for `target`.
    ///
    /// # Errors
    /// When no source has a verified binary.
    pub async fn binary(&self, target: &str) -> Result<Vec<u8>, String> {
        if let Some((t, p)) = &self.local
            && t == target
        {
            return std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
        }
        let name = format!("kor-{target}");
        let cached = self
            .cache
            .as_ref()
            .map(|c| c.join(&self.version).join(&name));
        if let Some(bin) = &cached
            && bin.is_file()
        {
            let data = std::fs::read(bin).map_err(|e| e.to_string())?;
            let sig = std::fs::read_to_string(format!("{}.minisig", bin.display()))
                .map_err(|_| format!("missing signature next to {}", bin.display()))?;
            self.verify(&data, &sig)?;
            return Ok(data);
        }
        if !self.allow_download {
            return Err(format!(
                "no verified Kor {} for {target}: put {name} and {name}.minisig into {}",
                self.version,
                cached
                    .as_ref()
                    .and_then(|c| c.parent())
                    .map_or_else(|| "the cache".into(), |p| p.display().to_string())
            ));
        }
        let base = format!(
            "https://github.com/{}/releases/download/v{}/{name}",
            crate::update::repo(),
            self.version
        );
        let data = download(&base, MAX_BINARY).await?;
        let sig = String::from_utf8(download(&format!("{base}.minisig"), 64 * 1024).await?)
            .map_err(|_| "signature is not text".to_owned())?;
        self.verify(&data, &sig)?;
        if let Some(bin) = &cached
            && let Some(dir) = bin.parent()
            && std::fs::create_dir_all(dir).is_ok()
        {
            let _ = std::fs::write(bin, &data);
            let _ = std::fs::write(format!("{}.minisig", bin.display()), &sig);
        }
        Ok(data)
    }
}

async fn download(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if !res.status().is_success() {
        return Err(format!("{url}: HTTP {}", res.status().as_u16()));
    }
    let b = res.bytes().await.map_err(|e| e.to_string())?;
    if b.len() > max {
        return Err("download too large".into());
    }
    Ok(b.to_vec())
}

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ensured {
    pub info: RemoteInfo,
    /// The version that was replaced (`Some("")` when nothing was installed before);
    /// `None` when the remote already matched.
    pub replaced: Option<String>,
}

/// Install or update `~/.kor/bin/kor` on the remote so it runs `src.version`.
///
/// # Errors
/// Unsupported OS/arch, no verified binary, a hash mismatch after upload, SSH failure.
pub async fn ensure(sh: &dyn Shell, src: &BinarySource) -> Result<Ensured, String> {
    let info = probe(sh).await?;
    if info.version.as_deref() == Some(src.version.as_str()) {
        return Ok(Ensured {
            info,
            replaced: None,
        });
    }
    let target = info
        .target
        .ok_or_else(|| format!("no Kor build for {} {}", info.os, info.arch))?;
    let data = src.binary(target).await?;
    let want = sha256_hex(&data);
    let up = r#"d="$HOME/.kor/bin"; mkdir -p "$d" && cat > "$d/kor.new" && chmod 755 "$d/kor.new" && { sha256sum "$d/kor.new" 2>/dev/null || shasum -a 256 "$d/kor.new"; }"#;
    let out = sh.run(up, Some(&data), Duration::from_secs(1800)).await?;
    let got = out
        .text()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    if !out.ok() || got != want {
        let _ = sh
            .run(
                r#"rm -f "$HOME/.kor/bin/kor.new""#,
                None,
                Duration::from_secs(30),
            )
            .await;
        return Err(format!(
            "upload check failed (sha256 {got} != {want}) {}",
            out.error()
        ));
    }
    let mv = sh
        .run(
            r#"mv -f "$HOME/.kor/bin/kor.new" "$HOME/.kor/bin/kor""#,
            None,
            Duration::from_secs(30),
        )
        .await?;
    if !mv.ok() {
        return Err(mv.error());
    }
    let after = probe(sh).await?;
    if after.version.as_deref() != Some(src.version.as_str()) {
        return Err(format!(
            "the installed kor reports {:?}, expected {}",
            after.version, src.version
        ));
    }
    Ok(Ensured {
        replaced: Some(info.version.unwrap_or_default()),
        info: after,
    })
}

// ---------------------------------------------------------------------------------------------
// Task store (remote side)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskState {
    pub id: String,
    /// `queued`, `running`, `waiting` (for an approval), `idle`, `failed`.
    pub state: String,
    pub project: String,
    pub prompt: String,
    pub started: u64,
    pub updated: u64,
    /// Inbox lines handled.
    pub consumed: usize,
    /// Events written.
    pub events: u64,
}

/// `<dir>/<id>/{state.json, inbox.jsonl, events.jsonl, lock, worker.log}`.
#[derive(Debug, Clone)]
pub struct TaskStore {
    pub dir: PathBuf,
}

impl TaskStore {
    pub fn standard() -> Option<Self> {
        crate::kor_home().map(|h| Self {
            dir: h.join("remote").join("tasks"),
        })
    }

    pub fn task_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    pub fn exists(&self, id: &str) -> bool {
        wire::valid_task_id(id) && self.task_dir(id).join("state.json").is_file()
    }

    pub fn state(&self, id: &str) -> Option<TaskState> {
        let b = std::fs::read(self.task_dir(id).join("state.json")).ok()?;
        serde_json::from_slice(&b).ok()
    }

    /// Atomic write.
    ///
    /// # Errors
    /// On I/O errors.
    pub fn save_state(&self, s: &TaskState) -> Result<(), String> {
        let dir = self.task_dir(&s.id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join("state.json.tmp");
        let body = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("state.json")).map_err(|e| e.to_string())
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut TaskState)) {
        if let Some(mut s) = self.state(id) {
            f(&mut s);
            s.updated = now_ms();
            let _ = self.save_state(&s);
        }
    }

    /// Newest first.
    pub fn list(&self) -> Vec<TaskState> {
        let mut v: Vec<TaskState> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| self.state(&e.file_name().to_string_lossy()))
            .collect();
        v.sort_by(|a, b| b.started.cmp(&a.started));
        v
    }

    /// A new task for `text` in `project`.
    ///
    /// # Errors
    /// On I/O errors.
    pub fn create(&self, project: &str, text: &str) -> Result<String, String> {
        let now = now_ms();
        let id = format!("r-{now}-{:04x}", std::process::id() & 0xffff);
        self.save_state(&TaskState {
            id: id.clone(),
            state: "queued".into(),
            project: project.to_owned(),
            prompt: text.chars().take(200).collect(),
            started: now,
            updated: now,
            ..TaskState::default()
        })?;
        Ok(id)
    }

    /// Append a command for the worker.
    ///
    /// # Errors
    /// On I/O errors.
    pub fn push(&self, id: &str, c: &Command) -> Result<(), String> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.task_dir(id).join("inbox.jsonl"))
            .map_err(|e| e.to_string())?;
        f.write_all(c.frame().as_bytes()).map_err(|e| e.to_string())
    }

    fn inbox(&self, id: &str) -> Vec<Command> {
        std::fs::read_to_string(self.task_dir(id).join("inbox.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or(Command::Detach))
            .collect()
    }

    fn events_path(&self, id: &str) -> PathBuf {
        self.task_dir(id).join("events.jsonl")
    }

    /// `confirm` events of task `id` not yet answered (`confirmed`), oldest first — the approval
    /// queue (`kor approve`, ADR 0051) answers them with [`Command::Approve`] through [`Self::push`].
    pub fn pending_confirms(&self, id: &str) -> Vec<Value> {
        if !wire::valid_task_id(id) {
            return Vec::new();
        }
        let text = std::fs::read_to_string(self.events_path(id)).unwrap_or_default();
        let mut open: Vec<Value> = Vec::new();
        for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            match v["type"].as_str() {
                Some("confirm") => open.push(v),
                Some("confirmed") => open.retain(|c| c["id"] != v["id"]),
                _ => {}
            }
        }
        open
    }
}

/// Events file of one task, written by its worker only.
struct EventLog {
    store: TaskStore,
    id: String,
    seq: u64,
}

impl EventLog {
    fn new(store: TaskStore, id: &str) -> Self {
        let seq = store.state(id).map_or(0, |s| s.events);
        Self {
            store,
            id: id.to_owned(),
            seq,
        }
    }

    fn emit(&mut self, mut body: Value) {
        body["task"] = Value::String(self.id.clone());
        body["seq"] = json!(self.seq);
        body["at"] = json!(now_ms());
        self.seq += 1;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.store.events_path(&self.id))
        {
            let _ = writeln!(f, "{body}");
        }
        let seq = self.seq;
        self.store.update(&self.id, |s| s.events = seq);
    }
}

// ---------------------------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------------------------

/// One agent turn. Production: [`AgentTurns`]; tests script it.
#[async_trait]
pub trait TurnRunner: Send {
    /// `Ok(Some(note))` shows a notice (stopped at the turn limit, ...).
    async fn turn(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<Option<String>, String>;
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Review>>>>;
type Events = Arc<Mutex<mpsc::UnboundedReceiver<AgentEvent>>>;

/// Write every queued agent event to the log; `true` once the turn's sender is gone.
/// The pump and the approver both drain through here, so a permission card never overtakes
/// the text sent before it.
fn drain(rx: &Events, log: &Mutex<EventLog>) -> bool {
    let mut rx = lock(rx);
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if let Some(body) = wire::event_json(ev) {
                    lock(log).emit(body);
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => return false,
            Err(mpsc::error::TryRecvError::Disconnected) => return true,
        }
    }
}

struct InboxApprover {
    log: Arc<Mutex<EventLog>>,
    pending: Pending,
    cancel: CancellationToken,
    events: Events,
}

#[async_trait]
impl Approver for InboxApprover {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.review(req).await.reply
    }

    async fn review(&self, req: &PermissionRequest) -> Review {
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(req.id.clone(), tx);
        drain(&self.events, &self.log);
        {
            let mut log = lock(&self.log);
            log.emit(
                json!({ "type": "tool", "id": req.id, "name": req.tool, "summary": req.summary }),
            );
            log.emit(wire::confirm_json(req));
            let id = log.id.clone();
            log.store.update(&id, |s| s.state = "waiting".into());
        }
        // Cancelled or the worker stopping: the safe answer is no.
        let review = tokio::select! {
            r = rx => r.unwrap_or_else(|_| PermissionReply::Deny.into()),
            () = self.cancel.cancelled() => PermissionReply::Deny.into(),
        };
        let mut log = lock(&self.log);
        let reply = match review.reply {
            PermissionReply::Allow => "yes",
            PermissionReply::AllowAlways => "always",
            PermissionReply::Deny => "no",
        };
        log.emit(json!({ "type": "confirmed", "id": req.id, "approval": reply }));
        let id = log.id.clone();
        log.store.update(&id, |s| s.state = "running".into());
        review
    }
}

/// Run every queued turn of task `id`, then exit (the next `send` starts a new worker).
/// Holds `<task>/lock` so two workers never run one task.
///
/// # Errors
/// When the task does not exist or its lock cannot be taken.
#[allow(clippy::too_many_lines)]
pub async fn run_worker(
    store: &TaskStore,
    id: &str,
    runner: &mut dyn TurnRunner,
) -> Result<(), String> {
    if !store.exists(id) {
        return Err(format!("no task {id}"));
    }
    let lock_path = store.task_dir(id).join("lock");
    let _guard = tokio::task::spawn_blocking(move || kor_platform::lock_file(&lock_path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let log = Arc::new(Mutex::new(EventLog::new(store.clone(), id)));
    let mut next = store.state(id).map_or(0, |s| s.consumed);
    let mut queue: VecDeque<String> = VecDeque::new();
    loop {
        let inbox = store.inbox(id);
        while next < inbox.len() {
            if let Command::Send { text, .. } = &inbox[next] {
                queue.push_back(text.clone());
            }
            next += 1;
        }
        let Some(text) = queue.pop_front() else {
            store.update(id, |s| {
                s.state = "idle".into();
                s.consumed = next;
            });
            // A `send` that arrived while we were finishing: the relay saw `running` and did not
            // start a worker, so this one takes it.
            if store.inbox(id).len() > next {
                continue;
            }
            return Ok(());
        };
        store.update(id, |s| {
            s.state = "running".into();
            s.consumed = next;
        });
        lock(&log).emit(json!({ "type": "turnStart" }));
        let cancel = CancellationToken::new();
        let pending: Pending = Arc::default();
        // Approvals, aborts and follow-ups arriving during the turn.
        let watcher = {
            let (store, id, cancel, pending) = (
                store.clone(),
                id.to_owned(),
                cancel.clone(),
                pending.clone(),
            );
            let start = next;
            // Runs until the turn ends (aborted below); sends are queued after the turn.
            tokio::spawn(async move {
                let mut seen = start;
                loop {
                    let inbox = store.inbox(&id);
                    while seen < inbox.len() {
                        match &inbox[seen] {
                            Command::Approve {
                                id,
                                approval,
                                files,
                            } => {
                                if let Ok(r) = wire::parse_review(approval, files.as_ref())
                                    && let Some(tx) = lock(&pending).remove(id)
                                {
                                    let _ = tx.send(r);
                                }
                            }
                            Command::Abort => cancel.cancel(),
                            _ => {}
                        }
                        seen += 1;
                    }
                    tokio::time::sleep(POLL).await;
                }
            })
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let rx: Events = Arc::new(Mutex::new(rx));
        let approver = InboxApprover {
            log: log.clone(),
            pending: pending.clone(),
            cancel: cancel.clone(),
            events: rx.clone(),
        };
        let (pump_rx, pump_log) = (rx.clone(), log.clone());
        let pump = tokio::spawn(async move {
            while !drain(&pump_rx, &pump_log) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let result = runner.turn(text, &tx, &approver, cancel.clone()).await;
        drop(tx);
        let _ = pump.await;
        watcher.abort();
        let _ = watcher.await;
        // The watcher's progress is re-read from the inbox: lines it handled are approvals or
        // aborts of this turn; sends are queued again below.
        let inbox = store.inbox(id);
        while next < inbox.len() {
            if let Command::Send { text, .. } = &inbox[next] {
                queue.push_back(text.clone());
            }
            next += 1;
        }
        let mut l = lock(&log);
        match result {
            Ok(Some(note)) => l.emit(json!({ "type": "notice", "text": note })),
            Ok(None) => {}
            Err(e) => l.emit(json!({ "type": "error", "message": e })),
        }
        l.emit(json!({ "type": "done" }));
        drop(l);
        store.update(id, |s| s.consumed = next);
    }
}

/// The real agent: `prepare` once (resuming the task's transcript), save after every turn.
pub struct AgentTurns {
    pub l: kor_i18n::Localizer,
    pub loaded: kor_config::Loaded,
    pub root: PathBuf,
    pub id: String,
    prepared: Option<crate::Prepared>,
}

impl std::fmt::Debug for AgentTurns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTurns")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("hello", &self.hello)
            .finish_non_exhaustive()
    }
}

impl AgentTurns {
    pub fn new(
        l: kor_i18n::Localizer,
        loaded: kor_config::Loaded,
        root: PathBuf,
        id: &str,
    ) -> Self {
        Self {
            l,
            loaded,
            root,
            id: id.to_owned(),
            prepared: None,
        }
    }
}

#[async_trait]
impl TurnRunner for AgentTurns {
    async fn turn(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<Option<String>, String> {
        if self.prepared.is_none() {
            let exists = crate::sessions_dir()
                .is_some_and(|d| d.join(format!("{}.jsonl", self.id)).is_file());
            let opts = crate::Options {
                root: self.root.clone(),
                model: None,
                mode: None,
                resume: exists.then(|| crate::Resume::Id(self.id.clone())),
                new_id: (!exists).then(|| self.id.clone()),
                surface: kor_agent::prompt::Surface::Terminal,
            };
            let mut notes = Vec::new();
            let prepared = crate::prepare(&self.l, &self.loaded, &opts, &mut notes);
            for n in notes {
                let _ = events.send(AgentEvent::Notice(n));
            }
            let mut p = prepared?;
            let mcp = crate::attach_mcp(&self.l, &mut p.agent, &p.compat).await;
            for n in mcp.notes {
                let _ = events.send(AgentEvent::Notice(n));
            }
            self.prepared = Some(p);
        }
        let p = self.prepared.as_mut().ok_or("agent not ready")?;
        crate::refresh_memory(&mut p.agent);
        let r = p.agent.send(text, events, approver, cancel).await;
        if let Some(path) = &p.path {
            kor_agent::session::save(path, &p.agent.messages).map_err(|e| e.to_string())?;
        }
        let max = p.agent.max_turns;
        let note = |k: &str| Some(self.l.t_args(k, &kor_i18n::args!("n" => max)));
        match r {
            Ok(StopReason::Budget | StopReason::MaxTurns) => Ok(note("stop-max-turns")),
            Ok(StopReason::Blocked(reason)) => Err(self
                .l
                .t_args("prompt-blocked", &kor_i18n::args!("reason" => reason))),
            Ok(_) => Ok(None),
            Err(e) => Err(crate::provider_error(&self.l, &e)),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Relay (`kor remote-serve` on the SSH channel)
// ---------------------------------------------------------------------------------------------

/// Starts the worker of a task (a detached `kor remote-serve --worker <id>` in production).
pub type Spawn<'a> = dyn Fn(&str) -> Result<(), String> + Send + Sync + 'a;

/// Detached worker process: `<exe> remote-serve --worker <id> --project <dir>`, output to
/// `<task>/worker.log`.
///
/// # Errors
/// If the process cannot start.
pub fn spawn_worker(store: &TaskStore, id: &str, project: &Path) -> Result<(), String> {
    use kor_platform::Detach as _;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log =
        std::fs::File::create(store.task_dir(id).join("worker.log")).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(["remote-serve", "--worker", id, "--project"])
        .arg(project)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        .detach()
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the task worker: {e}"))
}

struct Follow {
    task: String,
    from: u64,
    offset: u64,
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, v: &Value) -> Result<(), String> {
    let mut s = v.to_string();
    s.push('\n');
    w.write_all(s.as_bytes())
        .await
        .map_err(|e| format!("{GONE}{e}"))?;
    w.flush().await.map_err(|e| format!("{GONE}{e}"))
}

/// Prefix of write errors: the client went away (a normal end, the task keeps running).
const GONE: &str = "client gone: ";

/// New lines of the followed task's event log (sequence ≥ `from`).
fn read_new(store: &TaskStore, f: &mut Follow) -> Vec<Value> {
    let Ok(mut file) = std::fs::File::open(store.events_path(&f.task)) else {
        return Vec::new();
    };
    if file.seek(std::io::SeekFrom::Start(f.offset)).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut r = std::io::BufReader::new(file);
    let mut line = String::new();
    while r.read_line(&mut line).unwrap_or(0) > 0 {
        if !line.ends_with('\n') {
            break; // half-written: next time
        }
        f.offset += line.len() as u64;
        if let Ok(v) = serde_json::from_str::<Value>(&line)
            && v["seq"].as_u64().unwrap_or(0) >= f.from
        {
            out.push(v);
        }
        line.clear();
    }
    out
}

/// Serve one client on `input`/`output` until it disconnects. Tasks keep running afterwards.
///
/// # Errors
/// When the client cannot be written to.
pub async fn serve<R, W>(
    input: R,
    output: W,
    store: &TaskStore,
    project: &Path,
    spawn: &Spawn<'_>,
) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    match serve_inner(input, output, store, project, spawn).await {
        Err(e) if e.starts_with(GONE) => Ok(()),
        r => r,
    }
}

async fn serve_inner<R, W>(
    input: R,
    mut output: W,
    store: &TaskStore,
    project: &Path,
    spawn: &Spawn<'_>,
) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    std::fs::create_dir_all(&store.dir).map_err(|e| e.to_string())?;
    let project_s = project.to_string_lossy().into_owned();
    write_frame(
        &mut output,
        &json!({ "type": "hello", "protocol": PROTOCOL, "version": kor_core::VERSION,
                 "project": project_s, "tasks": store.list() }),
    )
    .await?;
    let mut lines = input.lines();
    let mut follow: Option<Follow> = None;
    let mut tick = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return Ok(()) };
                if line.len() > MAX_FRAME {
                    write_frame(&mut output, &json!({"type": "error", "message": "frame too large"})).await?;
                    continue;
                }
                let cmd = match serde_json::from_str::<Command>(&line) {
                    Ok(c) => c,
                    Err(e) => {
                        write_frame(&mut output, &json!({"type": "error", "message": format!("bad command: {e}")})).await?;
                        continue;
                    }
                };
                if let Err(e) = handle(cmd, store, &project_s, spawn, &mut follow, &mut output).await {
                    write_frame(&mut output, &json!({"type": "error", "message": e})).await?;
                }
            }
            _ = tick.tick() => {
                if let Some(f) = follow.as_mut() {
                    for v in read_new(store, f) {
                        write_frame(&mut output, &v).await?;
                    }
                }
            }
        }
    }
}

async fn handle<W: AsyncWrite + Unpin>(
    cmd: Command,
    store: &TaskStore,
    project: &str,
    spawn: &Spawn<'_>,
    follow: &mut Option<Follow>,
    output: &mut W,
) -> Result<(), String> {
    let attached = |follow: &Option<Follow>| {
        follow
            .as_ref()
            .map(|f| f.task.clone())
            .ok_or_else(|| "no task attached".to_owned())
    };
    match cmd {
        Command::Send { text, task } => {
            if text.trim().is_empty() {
                return Err("empty message".into());
            }
            let id = match task {
                Some(t) if store.exists(&t) => t,
                Some(t) => return Err(format!("no task {t}")),
                None => store.create(project, &text)?,
            };
            let from = store.state(&id).map_or(0, |s| s.events);
            store.push(&id, &Command::Send { text, task: None })?;
            // After the append: a worker that is finishing re-reads the inbox (see run_worker).
            let busy = store
                .state(&id)
                .is_some_and(|s| s.state == "running" || s.state == "waiting");
            if !busy {
                spawn(&id)?;
            }
            *follow = Some(Follow {
                task: id.clone(),
                from,
                offset: 0,
            });
            write_frame(
                output,
                &json!({"type": "attached", "task": id, "from": from}),
            )
            .await
        }
        Command::Attach { task, from } => {
            if !store.exists(&task) {
                return Err(format!("no task {task}"));
            }
            write_frame(
                output,
                &json!({"type": "attached", "task": task, "from": from}),
            )
            .await?;
            *follow = Some(Follow {
                task,
                from,
                offset: 0,
            });
            Ok(())
        }
        c @ (Command::Approve { .. } | Command::Abort) => store.push(&attached(follow)?, &c),
        Command::Tasks => {
            write_frame(output, &json!({"type": "tasks", "tasks": store.list()})).await
        }
        Command::Detach => {
            *follow = None;
            write_frame(output, &json!({"type": "detached"})).await
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Client (local side)
// ---------------------------------------------------------------------------------------------

/// A connection to `kor remote-serve`.
pub struct Client {
    _child: Option<tokio::process::Child>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    lines: tokio::io::Lines<BufReader<Box<dyn AsyncRead + Unpin + Send>>>,
    pub hello: Value,
}

impl Client {
    /// Over any byte stream (tests use in-memory pipes). Reads and checks `hello`.
    ///
    /// # Errors
    /// No hello within 60 s, or another protocol version.
    pub async fn from_io(
        reader: Box<dyn AsyncRead + Unpin + Send>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<Self, String> {
        let mut c = Self {
            _child: None,
            writer,
            lines: BufReader::new(reader).lines(),
            hello: Value::Null,
        };
        let hello = tokio::time::timeout(Duration::from_secs(60), c.next())
            .await
            .map_err(|_| "the remote Kor did not answer".to_owned())?
            .ok_or("the remote Kor closed the connection")?;
        if hello["type"] != "hello" || hello["protocol"].as_u64() != Some(u64::from(PROTOCOL)) {
            return Err(format!("unexpected remote Kor greeting: {hello}"));
        }
        c.hello = hello;
        Ok(c)
    }

    /// `ssh … "$HOME/.kor/bin/kor" remote-serve [--project <dir>]`.
    ///
    /// # Errors
    /// When ssh cannot start or the remote does not greet.
    pub async fn connect(h: &SshHost, project: Option<&str>) -> Result<Self, String> {
        let mut remote = format!("{REMOTE_BIN} remote-serve");
        if let Some(p) = project {
            if p.contains(['\n', '\0']) {
                return Err("invalid project path".into());
            }
            remote.push_str(" --project ");
            remote.push_str(&kor_agent::remote::sh_quote(p));
        }
        let mut child = kor_agent::remote::ssh_command(h, &remote)
            .spawn()
            .map_err(|e| format!("could not start ssh ({e}); install the OpenSSH client"))?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let inp = child.stdin.take().ok_or("no stdin")?;
        let mut err = child.stderr.take();
        match Self::from_io(Box::new(out), Box::new(inp)).await {
            Ok(c) => Ok(Self {
                _child: Some(child),
                ..c
            }),
            Err(e) => {
                let mut text = String::new();
                if let Some(s) = err.as_mut() {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        tokio::io::AsyncReadExt::read_to_string(s, &mut text),
                    )
                    .await;
                }
                Err(if text.trim().is_empty() {
                    e
                } else {
                    format!("{e}: {}", text.trim())
                })
            }
        }
    }

    /// # Errors
    /// When the connection is gone.
    pub async fn send(&mut self, c: &Command) -> Result<(), String> {
        self.writer
            .write_all(c.frame().as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        self.writer.flush().await.map_err(|e| e.to_string())
    }

    /// Next frame; `None` when the connection closed.
    pub async fn next(&mut self) -> Option<Value> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if line.len() > MAX_FRAME {
                continue;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                return Some(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_agent::remote::Output;

    // ----- install with a fake ssh that keeps the "remote" in a temp dir -----

    struct FakeRemote {
        dir: PathBuf,
        os: &'static str,
        arch: &'static str,
        corrupt: bool,
    }

    #[async_trait]
    impl Shell for FakeRemote {
        async fn run(
            &self,
            cmd: &str,
            stdin: Option<&[u8]>,
            _: Duration,
        ) -> Result<Output, String> {
            let bin = self.dir.join("kor");
            let new = self.dir.join("kor.new");
            let ok = |s: String| Output {
                code: Some(0),
                stdout: s.into_bytes(),
                stderr: Vec::new(),
            };
            if cmd.starts_with("uname") {
                // The "binary" is a text file holding its version.
                let v = std::fs::read_to_string(&bin)
                    .map(|v| format!("kor {v}\n"))
                    .unwrap_or_default();
                return Ok(ok(format!("{}\n{}\n{v}", self.os, self.arch)));
            }
            if cmd.contains("cat > ") {
                let mut data = stdin.unwrap_or_default().to_vec();
                std::fs::write(&new, &data).map_err(|e| e.to_string())?;
                if self.corrupt {
                    data.push(b'!');
                }
                return Ok(ok(format!("{}  {}\n", sha256_hex(&data), new.display())));
            }
            if cmd.starts_with("mv -f") {
                std::fs::rename(&new, &bin).map_err(|e| e.to_string())?;
                return Ok(ok(String::new()));
            }
            if cmd.starts_with("rm -f") {
                let _ = std::fs::remove_file(&new);
                return Ok(ok(String::new()));
            }
            Err(format!("unexpected command {cmd}"))
        }
    }

    fn signed_cache(dir: &Path, version: &str, target: &str, data: &[u8]) -> String {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("key");
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, data, None, None)
            .expect("sign")
            .to_string();
        let d = dir.join(version);
        std::fs::create_dir_all(&d).expect("mkdir");
        std::fs::write(d.join(format!("kor-{target}")), data).expect("bin");
        std::fs::write(d.join(format!("kor-{target}.minisig")), sig).expect("sig");
        kp.pk.to_base64()
    }

    fn source(cache: &Path, key: Option<String>, version: &str) -> BinarySource {
        BinarySource {
            version: version.into(),
            local: None,
            cache: Some(cache.to_path_buf()),
            key,
            allow_download: false,
        }
    }

    #[tokio::test]
    async fn install_update_and_up_to_date() {
        let t = tempfile::tempdir().expect("tmp");
        let remote = FakeRemote {
            dir: t.path().join("remote"),
            os: "Linux",
            arch: "x86_64",
            corrupt: false,
        };
        std::fs::create_dir_all(&remote.dir).expect("mkdir");
        let cache = t.path().join("cache");
        let target = "x86_64-unknown-linux-musl";
        let key = signed_cache(&cache, "1.2.0", target, b"1.2.0");
        // First connection: nothing installed.
        let e = ensure(&remote, &source(&cache, Some(key.clone()), "1.2.0"))
            .await
            .expect("install");
        assert_eq!(e.replaced.as_deref(), Some(""));
        assert_eq!(e.info.version.as_deref(), Some("1.2.0"));
        // Same version: nothing to do.
        let e = ensure(&remote, &source(&cache, Some(key), "1.2.0"))
            .await
            .expect("check");
        assert_eq!(e.replaced, None);
        // Version mismatch: the local CLI is newer, the remote is updated.
        let key2 = signed_cache(&cache, "1.3.0", target, b"1.3.0");
        let e = ensure(&remote, &source(&cache, Some(key2), "1.3.0"))
            .await
            .expect("update");
        assert_eq!(e.replaced.as_deref(), Some("1.2.0"));
        assert_eq!(
            std::fs::read_to_string(remote.dir.join("kor")).expect("bin"),
            "1.3.0"
        );
    }

    #[tokio::test]
    async fn unverified_or_corrupt_binaries_never_run() {
        let t = tempfile::tempdir().expect("tmp");
        let mut remote = FakeRemote {
            dir: t.path().join("remote"),
            os: "Linux",
            arch: "aarch64",
            corrupt: false,
        };
        std::fs::create_dir_all(&remote.dir).expect("mkdir");
        let cache = t.path().join("cache");
        let target = "aarch64-unknown-linux-gnu";
        let _right = signed_cache(&cache, "2.0.0", target, b"2.0.0");
        let other = minisign::KeyPair::generate_unencrypted_keypair().expect("key");
        // Signed by another key.
        let err = ensure(
            &remote,
            &source(&cache, Some(other.pk.to_base64()), "2.0.0"),
        )
        .await
        .expect_err("bad signature");
        assert!(err.contains("signature"), "{err}");
        assert!(!remote.dir.join("kor.new").exists() && !remote.dir.join("kor").exists());
        // No key in this build.
        assert!(
            ensure(&remote, &source(&cache, None, "2.0.0"))
                .await
                .is_err()
        );
        // Nothing cached, downloads off.
        let err = ensure(&remote, &source(&cache, None, "9.9.9"))
            .await
            .expect_err("missing");
        assert!(
            err.contains("kor-aarch64-unknown-linux-gnu.minisig"),
            "{err}"
        );
        // Transport corruption: the staged file is removed, nothing is renamed into place.
        remote.corrupt = true;
        let key = signed_cache(&cache, "2.0.0", target, b"2.0.0");
        let err = ensure(&remote, &source(&cache, Some(key), "2.0.0"))
            .await
            .expect_err("corrupt");
        assert!(err.contains("sha256"), "{err}");
        assert!(!remote.dir.join("kor.new").exists() && !remote.dir.join("kor").exists());
        // Unsupported system.
        remote.os = "FreeBSD";
        assert!(
            ensure(&remote, &source(&cache, None, "2.0.0"))
                .await
                .is_err()
        );
        assert_eq!(
            remote_target("Darwin", "arm64"),
            Some("aarch64-apple-darwin")
        );
    }

    /// Real SSH: `KOR_TEST_SSH=user@host[:port] cargo test -p kor-runtime -- --ignored real_ssh`.
    /// Read-only: `uname`, the installed version and a full `kor discover` scan (no sudo).
    #[tokio::test]
    #[ignore = "needs a reachable SSH server (KOR_TEST_SSH)"]
    async fn real_ssh_probe_and_discover() {
        let target = std::env::var("KOR_TEST_SSH").expect("KOR_TEST_SSH");
        let h = kor_agent::remote::resolve_target(&target, None).expect("target");
        let sh = kor_agent::remote::SshShell::new(&h);
        let info = probe(&sh).await.expect("probe");
        assert!(!info.os.is_empty(), "{info:?}");
        let host = kor_graph_host(&h.host);
        let d = kor_agent::remote::discover::discover(&sh, &target, &host, 0, &|_| false)
            .await
            .expect("discover");
        assert!(!d.snapshot.kernel.is_empty());
    }

    fn kor_graph_host(h: &str) -> String {
        h.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    }

    // ----- protocol, disconnect and reattach, with in-memory pipes and in-process workers -----

    /// Says "working", asks to run `deploy.sh`, then says what the answer was.
    struct Scripted;

    #[async_trait]
    impl TurnRunner for Scripted {
        async fn turn(
            &mut self,
            text: String,
            events: &mpsc::UnboundedSender<AgentEvent>,
            approver: &dyn Approver,
            _cancel: CancellationToken,
        ) -> Result<Option<String>, String> {
            let _ = events.send(AgentEvent::Text(format!("working on {text}")));
            let req = PermissionRequest {
                id: format!("c-{text}"),
                tool: "bash".into(),
                summary: "./deploy.sh".into(),
                files: Vec::new(),
                destructive: None,
                rules: Vec::new(),
            };
            let r = approver.review(&req).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = events.send(AgentEvent::Text(format!("answer {:?}", r.reply)));
            Ok(None)
        }
    }

    fn connect_local(
        store: &TaskStore,
    ) -> (
        tokio::task::JoinHandle<Result<(), String>>,
        impl std::future::Future<Output = Client>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(1 << 16);
        let (sr, sw) = tokio::io::split(server_io);
        let s = store.clone();
        let server = tokio::spawn(async move {
            let s2 = s.clone();
            let spawn = move |id: &str| {
                let (s3, id) = (s2.clone(), id.to_owned());
                tokio::spawn(async move { run_worker(&s3, &id, &mut Scripted).await });
                Ok(())
            };
            serve(BufReader::new(sr), sw, &s, Path::new("/srv/app"), &spawn).await
        });
        let (cr, cw) = tokio::io::split(client_io);
        (server, async move {
            Client::from_io(Box::new(cr), Box::new(cw))
                .await
                .expect("hello")
        })
    }

    async fn until(c: &mut Client, ty: &str) -> Vec<Value> {
        let mut seen = Vec::new();
        loop {
            let v = tokio::time::timeout(Duration::from_secs(10), c.next())
                .await
                .expect("timely")
                .expect("open");
            let done = v["type"] == ty;
            seen.push(v);
            if done {
                return seen;
            }
        }
    }

    async fn wait_state(store: &TaskStore, id: &str, want: &str) {
        for _ in 0..200 {
            if store.state(id).is_some_and(|s| s.state == want) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("task {id} never reached {want}: {:?}", store.state(id));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn task_survives_disconnect_and_reattach_replays_it() {
        let t = tempfile::tempdir().expect("tmp");
        let store = TaskStore {
            dir: t.path().join("tasks"),
        };
        // Connect, start a task, see the permission card, then drop the connection.
        let (server, client) = connect_local(&store);
        let mut c = client.await;
        assert_eq!(c.hello["project"], "/srv/app");
        c.send(&Command::Send {
            text: "deploy".into(),
            task: None,
        })
        .await
        .expect("send");
        let first = until(&mut c, "confirm").await;
        let id = first[0]["task"].as_str().expect("attached task").to_owned();
        assert_eq!(first[0]["type"], "attached");
        assert!(
            first
                .iter()
                .any(|v| v["type"] == "text" && v["delta"] == "working on deploy")
        );
        drop(c);
        server
            .await
            .expect("join")
            .expect("serve ends on disconnect");
        // The task is still there, waiting for its answer.
        wait_state(&store, &id, "waiting").await;
        // Reattach: everything is replayed, then the answer goes through.
        let (server, client) = connect_local(&store);
        let mut c = client.await;
        assert_eq!(c.hello["tasks"][0]["id"], id.as_str());
        c.send(&Command::Attach {
            task: id.clone(),
            from: 0,
        })
        .await
        .expect("attach");
        let replay = until(&mut c, "confirm").await;
        assert!(replay.iter().any(|v| v["type"] == "turnStart"));
        let confirm = replay.last().expect("confirm");
        assert_eq!(confirm["summary"], "./deploy.sh");
        c.send(&Command::Approve {
            id: "c-deploy".into(),
            approval: "yes".into(),
            files: None,
        })
        .await
        .expect("approve");
        let rest = until(&mut c, "done").await;
        assert!(
            rest.iter()
                .any(|v| v["type"] == "confirmed" && v["approval"] == "yes")
        );
        assert!(rest.iter().any(|v| v["delta"] == "answer Allow"));
        wait_state(&store, &id, "idle").await;
        // A follow-up turn while disconnected: it runs to its approval with nobody attached.
        c.send(&Command::Send {
            text: "again".into(),
            task: Some(id.clone()),
        })
        .await
        .expect("send");
        let _ = until(&mut c, "attached").await;
        c.send(&Command::Detach).await.expect("detach");
        drop(c);
        server.await.expect("join").expect("serve");
        wait_state(&store, &id, "waiting").await;
        let (server, client) = connect_local(&store);
        let mut c = client.await;
        let from = store.state(&id).expect("state").events - 2;
        c.send(&Command::Attach {
            task: id.clone(),
            from,
        })
        .await
        .expect("attach");
        let tail = until(&mut c, "confirm").await;
        assert!(
            tail.iter()
                .all(|v| v["seq"].as_u64().is_none_or(|s| s >= from))
        );
        c.send(&Command::Abort).await.expect("abort");
        c.send(&Command::Approve {
            id: "c-again".into(),
            approval: "no".into(),
            files: None,
        })
        .await
        .expect("deny");
        let end = until(&mut c, "done").await;
        assert!(end.iter().any(|v| v["delta"] == "answer Deny"));
        c.send(&Command::Tasks).await.expect("tasks");
        let tasks = until(&mut c, "tasks").await;
        assert_eq!(
            tasks.last().expect("tasks")["tasks"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        // Bad frames get an error, not a crash.
        c.send(&Command::Attach {
            task: "nope".into(),
            from: 0,
        })
        .await
        .expect("send");
        let err = until(&mut c, "error").await;
        assert!(
            err.last().expect("err")["message"]
                .as_str()
                .is_some_and(|m| m.contains("no task"))
        );
        drop(c);
        server.await.expect("join").expect("serve");
    }
}
