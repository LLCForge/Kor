//! Everything the server keeps: members, certificates, tokens, versioned shared documents, the task
//! queue, the vault, share links and the audit chain. Tokens are stored only as SHA-256 hashes.

use std::path::Path;

use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use base64::Engine as _;
use serde::Serialize;
use serde_json::{Value, json};

use crate::db::{Db, Row, Val};
use crate::{Layout, ServerConfig, now_ms, pki, random_token, sha256_hex};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// A failure with the HTTP status it maps to; the message is safe to show the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// HTTP status code.
    pub status: u16,
    /// Message for the caller.
    pub message: String,
}

impl Error {
    /// An error with this status and message.
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<String> for Error {
    fn from(e: String) -> Self {
        Self::new(500, e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Result with the server's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// What a member may do. Ordered: each role includes the ones below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Reads shared documents, the queue and reports.
    Viewer,
    /// Also writes memory, graph and reports, queues and works tasks, opens vault secrets, relays.
    Member,
    /// Also writes rules, templates, hooks and tools, the vault, and reads the audit log.
    Admin,
}

impl Role {
    /// `admin`, `member` (or the spec's `dev`), `viewer`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "admin" => Some(Self::Admin),
            "member" | "dev" => Some(Self::Member),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// The role's name as stored and shown.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }
}

/// Shared document namespaces and the role that may write each.
pub fn writer_of(ns: &str) -> Option<Role> {
    match ns {
        // Synced to every member and acted on there (hooks and tools run commands): admins only.
        "rules" | "templates" | "hooks" | "tools" => Some(Role::Admin),
        "memory" | "graph" | "reports" => Some(Role::Member),
        _ => None,
    }
}

/// `a-z A-Z 0-9 . _ - /`, 1–200 characters, no empty or `..` segment.
pub fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 200
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        && k.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// A lower-case e-mail address of sane shape.
///
/// # Errors
/// Anything that is not one.
pub fn normalize_email(e: &str) -> Result<String> {
    let e = e.trim().to_ascii_lowercase();
    let ok = e.len() <= 254
        && e.split_once('@').is_some_and(|(u, d)| {
            !u.is_empty()
                && !d.is_empty()
                && d.contains('.')
                && !e.contains("..")
                && e.bytes().filter(|b| *b == b'@').count() == 1
                && e.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
        });
    if ok {
        Ok(e)
    } else {
        Err(Error::new(400, "not an e-mail address"))
    }
}

/// What `add-member` hands to the new member (also written as a bundle file).
#[derive(Clone, Serialize)]
pub struct Enrollment {
    /// Member's e-mail address.
    pub email: String,
    /// Member's role.
    pub role: Role,
    /// `https://<domain>:<port>`.
    /// Server URL the member connects to.
    pub server: String,
    /// Team CA certificate, PEM.
    pub ca: String,
    /// Member's client certificate, PEM.
    pub cert: String,
    /// Member's client key, PEM.
    pub key: String,
    /// Bearer token for the member.
    pub token: String,
    /// When the token expires (Unix ms).
    pub expires_ms: i64,
    /// Where the bundle file was written (not part of the bundle).
    #[serde(skip)]
    pub bundle: std::path::PathBuf,
}

impl std::fmt::Debug for Enrollment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Enrollment")
            .field("email", &self.email)
            .field("role", &self.role)
            .field("bundle", &self.bundle)
            .finish_non_exhaustive()
    }
}

/// One shared document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    /// Namespace (`rules`, `templates`, `memory`, …).
    pub ns: String,
    /// Key inside the namespace.
    pub key: String,
    /// Document text.
    pub body: String,
    /// Version, raised on every write.
    pub version: i64,
    /// Who wrote the current version.
    pub updated_by: String,
    /// When it was written (Unix ms).
    pub updated_ms: i64,
}

/// One queued task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Task id.
    pub id: String,
    /// `queued`, `running`, `done`, `failed`.
    /// `queued`, `running`, `done` or `failed`.
    pub state: String,
    /// What to do.
    pub prompt: String,
    /// Repository the task runs on.
    pub repo: String,
    /// Callback URL notified at the end.
    pub callback: String,
    /// Who queued it.
    pub created_by: String,
    /// Member that claimed it.
    pub worker: String,
    /// Result text.
    pub result: String,
    /// When it was queued (Unix ms).
    pub created_ms: i64,
    /// Last change (Unix ms).
    pub updated_ms: i64,
    /// Per-task callback signing secret.
    #[serde(skip)]
    pub secret: String,
}

/// One audit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Sequence number.
    pub seq: i64,
    /// Time (Unix ms).
    pub ts_ms: i64,
    /// Who did it.
    pub actor: String,
    /// What was done.
    pub action: String,
    /// What it was done to.
    pub target: String,
    /// Details.
    pub detail: String,
    /// Hash of the previous entry.
    pub prev: String,
    /// Hash of this entry, chained to `prev`.
    pub hash: String,
}

/// Counts for `kor server status`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Members with the admin role.
    pub admins: i64,
    /// Members with the member role.
    pub members: i64,
    /// Members with the viewer role.
    pub viewers: i64,
    /// Shared documents.
    pub docs: i64,
    /// Queued tasks.
    pub queued: i64,
    /// Running tasks.
    pub running: i64,
    /// Vault secrets.
    pub vault: i64,
    /// Audit entries.
    pub audit: i64,
}

/// The server's storage.
pub struct Store {
    /// The database.
    pub db: Db,
    vault: LessSafeKey,
    /// Serialises audit appends so the chain has no forks.
    audit_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("db", &self.db)
            .finish_non_exhaustive()
    }
}

const TASK_COLS: &str =
    "id, state, prompt, repo, callback, created_by, worker, result, created_ms, updated_ms, secret";

fn task_of(r: &Row) -> Task {
    Task {
        id: r[0].s(),
        state: r[1].s(),
        prompt: r[2].s(),
        repo: r[3].s(),
        callback: r[4].s(),
        created_by: r[5].s(),
        worker: r[6].s(),
        result: r[7].s(),
        created_ms: r[8].i(),
        updated_ms: r[9].i(),
        secret: r[10].s(),
    }
}

impl Store {
    /// Open the database and the vault key (`KOR_SERVER_VAULT_KEY`, 64 hex chars, or
    /// `vault.key` in the server directory).
    ///
    /// # Errors
    /// The database cannot be opened or the vault key is missing/damaged.
    pub async fn open(cfg: &ServerConfig, dir: &Path) -> std::result::Result<Self, String> {
        let db = Db::open(&cfg.db, dir).await?;
        let hexkey = match std::env::var("KOR_SERVER_VAULT_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            // kor-lint: allow(blocking_in_async) a 64-byte key file, read once at server start
            _ => std::fs::read_to_string(Layout::new(dir).vault_key())
                .map_err(|e| format!("vault key: {e}"))?,
        };
        let raw = unhex(hexkey.trim())
            .filter(|k| k.len() == 32)
            .ok_or("vault key: 64 hex characters expected")?;
        let key = UnboundKey::new(&CHACHA20_POLY1305, &raw).map_err(|_| "vault key")?;
        Ok(Self {
            db,
            vault: LessSafeKey::new(key),
            audit_lock: tokio::sync::Mutex::new(()),
        })
    }

    // -- audit -----------------------------------------------------------------------------------

    fn chain_hash(
        prev: &str,
        seq: i64,
        ts: i64,
        actor: &str,
        action: &str,
        target: &str,
        detail: &str,
    ) -> String {
        let canon = json!([prev, seq, ts, actor, action, target, detail]).to_string();
        sha256_hex(&canon)
    }

    /// Append one entry to the hash chain.
    ///
    /// # Errors
    /// A database error.
    pub async fn audit(
        &self,
        actor: &str,
        action: &str,
        target: &str,
        detail: &Value,
    ) -> Result<()> {
        let _g = self.audit_lock.lock().await;
        let last = self
            .db
            .query("SELECT seq, hash FROM audit ORDER BY seq DESC LIMIT 1", &[])
            .await?;
        let (seq, prev) = last
            .first()
            .map_or((1, String::new()), |r| (r[0].i() + 1, r[1].s()));
        let ts = now_ms();
        let detail = detail.to_string();
        let hash = Self::chain_hash(&prev, seq, ts, actor, action, target, &detail);
        self.db
            .exec(
                "INSERT INTO audit (seq, ts_ms, actor, action, target, detail, prev, hash) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[seq.into(), ts.into(), actor.into(), action.into(), target.into(), detail.into(), prev.into(), hash.into()],
            )
            .await?;
        Ok(())
    }

    /// Entries after `after`, at most `limit` (1–1000).
    ///
    /// # Errors
    /// A database error.
    pub async fn audit_list(&self, after: i64, limit: i64) -> Result<Vec<AuditEntry>> {
        let rows = self
            .db
            .query(
                "SELECT seq, ts_ms, actor, action, target, detail, prev, hash FROM audit WHERE seq > $1 ORDER BY seq LIMIT $2",
                &[after.into(), limit.clamp(1, 1000).into()],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| AuditEntry {
                seq: r[0].i(),
                ts_ms: r[1].i(),
                actor: r[2].s(),
                action: r[3].s(),
                target: r[4].s(),
                detail: r[5].s(),
                prev: r[6].s(),
                hash: r[7].s(),
            })
            .collect())
    }

    /// Walk the whole chain; the number of entries, or the first broken sequence number.
    ///
    /// # Errors
    /// `Err(seq)` where the chain breaks; a database error as `Err(-1)`.
    pub async fn audit_verify(&self) -> std::result::Result<i64, i64> {
        let mut prev = String::new();
        let mut after = 0;
        let mut n = 0;
        loop {
            let page = self.audit_list(after, 1000).await.map_err(|_| -1)?;
            if page.is_empty() {
                return Ok(n);
            }
            for e in &page {
                let h = Self::chain_hash(
                    &prev, e.seq, e.ts_ms, &e.actor, &e.action, &e.target, &e.detail,
                );
                if e.prev != prev || e.hash != h || e.seq != n + 1 {
                    return Err(e.seq);
                }
                prev.clone_from(&e.hash);
                n += 1;
                after = e.seq;
            }
        }
    }

    // -- members -------------------------------------------------------------------------------

    /// Add a member (or change their role) and issue a client certificate and a member token;
    /// writes the bundle to `members/<email>.json` (owner-only).
    ///
    /// # Errors
    /// A bad e-mail, signing or a database error.
    pub async fn add_member(
        &self,
        cfg: &ServerConfig,
        layout: &Layout,
        email: &str,
        role: Role,
        by: &str,
    ) -> Result<Enrollment> {
        let email = normalize_email(email)?;
        let ca_key = tokio::fs::read_to_string(layout.ca_key())
            .await
            .map_err(|e| format!("CA key: {e}"))?;
        let ca = tokio::fs::read_to_string(layout.ca_cert())
            .await
            .map_err(|e| format!("CA: {e}"))?;
        let cert = pki::issue_client(&cfg.team, &ca_key, &email, i64::from(cfg.member_days))?;
        let now = now_ms();
        self.db
            .exec(
                "INSERT INTO members (email, role, created_ms, added_by) VALUES ($1, $2, $3, $4) ON CONFLICT (email) DO UPDATE SET role = excluded.role",
                &[(&email).into(), role.as_str().into(), now.into(), by.into()],
            )
            .await?;
        self.db
            .exec(
                "INSERT INTO certs (fingerprint, email, expires_ms, revoked) VALUES ($1, $2, $3, $4)",
                &[(&cert.fingerprint).into(), (&email).into(), cert.expires_ms.into(), 0i64.into()],
            )
            .await?;
        let token = random_token("kmt")?;
        let expires = now + i64::from(cfg.member_days) * 86_400_000;
        self.db
            .exec(
                "INSERT INTO tokens (hash, email, kind, expires_ms) VALUES ($1, $2, $3, $4)",
                &[
                    sha256_hex(&token).into(),
                    (&email).into(),
                    "member".into(),
                    expires.into(),
                ],
            )
            .await?;
        let port = cfg
            .listen
            .parse::<std::net::SocketAddr>()
            .map_or(8443, |a| a.port());
        let e = Enrollment {
            server: format!("https://{}:{port}", cfg.domain),
            email: email.clone(),
            role,
            ca,
            cert: cert.cert_pem,
            key: cert.key_pem,
            token,
            expires_ms: expires,
            bundle: layout
                .members()
                .join(format!("{}.json", email.replace('@', "_at_"))),
        };
        let text = serde_json::to_string_pretty(&e).map_err(|x| x.to_string())?;
        pki::write_private(&e.bundle, &text)?;
        self.audit(
            by,
            "member.add",
            &email,
            &json!({ "role": role.as_str(), "cert": cert.fingerprint }),
        )
        .await?;
        Ok(e)
    }

    /// Remove a member: their certificates are revoked and their tokens deleted.
    ///
    /// # Errors
    /// Not a member, or a database error.
    pub async fn remove_member(&self, email: &str, by: &str) -> Result<()> {
        let email = normalize_email(email)?;
        let n = self
            .db
            .exec("DELETE FROM members WHERE email = $1", &[(&email).into()])
            .await?;
        if n == 0 {
            return Err(Error::new(404, format!("{email} is not a member")));
        }
        self.db
            .exec(
                "UPDATE certs SET revoked = $1 WHERE email = $2",
                &[1i64.into(), (&email).into()],
            )
            .await?;
        self.db
            .exec("DELETE FROM tokens WHERE email = $1", &[(&email).into()])
            .await?;
        self.audit(by, "member.remove", &email, &json!({})).await
    }

    /// The member's role, if they are one.
    ///
    /// # Errors
    /// A database error.
    pub async fn role(&self, email: &str) -> Result<Option<Role>> {
        let rows = self
            .db
            .query("SELECT role FROM members WHERE email = $1", &[email.into()])
            .await?;
        Ok(rows.first().and_then(|r| Role::parse(&r[0].s())))
    }

    /// Every member with their role.
    ///
    /// # Errors
    /// A database error.
    pub async fn members(&self) -> Result<Vec<(String, Role)>> {
        let rows = self
            .db
            .query("SELECT email, role FROM members ORDER BY email", &[])
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| Some((r[0].s(), Role::parse(&r[1].s())?)))
            .collect())
    }

    /// The e-mail a live, unrevoked client certificate belongs to.
    ///
    /// # Errors
    /// A database error.
    pub async fn cert_owner(&self, fingerprint: &str) -> Result<Option<String>> {
        let rows = self
            .db
            .query(
                "SELECT email FROM certs WHERE fingerprint = $1 AND revoked = $2 AND expires_ms > $3",
                &[fingerprint.into(), 0i64.into(), now_ms().into()],
            )
            .await?;
        Ok(rows.first().map(|r| r[0].s()))
    }

    /// The e-mail a live token belongs to.
    ///
    /// # Errors
    /// A database error.
    pub async fn token_owner(&self, token: &str) -> Result<Option<String>> {
        if token.len() > 200 {
            return Ok(None);
        }
        let rows = self
            .db
            .query(
                "SELECT email FROM tokens WHERE hash = $1 AND expires_ms > $2",
                &[sha256_hex(token).into(), now_ms().into()],
            )
            .await?;
        Ok(rows.first().map(|r| r[0].s()))
    }

    /// A session token for a member who signed in through SSO.
    ///
    /// # Errors
    /// A database error.
    pub async fn issue_session(&self, email: &str, hours: u32, via: &str) -> Result<(String, i64)> {
        let token = random_token("kst")?;
        let expires = now_ms() + i64::from(hours) * 3_600_000;
        self.db
            .exec(
                "INSERT INTO tokens (hash, email, kind, expires_ms) VALUES ($1, $2, $3, $4)",
                &[
                    sha256_hex(&token).into(),
                    email.into(),
                    "session".into(),
                    expires.into(),
                ],
            )
            .await?;
        // Expired tokens go when new ones come.
        self.db
            .exec(
                "DELETE FROM tokens WHERE expires_ms < $1",
                &[now_ms().into()],
            )
            .await?;
        self.audit(email, "auth.session", via, &json!({ "expires": expires }))
            .await?;
        Ok((token, expires))
    }

    // -- shared documents ----------------------------------------------------------------------

    /// # Errors
    /// A database error.
    pub async fn doc(&self, ns: &str, key: &str, version: Option<i64>) -> Result<Option<Doc>> {
        let rows = match version {
            None => {
                self.db
                    .query(
                        "SELECT body, version, updated_by, updated_ms FROM docs WHERE ns = $1 AND key = $2",
                        &[ns.into(), key.into()],
                    )
                    .await?
            }
            Some(v) => {
                self.db
                    .query(
                        "SELECT body, version, updated_by, updated_ms FROM doc_history WHERE ns = $1 AND key = $2 AND version = $3",
                        &[ns.into(), key.into(), v.into()],
                    )
                    .await?
            }
        };
        Ok(rows.first().map(|r| Doc {
            ns: ns.to_owned(),
            key: key.to_owned(),
            body: r[0].s(),
            version: r[1].i(),
            updated_by: r[2].s(),
            updated_ms: r[3].i(),
        }))
    }

    /// Keys and versions in a namespace (bodies left out).
    ///
    /// # Errors
    /// A database error.
    pub async fn docs(&self, ns: &str) -> Result<Vec<Doc>> {
        let rows = self
            .db
            .query(
                "SELECT key, version, updated_by, updated_ms FROM docs WHERE ns = $1 ORDER BY key LIMIT 5000",
                &[ns.into()],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Doc {
                ns: ns.to_owned(),
                key: r[0].s(),
                body: String::new(),
                version: r[1].i(),
                updated_by: r[2].s(),
                updated_ms: r[3].i(),
            })
            .collect())
    }

    /// Write a new version. `expected` (the version the writer last saw; 0 = must not exist)
    /// turns a lost update into a 409 instead of silently overwriting someone else's change.
    ///
    /// # Errors
    /// A version conflict or a database error.
    pub async fn put_doc(
        &self,
        ns: &str,
        key: &str,
        body: &str,
        expected: Option<i64>,
        by: &str,
    ) -> Result<i64> {
        let guard = self.audit_lock.lock().await;
        let cur = self
            .db
            .query(
                "SELECT version FROM docs WHERE ns = $1 AND key = $2",
                &[ns.into(), key.into()],
            )
            .await?
            .first()
            .map_or(0, |r| r[0].i());
        if let Some(e) = expected
            && e != cur
        {
            return Err(Error::new(
                409,
                format!("version conflict: the server has version {cur}"),
            ));
        }
        let v = cur + 1;
        let now = now_ms();
        self.db
            .exec(
                "INSERT INTO docs (ns, key, body, version, updated_by, updated_ms) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (ns, key) DO UPDATE SET body = excluded.body, version = excluded.version, updated_by = excluded.updated_by, updated_ms = excluded.updated_ms",
                &[ns.into(), key.into(), body.into(), v.into(), by.into(), now.into()],
            )
            .await?;
        self.db
            .exec(
                "INSERT INTO doc_history (ns, key, version, body, updated_by, updated_ms) VALUES ($1, $2, $3, $4, $5, $6)",
                &[ns.into(), key.into(), v.into(), body.into(), by.into(), now.into()],
            )
            .await?;
        drop(guard);
        self.audit(
            by,
            "doc.put",
            &format!("{ns}/{key}"),
            &json!({ "version": v, "bytes": body.len() }),
        )
        .await?;
        Ok(v)
    }

    // -- vault ---------------------------------------------------------------------------------

    /// Store a secret sealed with the vault key (the name is the associated data, so a sealed
    /// value cannot be moved to another name).
    ///
    /// # Errors
    /// A database or crypto error.
    pub async fn vault_put(&self, name: &str, value: &str, by: &str) -> Result<()> {
        let mut nonce = [0u8; NONCE_LEN];
        aws_lc_rs::rand::fill(&mut nonce).map_err(|_| "no system randomness".to_owned())?;
        let mut buf = value.as_bytes().to_vec();
        self.vault
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(name.as_bytes()),
                &mut buf,
            )
            .map_err(|_| "seal failed".to_owned())?;
        self.db
            .exec(
                "INSERT INTO vault (name, nonce, sealed, updated_by, updated_ms) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (name) DO UPDATE SET nonce = excluded.nonce, sealed = excluded.sealed, updated_by = excluded.updated_by, updated_ms = excluded.updated_ms",
                &[name.into(), B64.encode(nonce).into(), B64.encode(&buf).into(), by.into(), now_ms().into()],
            )
            .await?;
        self.audit(by, "vault.put", name, &json!({})).await
    }

    /// Open a secret; every opening is audited.
    ///
    /// # Errors
    /// A damaged entry (wrong key) or a database error.
    pub async fn vault_get(&self, name: &str, by: &str) -> Result<Option<String>> {
        let rows = self
            .db
            .query(
                "SELECT nonce, sealed FROM vault WHERE name = $1",
                &[name.into()],
            )
            .await?;
        let Some(r) = rows.first() else {
            return Ok(None);
        };
        let nonce: [u8; NONCE_LEN] = B64
            .decode(r[0].s())
            .ok()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(|| "damaged vault entry".to_owned())?;
        let mut buf = B64
            .decode(r[1].s())
            .map_err(|_| "damaged vault entry".to_owned())?;
        let plain = self
            .vault
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(name.as_bytes()),
                &mut buf,
            )
            .map_err(|_| "vault entry does not open with this key".to_owned())?;
        let v = String::from_utf8(plain.to_vec()).map_err(|_| "damaged vault entry".to_owned())?;
        self.audit(by, "vault.open", name, &json!({})).await?;
        Ok(Some(v))
    }

    /// # Errors
    /// A database error.
    pub async fn vault_names(&self) -> Result<Vec<String>> {
        let rows = self
            .db
            .query("SELECT name FROM vault ORDER BY name", &[])
            .await?;
        Ok(rows.iter().map(|r| r[0].s()).collect())
    }

    /// # Errors
    /// Not found, or a database error.
    pub async fn vault_delete(&self, name: &str, by: &str) -> Result<()> {
        if self
            .db
            .exec("DELETE FROM vault WHERE name = $1", &[name.into()])
            .await?
            == 0
        {
            return Err(Error::new(404, "no such secret"));
        }
        self.audit(by, "vault.delete", name, &json!({})).await
    }

    // -- share links ---------------------------------------------------------------------------

    /// A read-only link to one report for someone outside the team, expiring after `hours`.
    ///
    /// # Errors
    /// No such report, or a database error.
    pub async fn share(&self, ns: &str, key: &str, hours: i64, by: &str) -> Result<(String, i64)> {
        if self.doc(ns, key, None).await?.is_none() {
            return Err(Error::new(404, "no such document"));
        }
        let token = random_token("ksh")?;
        let expires = now_ms() + hours.clamp(1, 24 * 30) * 3_600_000;
        self.db
            .exec(
                "INSERT INTO shares (hash, ns, key, created_by, expires_ms) VALUES ($1, $2, $3, $4, $5)",
                &[sha256_hex(&token).into(), ns.into(), key.into(), by.into(), expires.into()],
            )
            .await?;
        self.audit(
            by,
            "share.create",
            &format!("{ns}/{key}"),
            &json!({ "expires": expires }),
        )
        .await?;
        Ok((token, expires))
    }

    /// The document behind a live share link.
    ///
    /// # Errors
    /// A database error.
    pub async fn shared(&self, token: &str) -> Result<Option<Doc>> {
        if token.len() > 200 {
            return Ok(None);
        }
        let rows = self
            .db
            .query(
                "SELECT ns, key FROM shares WHERE hash = $1 AND expires_ms > $2",
                &[sha256_hex(token).into(), now_ms().into()],
            )
            .await?;
        let Some(r) = rows.first() else {
            return Ok(None);
        };
        let d = self.doc(&r[0].s(), &r[1].s(), None).await?;
        if d.is_some() {
            self.audit(
                "share-link",
                "share.open",
                &format!("{}/{}", r[0].s(), r[1].s()),
                &json!({}),
            )
            .await?;
        }
        Ok(d)
    }

    // -- task queue ----------------------------------------------------------------------------

    /// Queue a task; returns it with its callback secret.
    ///
    /// # Errors
    /// A database error.
    pub async fn task_create(
        &self,
        prompt: &str,
        repo: &str,
        callback: &str,
        by: &str,
    ) -> Result<Task> {
        let id = random_token("tsk")?;
        let secret = random_token("kcb")?;
        let now = now_ms();
        self.db
            .exec(
                "INSERT INTO tasks (id, state, prompt, repo, callback, secret, created_by, worker, result, created_ms, updated_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[(&id).into(), "queued".into(), prompt.into(), repo.into(), callback.into(), (&secret).into(), by.into(), "".into(), "".into(), now.into(), now.into()],
            )
            .await?;
        self.audit(
            by,
            "task.create",
            &id,
            &json!({ "repo": repo, "callback": !callback.is_empty() }),
        )
        .await?;
        self.task(&id)
            .await?
            .ok_or_else(|| Error::new(500, "task vanished"))
    }

    /// # Errors
    /// A database error.
    pub async fn task(&self, id: &str) -> Result<Option<Task>> {
        let rows = self
            .db
            .query(
                &format!("SELECT {TASK_COLS} FROM tasks WHERE id = $1"),
                &[id.into()],
            )
            .await?;
        Ok(rows.first().map(task_of))
    }

    /// # Errors
    /// A database error.
    pub async fn tasks(&self, state: Option<&str>, limit: i64) -> Result<Vec<Task>> {
        let rows = match state {
            Some(s) => {
                self.db
                    .query(
                        &format!("SELECT {TASK_COLS} FROM tasks WHERE state = $1 ORDER BY created_ms DESC LIMIT $2"),
                        &[s.into(), limit.clamp(1, 500).into()],
                    )
                    .await?
            }
            None => {
                self.db
                    .query(
                        &format!("SELECT {TASK_COLS} FROM tasks ORDER BY created_ms DESC LIMIT $1"),
                        &[limit.clamp(1, 500).into()],
                    )
                    .await?
            }
        };
        Ok(rows.iter().map(task_of).collect())
    }

    /// Hand the oldest queued task to `worker` (compare-and-set, so two workers never get the
    /// same task).
    ///
    /// # Errors
    /// A database error.
    pub async fn task_claim(&self, worker: &str) -> Result<Option<Task>> {
        for _ in 0..5 {
            let rows = self
                .db
                .query(
                    "SELECT id FROM tasks WHERE state = $1 ORDER BY created_ms LIMIT 1",
                    &["queued".into()],
                )
                .await?;
            let Some(id) = rows.first().map(|r| r[0].s()) else {
                return Ok(None);
            };
            let n = self
                .db
                .exec(
                    "UPDATE tasks SET state = $1, worker = $2, updated_ms = $3 WHERE id = $4 AND state = $5",
                    &["running".into(), worker.into(), now_ms().into(), (&id).into(), "queued".into()],
                )
                .await?;
            if n == 1 {
                self.audit(worker, "task.claim", &id, &json!({})).await?;
                return self.task(&id).await;
            }
        }
        Ok(None)
    }

    /// Finish a running task (only its worker may).
    ///
    /// # Errors
    /// Not this worker's running task, or a database error.
    pub async fn task_finish(
        &self,
        id: &str,
        worker: &str,
        ok: bool,
        result: &str,
    ) -> Result<Task> {
        let state = if ok { "done" } else { "failed" };
        let n = self
            .db
            .exec(
                "UPDATE tasks SET state = $1, result = $2, updated_ms = $3 WHERE id = $4 AND worker = $5 AND state = $6",
                &[state.into(), result.into(), now_ms().into(), id.into(), worker.into(), "running".into()],
            )
            .await?;
        if n == 0 {
            return Err(Error::new(409, "not a running task of this worker"));
        }
        self.audit(worker, "task.finish", id, &json!({ "state": state }))
            .await?;
        self.task(id)
            .await?
            .ok_or_else(|| Error::new(500, "task vanished"))
    }

    // -- status --------------------------------------------------------------------------------

    /// # Errors
    /// A database error.
    pub async fn counts(&self) -> Result<Counts> {
        let one = |sql: &'static str, p: Vec<Val>| async move {
            self.db
                .query(sql, &p)
                .await
                .map(|r| r.first().map_or(0, |r| r[0].i()))
        };
        let role = |r: &'static str| {
            one(
                "SELECT COUNT(*) FROM members WHERE role = $1",
                vec![r.into()],
            )
        };
        let state = |s: &'static str| {
            one(
                "SELECT COUNT(*) FROM tasks WHERE state = $1",
                vec![s.into()],
            )
        };
        Ok(Counts {
            admins: role("admin").await?,
            members: role("member").await?,
            viewers: role("viewer").await?,
            docs: one("SELECT COUNT(*) FROM docs", Vec::new()).await?,
            queued: state("queued").await?,
            running: state("running").await?,
            vault: one("SELECT COUNT(*) FROM vault", Vec::new()).await?,
            audit: one("SELECT COUNT(*) FROM audit", Vec::new()).await?,
        })
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store(dir: &Path) -> (Store, ServerConfig, Layout) {
        let (cfg, _) = crate::init(dir, "acme", "localhost", "root@example.com", None)
            .await
            .unwrap();
        (Store::open(&cfg, dir).await.unwrap(), cfg, Layout::new(dir))
    }

    #[tokio::test]
    async fn vault_is_sealed_at_rest_and_bound_to_its_name() {
        let d = tempfile::tempdir().unwrap();
        let (s, _, _) = store(d.path()).await;
        s.vault_put(
            "DATABASE_URL",
            "postgres://u:hunter2@db/x",
            "root@example.com",
        )
        .await
        .unwrap();
        let raw = s.db.query("SELECT sealed FROM vault", &[]).await.unwrap();
        let sealed = B64.decode(raw[0][0].s()).unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("hunter2"));
        assert_eq!(
            s.vault_get("DATABASE_URL", "root@example.com")
                .await
                .unwrap()
                .as_deref(),
            Some("postgres://u:hunter2@db/x")
        );
        // Moving the sealed row to another name does not open.
        s.db.exec("UPDATE vault SET name = $1", &["OTHER".into()])
            .await
            .unwrap();
        assert!(s.vault_get("OTHER", "root@example.com").await.is_err());
    }

    #[tokio::test]
    async fn audit_chain_detects_tampering() {
        let d = tempfile::tempdir().unwrap();
        let (s, _, _) = store(d.path()).await;
        s.put_doc(
            "rules",
            "AGENTS.md",
            "be brief",
            Some(0),
            "root@example.com",
        )
        .await
        .unwrap();
        let n = s.audit_verify().await.unwrap();
        assert!(n >= 2, "{n}");
        s.db.exec(
            "UPDATE audit SET actor = $1 WHERE seq = $2",
            &["eve".into(), 2i64.into()],
        )
        .await
        .unwrap();
        assert_eq!(s.audit_verify().await, Err(2));
    }

    #[tokio::test]
    async fn versions_conflict_instead_of_overwriting() {
        let d = tempfile::tempdir().unwrap();
        let (s, _, _) = store(d.path()).await;
        assert_eq!(
            s.put_doc("memory", "k", "a", Some(0), "x@y.z")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            s.put_doc("memory", "k", "b", Some(1), "x@y.z")
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            s.put_doc("memory", "k", "c", Some(1), "x@y.z")
                .await
                .unwrap_err()
                .status,
            409
        );
        assert_eq!(
            s.doc("memory", "k", Some(1)).await.unwrap().unwrap().body,
            "a"
        );
        assert_eq!(s.doc("memory", "k", None).await.unwrap().unwrap().body, "b");
    }

    #[tokio::test]
    async fn removed_member_loses_certificate_and_tokens() {
        let d = tempfile::tempdir().unwrap();
        let (s, cfg, l) = store(d.path()).await;
        let e = s
            .add_member(
                &cfg,
                &l,
                "Ali@Example.com",
                Role::Member,
                "root@example.com",
            )
            .await
            .unwrap();
        assert_eq!(e.email, "ali@example.com");
        assert!(e.bundle.is_file());
        let fp = pki::fingerprint(&pki::certs_from_pem(e.cert.as_bytes()).unwrap()[0]);
        assert_eq!(
            s.cert_owner(&fp).await.unwrap().as_deref(),
            Some("ali@example.com")
        );
        assert_eq!(
            s.token_owner(&e.token).await.unwrap().as_deref(),
            Some("ali@example.com")
        );
        s.remove_member("ali@example.com", "root@example.com")
            .await
            .unwrap();
        assert!(s.cert_owner(&fp).await.unwrap().is_none());
        assert!(s.token_owner(&e.token).await.unwrap().is_none());
        assert!(s.role("ali@example.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn one_task_goes_to_one_worker() {
        let d = tempfile::tempdir().unwrap();
        let (s, _, _) = store(d.path()).await;
        let t = s
            .task_create("fix the build", "acme/app", "", "a@x.io")
            .await
            .unwrap();
        let got = s.task_claim("w1@x.io").await.unwrap().unwrap();
        assert_eq!(got.id, t.id);
        assert!(s.task_claim("w2@x.io").await.unwrap().is_none());
        assert_eq!(
            s.task_finish(&t.id, "w2@x.io", true, "x")
                .await
                .unwrap_err()
                .status,
            409
        );
        assert_eq!(
            s.task_finish(&t.id, "w1@x.io", true, "diff")
                .await
                .unwrap()
                .state,
            "done"
        );
    }

    #[test]
    fn keys_and_emails() {
        assert!(valid_key("AGENTS.md") && valid_key("templates/new-endpoint.md"));
        for bad in ["", "../x", "a//b", "a/./b", "a b", "a\\b"] {
            assert!(!valid_key(bad), "{bad}");
        }
        assert!(normalize_email("a@b.c").is_ok());
        for bad in ["a", "@b.c", "a@b", "a@@b.c", "a b@c.d", "a@b..c"] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
        assert_eq!(Role::parse("dev"), Some(Role::Member));
        assert!(Role::Admin > Role::Member && Role::Member > Role::Viewer);
    }
}
