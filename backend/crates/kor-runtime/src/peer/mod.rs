//! Kor-to-Kor bridge (spec §31.1, ADR 0057): Kor on two machines (home PC ↔ server ↔ laptop)
//! delegates tasks to each other and shares state.
//!
//! - **Identity**: each machine has one long-term X25519 key in the secret store (OS keychain or
//!   the vault file, account `peer:identity`, never in a file or the agent's environment).
//! - **Pairing** ([`pair`]): `kor peer pair` shows a one-time code (and QR); the other machine
//!   joins with it. A Noise `XXpsk3` handshake keyed by the code authenticates both ends; each side
//!   stores the other's public key in `<kor home>/peer/peers.toml`.
//! - **Sessions** ([`noise`]): Noise XX between paired keys only (unknown keys are refused), over
//!   a Tailscale address, a direct address or `ssh <host> kor peer serve --stdio` ([`client`]).
//! - **Delegation** ([`server`], [`worker`]): a task with a project snapshot runs on the peer as
//!   a detached worker (the remote-serve model of ADR 0044), progress and approvals stream back,
//!   the result returns as changed files and a diff ([`snapshot`]). The worker keeps running when
//!   the delegating laptop goes away; reattach any time.
//! - **Handoff** ([`standby`]): a local session mirrors its checkpoint to a peer under a lease;
//!   when the laptop closes (no renewal), the peer continues it from that checkpoint.
//! - **Memory** ([`merge`]): knowledge records sync both ways with a three-way merge.
//! - **Permissions** ([`policy`]): per peer, which directories it may use, whether it may write
//!   there, which commands and extra tools it may run; read-only by default. Both ends audit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_secrets::{ExposeSecret as _, SecretStore, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub mod client;
#[cfg(test)]
mod e2e;
pub mod merge;
pub mod noise;
pub mod pair;
pub mod policy;
pub mod server;
pub mod snapshot;
pub mod standby;
pub mod worker;

/// Protocol version of the peer channel; both sides refuse another one.
pub const PROTOCOL: u32 = 1;
/// Default TCP port of `kor peer serve`.
pub const DEFAULT_PORT: u16 = 47_321;
/// Secret store account of this machine's key.
pub const SECRET_ACCOUNT: &str = "peer:identity";

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

pub fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter().fold(String::with_capacity(b.len() * 2), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn key32(s: &str) -> Option<[u8; 32]> {
    unhex(s).and_then(|v| v.try_into().ok())
}

/// Short, readable form of a public key for comparing on two screens: `ab12-cd34-ef56-7890`.
pub fn fingerprint(key: &[u8; 32]) -> String {
    let h = hex(&Sha256::digest(key)[..8]);
    format!("{}-{}-{}-{}", &h[0..4], &h[4..8], &h[8..12], &h[12..16])
}

/// This machine's long-term key pair.
#[derive(Clone)]
pub struct Identity {
    private: [u8; 32],
    pub public: [u8; 32],
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public", &hex(&self.public))
            .finish_non_exhaustive()
    }
}

impl Drop for Identity {
    fn drop(&mut self) {
        // Best effort: overwrite the private key before the memory is freed.
        self.private = [0; 32];
        std::hint::black_box(&self.private);
    }
}

impl Identity {
    /// # Errors
    /// When the system random source fails.
    pub fn generate() -> Result<Self, String> {
        let (private, public) = noise::generate()?;
        Ok(Self { private, public })
    }

    pub fn private_bytes(&self) -> &[u8] {
        &self.private
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public)
    }

    fn encode(&self) -> SecretString {
        SecretString::from(format!("{}:{}", hex(&self.private), hex(&self.public)))
    }

    fn decode(s: &str) -> Option<Self> {
        let (p, q) = s.split_once(':')?;
        Some(Self {
            private: key32(p)?,
            public: key32(q)?,
        })
    }

    /// The key in `store`, created on first use.
    ///
    /// # Errors
    /// When the store cannot be read or written, or holds a damaged key.
    pub fn load_or_create(store: &dyn SecretStore) -> Result<Self, String> {
        if let Some(s) = store.get(SECRET_ACCOUNT).map_err(|e| e.to_string())? {
            return Self::decode(s.expose_secret())
                .ok_or_else(|| "the stored peer key is damaged".to_owned());
        }
        let id = Self::generate()?;
        store
            .set(SECRET_ACCOUNT, &id.encode())
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
}

/// What this machine lets one peer do (read-only by default).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Grant {
    /// Directories on this machine the peer may run tasks in (besides its own snapshots).
    pub dirs: Vec<String>,
    /// May change files in `dirs`. Snapshot workspaces are the task's own copy and always
    /// writable.
    pub write: bool,
    /// Command prefixes the peer's tasks may run (`cargo test`, `npm run`, `*` = any).
    pub commands: Vec<String>,
    /// Extra tools by name (`WebFetch`, `mcp__…`; `network` = every network tool).
    pub tools: Vec<String>,
    /// Records the peer sends are merged into this machine's knowledge base.
    pub memory: bool,
}

/// One paired machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerEntry {
    pub name: String,
    /// Its public key, hex.
    pub key: String,
    /// `host:port` addresses it listens on (`ts:` prefix = a Tailscale address).
    #[serde(default)]
    pub addrs: Vec<String>,
    /// `user@host[:port]` or a `connections.toml` name: reach it through SSH.
    #[serde(default)]
    pub ssh: Option<String>,
    /// What this machine lets it do.
    #[serde(default)]
    pub grant: Grant,
    #[serde(default)]
    pub paired_at: u64,
}

impl PeerEntry {
    pub fn key_bytes(&self) -> Option<[u8; 32]> {
        key32(&self.key)
    }
}

/// `<kor home>/peer/peers.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Book {
    /// This machine's name as peers see it.
    pub name: String,
    /// Address `kor peer serve` listens on.
    pub listen: String,
    /// The peer `kor peer standby` and the desktop hand off to when this machine closes.
    pub handoff: Option<String>,
    #[serde(rename = "peer")]
    pub peers: Vec<PeerEntry>,
}

/// Peer names: what a user types (`kor peer run server …`), safe as a directory name.
pub fn valid_name(n: &str) -> bool {
    (1..=64).contains(&n.len())
        && n
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !n.starts_with('.')
}

fn host_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        })
        .unwrap_or_default();
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let clean = clean.trim_matches('-').to_owned();
    if clean.is_empty() { "kor".into() } else { clean }
}

impl Book {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("peers.toml")
    }

    /// The book in `dir`, or a new one named after this computer.
    ///
    /// # Errors
    /// When the file exists but cannot be read or parsed.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut b = match std::fs::read_to_string(Self::path(dir)) {
            Ok(t) => toml::from_str::<Self>(&t)
                .map_err(|e| format!("{}: {e}", Self::path(dir).display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.to_string()),
        };
        if b.name.is_empty() {
            b.name = host_name();
        }
        if b.listen.is_empty() {
            b.listen = format!("0.0.0.0:{DEFAULT_PORT}");
        }
        Ok(b)
    }

    /// Atomic write.
    ///
    /// # Errors
    /// On I/O errors.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("peers.toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, Self::path(dir)).map_err(|e| e.to_string())
    }

    pub fn get(&self, name: &str) -> Option<&PeerEntry> {
        self.peers.iter().find(|p| p.name == name)
    }

    pub fn by_key(&self, key: &[u8; 32]) -> Option<&PeerEntry> {
        self.peers
            .iter()
            .find(|p| p.key_bytes().as_ref() == Some(key))
    }

    /// Add or replace by key; a new name that clashes gets a `-2`, `-3` … suffix.
    pub fn upsert(&mut self, mut e: PeerEntry) -> String {
        if let Some(i) = self.peers.iter().position(|p| p.key == e.key) {
            e.name.clone_from(&self.peers[i].name);
            e.grant = self.peers[i].grant.clone();
            if e.ssh.is_none() {
                e.ssh.clone_from(&self.peers[i].ssh);
            }
            self.peers[i] = e;
            return self.peers[i].name.clone();
        }
        let base = if valid_name(&e.name) { e.name.clone() } else { "peer".into() };
        let mut name = base.clone();
        let mut n = 2;
        while self.get(&name).is_some() {
            name = format!("{base}-{n}");
            n += 1;
        }
        e.name.clone_from(&name);
        self.peers.push(e);
        name
    }

    /// # Errors
    /// When no peer has that name.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let before = self.peers.len();
        self.peers.retain(|p| p.name != name);
        if self.peers.len() == before {
            Err(format!("no peer named {name}"))
        } else {
            Ok(())
        }
    }
}

/// Starts the detached worker of a delegated task: `(peer, task id)`.
pub type SpawnWorker = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

/// One machine's side of the bridge: its key, its data directory and how it starts workers.
#[derive(Clone)]
pub struct Node {
    /// `<kor home>/peer` (tests: a temp dir per node).
    pub dir: PathBuf,
    pub identity: Identity,
    /// Where session transcripts live (handoff writes the checkpoint there).
    pub sessions: PathBuf,
    /// Kor home of the knowledge base (`knowledge.sqlite`); `None` = no memory sync.
    pub knowledge_home: Option<PathBuf>,
    pub spawn: SpawnWorker,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("dir", &self.dir)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl Node {
    /// The book, read fresh (grants edited on disk apply to the next request).
    ///
    /// # Errors
    /// As [`Book::load`].
    pub fn book(&self) -> Result<Book, String> {
        Book::load(&self.dir)
    }

    /// Task store of one peer's delegated tasks: peers never see each other's tasks.
    pub fn store(&self, peer: &str) -> crate::remote::TaskStore {
        crate::remote::TaskStore {
            dir: self.dir.join("tasks").join(peer),
        }
    }

    /// The handshake payload of this machine.
    ///
    /// # Errors
    /// As [`Book::load`].
    pub fn hello(&self) -> Result<serde_json::Value, String> {
        let b = self.book()?;
        Ok(serde_json::json!({
            "name": b.name, "protocol": PROTOCOL, "version": kor_core::VERSION,
            "addrs": client::advertised_addrs(&b.listen),
        }))
    }
}

/// `<kor home>/peer`.
pub fn home() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("peer"))
}

/// The production node: the configured secret store holds the key; workers are
/// `<this exe> peer worker <peer> <id>`.
///
/// # Errors
/// When there is no data directory or the secret store fails.
pub fn local_node(store: &dyn SecretStore) -> Result<Node, String> {
    let dir = home().ok_or("no data directory")?;
    let identity = Identity::load_or_create(store)?;
    let sessions = crate::sessions_dir().ok_or("no data directory")?;
    let spawn_dir = dir.clone();
    Ok(Node {
        dir,
        identity,
        sessions,
        knowledge_home: crate::kor_home(),
        spawn: Arc::new(move |peer: &str, id: &str| spawn_worker(&spawn_dir, peer, id)),
    })
}

/// Detached `<exe> peer worker <peer> <id>`, output to `<task>/worker.log`.
///
/// # Errors
/// If the process cannot start.
pub fn spawn_worker(dir: &Path, peer: &str, id: &str) -> Result<(), String> {
    use kor_platform::Detach as _;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let task_dir = dir.join("tasks").join(peer).join(id);
    let log = std::fs::File::create(task_dir.join("worker.log")).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(["peer", "worker", peer, id])
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        .detach()
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the task worker: {e}"))
}

/// Audit both ends (§6.7): `peer.<action>` with the peer's name and key fingerprint.
pub fn audit(action: &str, peer: &str, mut fields: serde_json::Value) {
    if let Some(o) = fields.as_object_mut() {
        o.insert("peer".into(), serde_json::Value::String(peer.to_owned()));
    }
    kor_agent::audit::record(&format!("peer.{action}"), fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_round_trips_and_names_are_unique() {
        let d = tempfile::tempdir().unwrap();
        let mut b = Book::load(d.path()).unwrap();
        assert!(!b.name.is_empty());
        assert!(b.listen.ends_with(":47321"));
        let e = |name: &str, key: &str| PeerEntry {
            name: name.into(),
            key: key.into(),
            addrs: vec!["127.0.0.1:1".into()],
            ssh: None,
            grant: Grant::default(),
            paired_at: 1,
        };
        assert_eq!(b.upsert(e("srv", &"aa".repeat(32))), "srv");
        assert_eq!(b.upsert(e("srv", &"bb".repeat(32))), "srv-2");
        // Same key again: keeps its name and grant.
        b.peers[0].grant.write = true;
        assert_eq!(b.upsert(e("other", &"aa".repeat(32))), "srv");
        assert!(b.peers[0].grant.write);
        b.save(d.path()).unwrap();
        let c = Book::load(d.path()).unwrap();
        assert_eq!(c, b);
        assert!(c.by_key(&[0xaa; 32]).is_some());
        assert!(c.by_key(&[0xcc; 32]).is_none());
        assert!(valid_name("home-pc.lan") && !valid_name("../x") && !valid_name(""));
    }

    #[test]
    fn identity_encodes_and_never_prints_the_private_key() {
        let id = Identity::generate().unwrap();
        let back = Identity::decode(id.encode().expose_secret()).unwrap();
        assert_eq!(back.public, id.public);
        assert_eq!(back.private_bytes(), id.private_bytes());
        assert!(!format!("{id:?}").contains(&hex(id.private_bytes())));
        assert_eq!(id.fingerprint().len(), 19);
        assert_eq!(unhex("0aff"), Some(vec![10, 255]));
        assert_eq!(unhex("0g"), None);
    }
}
