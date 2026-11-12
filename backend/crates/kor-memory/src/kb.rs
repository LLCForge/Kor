//! Global knowledge base (§31.3): `<kor home>/knowledge.sqlite`.
//!
//! One `SQLite` file (WAL, `synchronous=NORMAL`, large `mmap_size`, cached prepared statements)
//! with an FTS5 index. Every write goes to one writer thread over a channel; the thread commits
//! whatever has queued up (at most 256 records) in one transaction, so a burst becomes one group
//! commit and a lone write is committed at once. Full-text indexing is not part of that commit
//! (§31.3.1: a user write never waits for the query layer, ADR 0062): between writes the same
//! thread indexes the newest unindexed records in small transactions, so a write waits for at
//! most one chunk. Readers use their own connection (WAL lets them run while the writer commits).

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender as Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::embed::{Embed, Kind, VecIndex, quantize};

/// Records committed together at most.
const GROUP: usize = 256;
/// Messages (single records or whole batches) queued for the writer at most.
const QUEUE: usize = 16_384;
const CHECKPOINT_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// Quiet time before the writer merges FTS segments.
const IDLE: std::time::Duration = std::time::Duration::from_millis(500);
const MAX_FIELD: usize = 4000;
const MAX_LIST: usize = 50;
const MAX_TERMS: usize = 16;
const MAX_WORKSPACES: usize = 1000;
/// A word must be in more recent records than this (and in over 5% of them) to be common.
const COMMON_DOCS: i64 = 500;
/// Rare words of one query that are ranked (the rarest ones).
const RANKED_TERMS: usize = 6;
/// Newest ids searched first for a query of only common words.
const RECENT: i64 = 20_000;
/// A query of only common words looks at the newest ids in these widening windows (then all):
/// stop words are everywhere, so the newest few thousand rows almost always hold the answer.
const WINDOWS: [i64; 2] = [2_000, RECENT];
/// A word's measured frequency is reused until the base has grown by this many records (the
/// in-memory hot word index of §31.3.1).
const WORD_TTL: i64 = 5_000;
/// Words remembered at most (the cache is cleared when full).
const WORD_CACHE: usize = 20_000;
/// Score multiplier for records of the project being worked on (ranking, not filtering).
const SAME_PROJECT_BOOST: f64 = 2.0;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS records(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    project TEXT NOT NULL,
    kind TEXT NOT NULL,
    task TEXT NOT NULL,
    summary TEXT NOT NULL,
    files TEXT NOT NULL,
    decisions TEXT NOT NULL,
    errors TEXT NOT NULL,
    commands TEXT NOT NULL,
    rules TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS records_project_ts ON records(project, ts);
CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(
    project, task, summary, files, decisions, errors, commands, rules,
    content='records', content_rowid='id', tokenize='unicode61 remove_diacritics 2', detail='column'
);
CREATE TABLE IF NOT EXISTS workspaces(path TEXT PRIMARY KEY, added INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS vectors(id INTEGER PRIMARY KEY, model TEXT NOT NULL, vec BLOB NOT NULL);
CREATE TRIGGER IF NOT EXISTS records_vec_ad AFTER DELETE ON records BEGIN
    DELETE FROM vectors WHERE id = old.id;
END;
";

/// Records in the FTS index: ids up to `upto` (the rest are queued for the indexer). Created at
/// first open with every existing record, which the old insert trigger had indexed.
const FTS_STATE: &str = "
CREATE TABLE fts_state(upto INTEGER NOT NULL);
INSERT INTO fts_state(upto) SELECT coalesce(max(id), 0) FROM records;
";
/// Indexing replaced the insert trigger; deleting a record removes it from the index only when
/// it was indexed (an external-content `delete` of an unindexed row would corrupt the index).
const FTS_TRIGGERS: &str = "
DROP TRIGGER IF EXISTS records_ai;
DROP TRIGGER IF EXISTS records_ad;
CREATE TRIGGER records_ad AFTER DELETE ON records WHEN old.id <= (SELECT upto FROM fts_state) BEGIN
    INSERT INTO records_fts(records_fts, rowid, project, task, summary, files, decisions, errors, commands, rules)
    VALUES ('delete', old.id, old.project, old.task, old.summary, old.files, old.decisions, old.errors, old.commands, old.rules);
END;
";
/// Records indexed per indexing transaction: a write arriving meanwhile waits for one at most.
const INDEX_CHUNK: i64 = 256;

/// Low-power mode (§32.3, ADR 0062): background embedding pauses this long between batches.
const LOW_POWER_PAUSE: std::time::Duration = std::time::Duration::from_secs(2);
static LOW_POWER: AtomicBool = AtomicBool::new(false);

/// Slow background indexing (embedding) down on battery / small machines.
pub fn set_low_power(on: bool) {
    LOW_POWER.store(on, Ordering::Relaxed);
}

/// Records embedded per background batch (about half a second on a laptop CPU; closing the
/// base waits for the batch in progress).
const EMBED_BATCH: usize = 8;
/// Results taken from each of FTS and vector search before they are fused.
const FUSE_DEPTH: usize = 50;
/// Reciprocal rank fusion constant (Cormack et al. 2009).
const RRF_K: f64 = 60.0;

const COLUMNS: &str = "r.id, r.ts, r.project, r.kind, r.task, r.summary, r.files, r.decisions, r.errors, r.commands, r.rules";

/// One "what we learned" entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    /// Assigned by the database; ignored on write.
    pub id: i64,
    /// Unix seconds.
    pub ts: i64,
    /// [`crate::project_key`] of the project.
    pub project: String,
    /// `task`, `note`, ...
    pub kind: String,
    pub task: String,
    pub summary: String,
    pub files: Vec<String>,
    /// Decisions and their reasons.
    pub decisions: Vec<String>,
    /// Errors solved and how.
    pub errors: Vec<String>,
    pub commands: Vec<String>,
    /// Rules learned.
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub record: Record,
    /// Higher is more relevant (BM25, same-project boost applied).
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One Obsidian-friendly Markdown file per record.
    Markdown,
    /// One `knowledge.json` array.
    Json,
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

type Job = Box<dyn FnOnce(&Connection) -> Result<i64> + Send>;

enum Msg {
    Insert(Vec<Record>),
    /// Runs inside the group's transaction; the answer is sent after the commit.
    Run(Job, Sender<Result<i64>>),
    /// Answered once every record committed before it is in the full-text index.
    Indexed(Sender<Result<i64>>),
}

/// The vector side of the base: the embedder, the lazily loaded index and the background
/// thread that embeds new records.
struct Vectors {
    embed: Arc<dyn Embed>,
    /// `None` until the first semantic search loads it.
    index: Arc<Mutex<Option<VecIndex>>>,
    stop: Arc<AtomicBool>,
    wake: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

pub struct Knowledge {
    path: PathBuf,
    tx: Option<Sender<Msg>>,
    writer: Option<JoinHandle<()>>,
    stop: Option<Sender<()>>,
    checkpointer: Option<JoinHandle<()>>,
    read: Mutex<Connection>,
    /// Hot word index: word → (documents in the newest window, counted up to the common line;
    /// base size when counted).
    words: Mutex<HashMap<String, (i64, i64)>>,
    vectors: Option<Vectors>,
}

impl fmt::Debug for Knowledge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Knowledge")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn connect(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    // Address space only (pages load on use): 1M records are ~0.5 GB.
    c.pragma_update(None, "mmap_size", 1_073_741_824_i64)?;
    c.pragma_update(None, "cache_size", -32_768)?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    c.set_prepared_statement_cache_capacity(32);
    Ok(c)
}

fn clip(s: &str) -> String {
    let s = crate::mask(s.trim());
    if s.len() <= MAX_FIELD {
        return s;
    }
    let mut end = MAX_FIELD;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// List items masked and clipped, one line each (stored newline-joined).
fn items(v: &[String]) -> Vec<String> {
    v.iter()
        .take(MAX_LIST)
        .map(|s| clip(&s.replace('\n', " ")))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Secrets masked, fields clipped, timestamp set. Runs on the caller's thread, so the writer
/// only inserts.
fn normalize(mut r: Record) -> Record {
    if r.ts == 0 {
        r.ts = chrono::Utc::now().timestamp();
    }
    r.project = clip(&r.project);
    let kind: String = r
        .kind
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(20)
        .collect();
    r.kind = if kind.is_empty() { "task".into() } else { kind };
    r.task = clip(&r.task);
    r.summary = clip(&r.summary);
    for v in [
        &mut r.files,
        &mut r.decisions,
        &mut r.errors,
        &mut r.commands,
        &mut r.rules,
    ] {
        *v = items(v);
    }
    r
}

fn unlist(s: &str) -> Vec<String> {
    s.lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn row(r: &Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: r.get(0)?,
        ts: r.get(1)?,
        project: r.get(2)?,
        kind: r.get(3)?,
        task: r.get(4)?,
        summary: r.get(5)?,
        files: unlist(&r.get::<_, String>(6)?),
        decisions: unlist(&r.get::<_, String>(7)?),
        errors: unlist(&r.get::<_, String>(8)?),
        commands: unlist(&r.get::<_, String>(9)?),
        rules: unlist(&r.get::<_, String>(10)?),
    })
}

fn insert(c: &Connection, r: &Record) -> rusqlite::Result<()> {
    c.prepare_cached(
        "INSERT INTO records(ts, project, kind, task, summary, files, decisions, errors, commands, rules)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![
        r.ts,
        r.project,
        r.kind,
        r.task,
        r.summary,
        r.files.join("\n"),
        r.decisions.join("\n"),
        r.errors.join("\n"),
        r.commands.join("\n"),
        r.rules.join("\n"),
    ])
    .map(|_| ())
}

/// One bounded piece of FTS index merging; `false` once there is nothing left to merge.
fn merge_step(c: &Connection) -> bool {
    let before = c.total_changes();
    if c.execute(
        "INSERT INTO records_fts(records_fts, rank) VALUES ('merge', 500)",
        [],
    )
    .is_err()
    {
        return false;
    }
    // SQLite: a difference of 2 or more means the command did some merging.
    c.total_changes().saturating_sub(before) >= 2
}

/// Index the next unindexed records (at most [`INDEX_CHUNK`]) in one transaction. Returns the
/// new `upto`.
fn index_chunk(c: &mut Connection, upto: i64, last: i64) -> rusqlite::Result<i64> {
    let tx = c.transaction()?;
    let end: Option<i64> = tx
        .prepare_cached(
            "SELECT max(id) FROM (SELECT id FROM records WHERE id > ?1 ORDER BY id LIMIT ?2)",
        )?
        .query_row(params![upto, INDEX_CHUNK], |r| r.get(0))?;
    // Nothing left: the rest was forgotten before it was indexed.
    let end = end.unwrap_or(last).max(upto);
    tx.prepare_cached(
        "INSERT INTO records_fts(rowid, project, task, summary, files, decisions, errors, commands, rules)
         SELECT id, project, task, summary, files, decisions, errors, commands, rules
         FROM records WHERE id > ?1 AND id <= ?2 ORDER BY id",
    )?
    .execute(params![upto, end])?;
    tx.prepare_cached("UPDATE fts_state SET upto = ?1")?
        .execute([end])?;
    tx.commit()?;
    Ok(end)
}

fn max_id(c: &Connection) -> i64 {
    c.query_row("SELECT coalesce(max(id), 0) FROM records", [], |r| r.get(0))
        .unwrap_or(0)
}

/// Writer state between group commits.
struct Indexer {
    /// Records with ids up to this are in the FTS index.
    upto: i64,
    /// Highest committed record id.
    last: i64,
    /// Callers of [`Knowledge::flush`] with the id they wait for.
    waiting: Vec<(i64, Sender<Result<i64>>)>,
    /// An indexing error stops indexing until the next write (no busy loop on a broken disk).
    stalled: bool,
}

impl Indexer {
    fn answer(&mut self) {
        let upto = self.upto;
        self.waiting.retain(|(target, reply)| {
            if *target <= upto {
                let _ = reply.send(Ok(0));
                false
            } else {
                true
            }
        });
    }
}

fn writer(mut conn: Connection, rx: &Receiver<Msg>, wake: Option<&Sender<()>>) {
    let mut ix = Indexer {
        upto: conn
            .query_row("SELECT upto FROM fts_state", [], |r| r.get(0))
            .unwrap_or(0),
        last: max_id(&conn),
        waiting: Vec::new(),
        stalled: false,
    };
    // FTS segments written since the last idle merge (the indexer only merges a little).
    let mut dirty = true;
    loop {
        // Unindexed records: index a chunk whenever no write is waiting.
        if ix.upto < ix.last && !ix.stalled {
            match rx.try_recv() {
                Ok(m) => {
                    group(&mut conn, m, rx, &mut ix, wake);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            match index_chunk(&mut conn, ix.upto, ix.last) {
                Ok(end) => ix.upto = end,
                Err(e) => {
                    ix.stalled = true;
                    for (_, reply) in ix.waiting.drain(..) {
                        let _ = reply.send(Err(Error(e.to_string())));
                    }
                }
            }
            dirty = true;
            ix.answer();
            continue;
        }
        let first = if dirty {
            match rx.recv_timeout(IDLE) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    dirty = merge_step(&conn);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(m) => m,
                Err(_) => break,
            }
        };
        dirty = true;
        group(&mut conn, first, rx, &mut ix, wake);
    }
    // Closing: index what is left, so the base on disk is complete.
    while ix.upto < ix.last && !ix.stalled {
        match index_chunk(&mut conn, ix.upto, ix.last) {
            Ok(end) => ix.upto = end,
            Err(_) => ix.stalled = true,
        }
    }
    for (_, reply) in ix.waiting.drain(..) {
        let _ = reply.send(Ok(0));
    }
}

/// One group commit starting with `first`: everything already queued, up to GROUP records, goes
/// into this transaction (one `record_all` call is never split).
fn group(
    conn: &mut Connection,
    first: Msg,
    rx: &Receiver<Msg>,
    ix: &mut Indexer,
    wake: Option<&Sender<()>>,
) {
    let size = |m: &Msg| match m {
        Msg::Insert(rs) => rs.len(),
        Msg::Run(..) | Msg::Indexed(_) => 1,
    };
    let mut queued = size(&first);
    let mut batch = vec![first];
    while queued < GROUP {
        match rx.try_recv() {
            Ok(m) => {
                queued += size(&m);
                batch.push(m);
            }
            Err(_) => break,
        }
    }
    let mut answers = Vec::new();
    let mut index_waits = Vec::new();
    let inserted = batch.iter().any(|m| matches!(m, Msg::Insert(_)));
    let committed = conn.transaction().and_then(|tx| {
        for m in batch {
            match m {
                // shortcut: a record that fails to insert is dropped; the rest still commit.
                Msg::Insert(rs) => {
                    for r in &rs {
                        let _ = insert(&tx, r);
                    }
                }
                Msg::Run(job, reply) => answers.push((reply, job(&tx))),
                Msg::Indexed(reply) => index_waits.push(reply),
            }
        }
        tx.commit()
    });
    ix.last = max_id(conn);
    if inserted && committed.is_ok() {
        // A new write retries a stalled indexer.
        ix.stalled = false;
        // New records to embed (in the background; this write is already committed).
        if let Some(w) = wake {
            // A wake-up already queued is enough.
            let _ = w.try_send(());
        }
    }
    for (reply, answer) in answers {
        let _ = reply.send(match &committed {
            Ok(()) => answer,
            Err(e) => Err(Error(e.to_string())),
        });
    }
    let target = ix.last;
    ix.waiting
        .extend(index_waits.into_iter().map(|reply| (target, reply)));
    ix.answer();
}

/// Run `job` on the writer thread and wait for its committed answer.
fn run_on(
    tx: Option<&Sender<Msg>>,
    job: impl FnOnce(&Connection) -> Result<i64> + Send + 'static,
) -> Result<i64> {
    let (reply, answer) = mpsc::sync_channel(1);
    tx.and_then(|t| t.send(Msg::Run(Box::new(job), reply)).ok())
        .ok_or_else(|| Error("knowledge writer stopped".into()))?;
    answer
        .recv()
        .map_err(|_| Error("knowledge writer stopped".into()))?
}

/// The text a record is embedded from.
fn passage(r: &Row<'_>) -> rusqlite::Result<(i64, String)> {
    let mut text = String::new();
    for i in 1..=5 {
        let s: String = r.get(i)?;
        if !s.is_empty() {
            text.push_str(&s);
            text.push('\n');
        }
    }
    Ok((r.get(0)?, text))
}

/// Records after `after` without a vector of `model`, oldest first.
fn pending(c: &Connection, model: &str, after: i64, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut st = c.prepare_cached(
        "SELECT r.id, r.task, r.summary, r.decisions, r.errors, r.rules FROM records r
         LEFT JOIN vectors v ON v.id = r.id
         WHERE r.id > ?1 AND (v.id IS NULL OR v.model <> ?2) ORDER BY r.id LIMIT ?3",
    )?;
    let rows = st.query_map(params![after, model, limit_i64(limit)], passage)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Embed one batch of records after `after` and store their vectors (index updated under its
/// lock, so a concurrent load never sees a vector twice). Returns the last id done, `None` when
/// nothing was pending.
fn embed_batch(
    c: &Connection,
    embed: &dyn Embed,
    tx: Option<&Sender<Msg>>,
    index: &Mutex<Option<VecIndex>>,
    after: i64,
) -> Result<Option<(i64, usize)>> {
    let todo = pending(c, embed.model(), after, EMBED_BATCH)?;
    let Some(last) = todo.last().map(|t| t.0) else {
        return Ok(None);
    };
    let texts: Vec<String> = todo.iter().map(|t| t.1.clone()).collect();
    let vecs = embed.embed(&texts, Kind::Passage).map_err(Error)?;
    let rows: Vec<(i64, Vec<u8>)> = todo
        .iter()
        .zip(&vecs)
        .map(|(t, v)| (t.0, quantize(v)))
        .collect();
    let n = rows.len();
    let model = embed.model().to_owned();
    let mut ix = index
        .lock()
        .map_err(|_| Error("vector index poisoned".into()))?;
    let stored = rows.clone();
    run_on(tx, move |c| {
        let mut st = c.prepare_cached(
            "INSERT OR REPLACE INTO vectors(id, model, vec)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM records WHERE id = ?1)",
        )?;
        for (id, v) in &stored {
            st.execute(params![id, model, v])?;
        }
        Ok(0)
    })?;
    if let Some(ix) = ix.as_mut() {
        for (id, v) in &rows {
            ix.remove(*id);
            ix.push(*id, v);
        }
    }
    Ok(Some((last, n)))
}

/// Background embedding: new records after each commit, older ones missing a vector at start.
fn embed_loop(
    c: &Connection,
    embed: &dyn Embed,
    tx: &Sender<Msg>,
    index: &Mutex<Option<VecIndex>>,
    stop: &AtomicBool,
    wake: &Receiver<()>,
) {
    // Everything up to the newest vector is done (`kor knowledge reindex` fills older gaps).
    let mut after: i64 = c
        .query_row(
            "SELECT coalesce(max(id), 0) FROM vectors WHERE model = ?1",
            [embed.model()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    while !stop.load(Ordering::Relaxed) {
        match embed_batch(c, embed, Some(tx), index, after) {
            Ok(Some((last, _))) => {
                after = last;
                if LOW_POWER.load(Ordering::Relaxed) {
                    // Woken early by a close (stop flag checked by the loop) or new records.
                    let _ = wake.recv_timeout(LOW_POWER_PAUSE);
                }
            }
            Ok(None) => {
                if wake.recv().is_err() {
                    break;
                }
                while wake.try_recv().is_ok() {}
            }
            // shortcut: a model that fails stops background embedding until the next open.
            Err(_) => break,
        }
    }
}

fn load_index(c: &Connection, model: &str) -> Result<VecIndex> {
    let mut ix = VecIndex::default();
    let mut st = c.prepare("SELECT id, vec FROM vectors WHERE model = ?1")?;
    let mut rows = st.query([model])?;
    while let Some(r) = rows.next()? {
        let blob = r.get_ref(1)?.as_blob().unwrap_or_default();
        ix.push(r.get(0)?, blob);
    }
    Ok(ix)
}

/// Reciprocal rank fusion of ranked lists; `boost`'s records score ×2. Best first.
fn fuse(lists: Vec<Vec<Hit>>, boost: Option<&str>, limit: usize) -> Vec<Hit> {
    let mut by_id: HashMap<i64, Hit> = HashMap::new();
    for list in lists {
        for (rank, h) in list.into_iter().enumerate() {
            // Ranks are small (≤ a few hundred).
            #[allow(clippy::cast_precision_loss)]
            let s = 1.0 / (RRF_K + rank as f64 + 1.0);
            by_id
                .entry(h.record.id)
                .and_modify(|e| e.score += s)
                .or_insert(Hit { score: s, ..h });
        }
    }
    let mut out: Vec<Hit> = by_id
        .into_values()
        .map(|mut h| {
            if boost.is_some_and(|b| b == h.record.project) {
                h.score *= SAME_PROJECT_BOOST;
            }
            h
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.record.id.cmp(&a.record.id))
    });
    out.truncate(limit);
    out
}

impl Knowledge {
    /// Open (creating) the database at `path` and start its writer thread.
    ///
    /// # Errors
    /// When the file cannot be opened or its schema created.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, None)
    }

    /// [`Self::open`] with an embedder: new records are embedded by a background thread and
    /// [`Self::recall`] fuses vector similarity with FTS.
    ///
    /// # Errors
    /// When the file cannot be opened or its schema created.
    pub fn open_with(path: &Path, embed: Option<Arc<dyn Embed>>) -> Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let w = connect(path)?;
        // Larger pages: fewer page writes per FTS segment (takes effect when the file is new).
        w.pragma_update(None, "page_size", 8192)?;
        w.execute_batch(SCHEMA)?;
        // Asynchronous indexing (ADR 0062): the insert trigger of older bases goes; what it
        // indexed counts as indexed.
        let has_state: bool = w.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'fts_state'",
            [],
            |r| r.get(0),
        )?;
        if !has_state {
            w.execute_batch(FTS_STATE)?;
        }
        w.execute_batch(FTS_TRIGGERS)?;
        // Commits never stop for a checkpoint (and its fsync): a separate thread does them.
        w.pragma_update(None, "wal_autocheckpoint", 0)?;
        // Index maintenance never on a user's write (§31.3.1): only the background indexer writes
        // to the FTS index, and it merges a little as it goes (automerge), so a long burst keeps
        // few segments for searches (ADR 0062); idle time finishes the merging.
        w.execute_batch(
            "INSERT INTO records_fts(records_fts, rank) VALUES ('automerge', 8);
             INSERT INTO records_fts(records_fts, rank) VALUES ('crisismerge', 16);",
        )?;
        let read = connect(path)?;
        let ckpt = connect(path)?;
        // Bounded (§32.2): a writer that falls this far behind makes `record` wait (backpressure)
        // instead of growing the queue without limit.
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let (wake, woken) = mpsc::sync_channel(1);
        let writer_wake = embed.as_ref().map(|_| wake.clone());
        let writer = std::thread::Builder::new()
            .name("kor-knowledge".into())
            .spawn(move || writer(w, &rx, writer_wake.as_ref()))?;
        let vectors = match embed {
            Some(embed) => {
                let index = Arc::new(Mutex::new(None));
                let stop = Arc::new(AtomicBool::new(false));
                let (conn, model, queue, ix, halt) = (
                    connect(path)?,
                    embed.clone(),
                    tx.clone(),
                    index.clone(),
                    stop.clone(),
                );
                let thread = std::thread::Builder::new()
                    .name("kor-knowledge-embed".into())
                    .spawn(move || {
                        embed_loop(&conn, model.as_ref(), &queue, &ix, &halt, &woken);
                    })?;
                Some(Vectors {
                    embed,
                    index,
                    stop,
                    wake,
                    thread: Some(thread),
                })
            }
            None => None,
        };
        let (stop, stopped) = mpsc::sync_channel::<()>(1);
        let checkpointer = std::thread::Builder::new()
            .name("kor-knowledge-checkpoint".into())
            .spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(CHECKPOINT_EVERY)
                {
                    let _ = ckpt.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
                }
            })?;
        Ok(Self {
            path: path.to_owned(),
            tx: Some(tx),
            writer: Some(writer),
            stop: Some(stop),
            checkpointer: Some(checkpointer),
            read: Mutex::new(read),
            words: Mutex::default(),
            vectors,
        })
    }

    /// Whether this base has an embedder (semantic recall is on).
    pub fn semantic(&self) -> bool {
        self.vectors.is_some()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn send(&self, m: Msg) -> Result<()> {
        self.tx
            .as_ref()
            .and_then(|t| t.send(m).ok())
            .ok_or_else(|| Error("knowledge writer stopped".into()))
    }

    /// Run `job` on the writer thread and wait for its committed answer.
    fn run(&self, job: impl FnOnce(&Connection) -> Result<i64> + Send + 'static) -> Result<i64> {
        run_on(self.tx.as_ref(), job)
    }

    /// Queue a record (secrets masked, fields clipped). Returns at once; the writer commits it
    /// with whatever else is queued. [`Self::flush`] waits for it.
    ///
    /// # Errors
    /// Only when the writer thread is gone.
    pub fn record(&self, r: Record) -> Result<()> {
        self.send(Msg::Insert(vec![normalize(r)]))
    }

    /// Queue many records as one transaction (imports, bulk loads).
    ///
    /// # Errors
    /// Only when the writer thread is gone.
    pub fn record_all(&self, rs: Vec<Record>) -> Result<()> {
        self.send(Msg::Insert(rs.into_iter().map(normalize).collect()))
    }

    /// Wait until every record queued before this call is committed and in the full-text index
    /// (searchable).
    ///
    /// # Errors
    /// When the commit or the indexing failed, or the writer is gone.
    pub fn flush(&self) -> Result<()> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.send(Msg::Indexed(reply))?;
        answer
            .recv()
            .map_err(|_| Error("knowledge writer stopped".into()))?
            .map(|_| ())
    }

    /// Wait until every record queued before this call is committed (durable). Full-text search
    /// sees them once the writer has indexed them, normally within milliseconds; [`Self::flush`]
    /// waits for that too.
    ///
    /// # Errors
    /// When the commit failed or the writer is gone.
    pub fn commit(&self) -> Result<()> {
        self.run(|_| Ok(0)).map(|_| ())
    }

    /// Merge the whole FTS index into one segment (what idle merging converges to). Slow on a
    /// large base; for maintenance and benchmarks, never on the agent's path.
    ///
    /// # Errors
    /// On a database error.
    pub fn optimize(&self) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO records_fts(records_fts) VALUES ('optimize')",
                [],
            )?;
            Ok(0)
        })
        .map(|_| ())
    }

    /// Delete one record. `Ok(false)` when there was none with that id.
    ///
    /// # Errors
    /// On a database error.
    pub fn forget(&self, id: i64) -> Result<bool> {
        // The index lock is held across the delete, so a concurrent load cannot keep it.
        let mut ix = match &self.vectors {
            Some(v) => Some(
                v.index
                    .lock()
                    .map_err(|_| Error("vector index poisoned".into()))?,
            ),
            None => None,
        };
        let gone = self
            .run(move |c| {
                Ok(i64::from(
                    c.execute("DELETE FROM records WHERE id = ?1", [id])? > 0,
                ))
            })
            .map(|n| n > 0)?;
        if let Some(Some(ix)) = ix.as_deref_mut() {
            ix.remove(id);
        }
        Ok(gone)
    }

    /// The best `limit` records for `query`: FTS5 rank and vector similarity fused by reciprocal
    /// rank fusion when an embedder is set, FTS only otherwise (or when embedding fails).
    /// `only` and `boost` work as in [`Self::search`].
    ///
    /// # Errors
    /// On a database error.
    pub fn recall(
        &self,
        query: &str,
        only: Option<&str>,
        boost: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        let limit = limit.clamp(1, 200);
        let depth = limit.max(FUSE_DEPTH);
        let mut fts = self.search(query, only, boost, depth)?;
        let near = match &self.vectors {
            Some(v) => self.nearest(v, query, only, depth).unwrap_or_default(),
            None => Vec::new(),
        };
        if near.is_empty() {
            fts.truncate(limit);
            return Ok(fts);
        }
        Ok(fuse(vec![fts, near], boost, limit))
    }

    /// Records nearest to `query` by vector similarity (score = cosine), best first.
    fn nearest(
        &self,
        v: &Vectors,
        query: &str,
        only: Option<&str>,
        depth: usize,
    ) -> Result<Vec<Hit>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let q = v
            .embed
            .embed(&[query.to_owned()], Kind::Query)
            .map_err(Error)?
            .pop()
            .ok_or_else(|| Error("no query vector".into()))?;
        let top = {
            let mut ix = v
                .index
                .lock()
                .map_err(|_| Error("vector index poisoned".into()))?;
            if ix.is_none() {
                *ix = Some(load_index(&*self.reader()?, v.embed.model())?);
            }
            // shortcut: a project filter looks 4× deeper and filters; a per-project index if
            // one project is a tiny share of a huge base.
            let k = if only.is_some() { depth * 4 } else { depth };
            ix.as_ref().map(|ix| ix.top(&q, k)).unwrap_or_default()
        };
        let best = top.first().map_or(0.0, |t| t.1);
        let min = v
            .embed
            .min_similarity()
            .max(best - v.embed.similarity_band());
        let c = self.reader()?;
        let mut st =
            c.prepare_cached(&format!("SELECT {COLUMNS} FROM records r WHERE r.id = ?1"))?;
        let mut out = Vec::new();
        for (id, sim) in top.into_iter().filter(|t| t.1 >= min) {
            if let Some(record) = st.query_row([id], row).optional()?
                && only.is_none_or(|p| p == record.project)
            {
                out.push(Hit {
                    record,
                    score: f64::from(sim),
                });
            }
            if out.len() == depth {
                break;
            }
        }
        Ok(out)
    }

    /// Embed every record that has no vector of the current model, `progress(done, total)`
    /// after each batch. Returns how many were embedded. Needs an embedder.
    ///
    /// # Errors
    /// Without an embedder, on a database error or when the model fails.
    pub fn reindex(&self, progress: &dyn Fn(usize, usize)) -> Result<usize> {
        let v = self
            .vectors
            .as_ref()
            .ok_or_else(|| Error("no embedding model installed".into()))?;
        let c = connect(&self.path)?;
        let total: i64 = c.query_row(
            "SELECT count(*) FROM records r LEFT JOIN vectors v ON v.id = r.id
             WHERE v.id IS NULL OR v.model <> ?1",
            [v.embed.model()],
            |r| r.get(0),
        )?;
        let total = usize::try_from(total).unwrap_or(0);
        let (mut after, mut done) = (0, 0);
        progress(0, total);
        while let Some((last, n)) =
            embed_batch(&c, v.embed.as_ref(), self.tx.as_ref(), &v.index, after)?
        {
            after = last;
            done += n;
            progress(done, total.max(done));
        }
        Ok(done)
    }

    /// Records with a vector of the current model (0 without an embedder).
    ///
    /// # Errors
    /// On a database error.
    pub fn embedded(&self) -> Result<u64> {
        let Some(v) = &self.vectors else {
            return Ok(0);
        };
        let c = self.reader()?;
        let n: i64 = c.query_row(
            "SELECT count(*) FROM vectors WHERE model = ?1",
            [v.embed.model()],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    fn reader(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.read
            .lock()
            .map_err(|_| Error("knowledge reader poisoned".into()))
    }

    /// One record by id.
    ///
    /// # Errors
    /// On a database error.
    pub fn get(&self, id: i64) -> Result<Option<Record>> {
        let c = self.reader()?;
        let mut st =
            c.prepare_cached(&format!("SELECT {COLUMNS} FROM records r WHERE r.id = ?1"))?;
        Ok(st.query_row([id], row).optional()?)
    }

    /// Number of records.
    ///
    /// # Errors
    /// On a database error.
    pub fn count(&self) -> Result<u64> {
        let c = self.reader()?;
        let n: i64 = c.query_row("SELECT count(*) FROM records", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Newest records first, optionally of one project.
    ///
    /// # Errors
    /// On a database error.
    pub fn recent(&self, project: Option<&str>, limit: usize) -> Result<Vec<Record>> {
        let c = self.reader()?;
        let mut st = c.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM records r WHERE ?1 IS NULL OR r.project = ?1 ORDER BY r.ts DESC, r.id DESC LIMIT ?2"
        ))?;
        let rows = st.query_map(params![project, limit_i64(limit)], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Full-text search (any of the query's words, BM25 ranked). `only` keeps one project;
    /// `boost` ranks that project's records higher without hiding the others.
    ///
    /// # Errors
    /// On a database error.
    pub fn search(
        &self,
        query: &str,
        only: Option<&str>,
        boost: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        let words = terms(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 200);
        let c = self.reader()?;
        let (fts, by_rank, total) = {
            let mut cache = self
                .words
                .lock()
                .map_err(|_| Error("word cache poisoned".into()))?;
            plan(&c, words, &mut cache)?
        };
        // Rare words: BM25 over their short posting lists. Only common words: all of them must
        // match and the newest rows win, looked for among the newest RECENT ids first, so FTS5
        // never walks most of the table.
        let order = if by_rank { "rank" } else { "rowid DESC" };
        // BM25 only for ranked queries: its IDF walks each term's whole posting list, ~10 ms per
        // stop word at 1M records (ADR 0062), and newest-first results never use the score.
        let rank = if by_rank { "rank" } else { "0.0" };
        // Without a project filter the FTS table is sorted and limited on its own (FTS5's fast
        // path), and only those rows are joined; a join first would read every match.
        let sql = if only.is_some() {
            format!(
                "SELECT {COLUMNS}, {rank} FROM records_fts f JOIN records r ON r.id = f.rowid
                 WHERE records_fts MATCH ?1 AND f.rowid > ?4 AND r.project = ?2
                 ORDER BY f.{order} LIMIT ?3"
            )
        } else {
            format!(
                "SELECT {COLUMNS}, f.score FROM (SELECT rowid, {rank} AS score FROM records_fts
                 WHERE records_fts MATCH ?1 AND rowid > ?4 ORDER BY {order} LIMIT ?3) f
                 JOIN records r ON r.id = f.rowid"
            )
        };
        let mut st = c.prepare_cached(&sql)?;
        let fetch = if boost.is_some() { limit * 4 } else { limit };
        let mut run = |after: i64| -> Result<Vec<Hit>> {
            let rows = st.query_map(params![fts, only, limit_i64(fetch), after], |r| {
                let rank: f64 = r.get(11)?;
                Ok((row(r)?, -rank))
            })?;
            let mut out = Vec::new();
            for x in rows {
                let (record, score) = x?;
                let boosted = boost.is_some_and(|b| b == record.project);
                out.push(Hit {
                    score: if boosted {
                        score * SAME_PROJECT_BOOST
                    } else {
                        score
                    },
                    record,
                });
            }
            Ok(out)
        };
        let mut hits = Vec::new();
        if !by_rank {
            for w in WINDOWS {
                let after = total - w;
                if after <= 0 {
                    break;
                }
                hits = run(after)?;
                if hits.len() >= fetch {
                    break;
                }
            }
        }
        if by_rank || hits.len() < fetch {
            hits = run(0)?;
        }
        if by_rank {
            hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        } else {
            // Newest first, the current project's records ahead.
            let mine = |h: &Hit| boost.is_some_and(|b| b == h.record.project);
            hits.sort_by(|a, b| mine(b).cmp(&mine(a)).then(b.record.id.cmp(&a.record.id)));
        }
        hits.truncate(limit);
        Ok(hits)
    }

    /// Write every record to `dir`. Returns how many were written.
    ///
    /// # Errors
    /// When `dir` cannot be created or a file cannot be written.
    pub fn export(&self, dir: &Path, format: ExportFormat) -> Result<usize> {
        fs::create_dir_all(dir)?;
        // A separate connection: the live reader is not held for the whole export.
        let c = connect(&self.path)?;
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM records r ORDER BY r.id"))?;
        let mut rows = st.query([])?;
        let mut n = 0;
        match format {
            ExportFormat::Json => {
                let mut out = BufWriter::new(fs::File::create(dir.join("knowledge.json"))?);
                out.write_all(b"[\n")?;
                while let Some(r) = rows.next()? {
                    if n > 0 {
                        out.write_all(b",\n")?;
                    }
                    serde_json::to_writer_pretty(&mut out, &row(r)?).map_err(io::Error::other)?;
                    n += 1;
                }
                out.write_all(b"\n]\n")?;
                out.flush()?;
            }
            ExportFormat::Markdown => {
                while let Some(r) = rows.next()? {
                    let r = row(r)?;
                    fs::write(dir.join(markdown_name(&r)), markdown(&r))?;
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// §27.15: the knowledge base as a folder people can browse (`~/Kor/Bilgi/` by default):
    /// `<project folder name>/<kind>/<date>-<id>-<title>.md`, plus the weekly digests under
    /// `Özet/` when `digests` is given. One way: Kor rewrites only files whose text changed and
    /// removes only files it wrote itself (listed in `.kor-folder.json`); anything else in the
    /// folder is left alone. Returns `(written, removed)`.
    ///
    /// # Errors
    /// When the folder or a file cannot be written.
    pub fn sync_folder(&self, dir: &Path, digests: Option<&Path>) -> Result<(usize, usize)> {
        const LIST: &str = ".kor-folder.json";
        fs::create_dir_all(dir)?;
        let before: BTreeSet<String> = fs::read_to_string(dir.join(LIST))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut now: BTreeSet<String> = BTreeSet::new();
        let mut written = 0;
        let mut put = |rel: String, text: &str| -> Result<()> {
            let path = dir.join(&rel);
            if fs::read_to_string(&path).ok().as_deref() != Some(text) {
                if let Some(p) = path.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::write(&path, text)?;
                written += 1;
            }
            now.insert(rel);
            Ok(())
        };
        put(
            "README.md".into(),
            "# Kor bilgi tabanı\n\nKor'un hafızası, okunabilir Markdown olarak. Kor bu klasörü \
             kendisi günceller; buradaki değişiklikler hafızaya geri alınmaz (düzenlemek için \
             `kor know`, `kor forget`).\n",
        )?;
        let c = connect(&self.path)?;
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM records r ORDER BY r.id"))?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let r = row(r)?;
            let project = Path::new(&r.project)
                .file_name()
                .map_or_else(|| "Genel".to_owned(), |n| folder_name(&n.to_string_lossy()));
            let kind = folder_name(if r.kind.is_empty() { "note" } else { &r.kind });
            put(
                format!("{project}/{kind}/{}", markdown_name(&r)),
                &markdown(&r),
            )?;
        }
        if let Some(d) = digests {
            for e in fs::read_dir(d).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("md"))
                    && let Ok(text) = fs::read_to_string(e.path())
                {
                    put(format!("Özet/{name}"), &text)?;
                }
            }
        }
        let mut removed = 0;
        for rel in before.difference(&now) {
            // Only paths inside the folder (the list file could have been edited).
            if !rel.split(['/', '\\']).any(|s| s == ".." || s.is_empty())
                && fs::remove_file(dir.join(rel)).is_ok()
            {
                removed += 1;
            }
        }
        fs::write(
            dir.join(LIST),
            serde_json::to_string_pretty(&now).map_err(io::Error::other)?,
        )?;
        Ok((written, removed))
    }

    /// Register folders for the multi-repo view: a path, or a path whose last part has
    /// wildcards (`~/projects/*`). Returns the folders added (already registered ones too).
    ///
    /// # Errors
    /// When nothing matches or the registry is full.
    pub fn workspace_add(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let found = expand(pattern)?;
        if found.is_empty() {
            return Err(Error(format!("no folder matches {pattern}")));
        }
        let paths: Vec<String> = found.iter().map(|p| crate::project_key(p)).collect();
        let now = chrono::Utc::now().timestamp();
        self.run(move |c| {
            let have: i64 = c.query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))?;
            if usize::try_from(have).unwrap_or(usize::MAX) + paths.len() > MAX_WORKSPACES {
                return Err(Error(format!("at most {MAX_WORKSPACES} workspace folders")));
            }
            let mut st =
                c.prepare_cached("INSERT OR IGNORE INTO workspaces(path, added) VALUES (?1, ?2)")?;
            for p in &paths {
                st.execute(params![p, now])?;
            }
            Ok(0)
        })?;
        Ok(found)
    }

    /// `Ok(false)` when the folder was not registered.
    ///
    /// # Errors
    /// On a database error.
    pub fn workspace_remove(&self, path: &str) -> Result<bool> {
        let given = crate::project_key(Path::new(path.trim()));
        let canon = crate::canonical(Path::new(path.trim()))
            .ok()
            .map(|p| crate::project_key(&p));
        self.run(move |c| {
            let n = c.execute(
                "DELETE FROM workspaces WHERE path = ?1 OR path = ?2",
                params![given, canon],
            )?;
            Ok(i64::from(n > 0))
        })
        .map(|n| n > 0)
    }

    /// Registered workspace folders, sorted.
    ///
    /// # Errors
    /// On a database error.
    pub fn workspaces(&self) -> Result<Vec<String>> {
        let c = self.reader()?;
        let mut st = c.prepare_cached("SELECT path FROM workspaces ORDER BY path")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

impl Drop for Knowledge {
    /// Close the channel and let the writer commit what is queued before the process goes on.
    fn drop(&mut self) {
        // The embedding thread finishes its current batch and lets go of the writer.
        if let Some(mut v) = self.vectors.take() {
            v.stop.store(true, Ordering::Relaxed);
            let _ = v.wake.try_send(());
            if let Some(t) = v.thread.take() {
                let _ = t.join();
            }
        }
        self.tx.take();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        self.stop.take();
        if let Some(c) = self.checkpointer.take() {
            let _ = c.join();
        }
    }
}

fn limit_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Distinct words of a query (letters and digits only, so no FTS5 operator survives).
pub fn terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for w in query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2)
    {
        if !terms.contains(&w) {
            terms.push(w);
        }
        if terms.len() == MAX_TERMS {
            break;
        }
    }
    terms
}

/// FTS5 query for `words`, whether to order by rank, and the highest id. A word in more than 5%
/// of the newest [`RECENT`] records (and more than [`COMMON_DOCS`]) is common, like a stop word:
/// rare words are OR-ed and ranked; with only common words, the rarest few must all match.
/// `cache` (the hot word index) keeps each word's count for [`WORD_TTL`] new records, so a query
/// repeating known words skips the frequency probes.
fn plan(
    c: &Connection,
    words: Vec<String>,
    cache: &mut HashMap<String, (i64, i64)>,
) -> Result<(String, bool, i64)> {
    let total: i64 = c
        .prepare_cached("SELECT coalesce(max(id), 0) FROM records")?
        .query_row([], |r| r.get(0))?;
    // Document frequency in the newest RECENT records, counted only up to the "common" line:
    // a stop word costs at most `cap` rowids, never its whole posting list.
    let window = total.min(RECENT);
    let cap = (window / 20).max(COMMON_DOCS) + 1;
    let mut df = c.prepare_cached(
        "SELECT count(*) FROM (SELECT 1 FROM records_fts
         WHERE records_fts MATCH ?1 AND rowid > ?3 LIMIT ?2)",
    )?;
    let (mut rare, mut common) = (Vec::new(), Vec::new());
    for w in words {
        let docs = match cache.get(&w) {
            Some(&(docs, at)) if total - at < WORD_TTL && at <= total => docs,
            _ => {
                let docs: i64 = df
                    .query_row(params![format!("\"{w}\""), cap, total - window], |r| {
                        r.get(0)
                    })?;
                if cache.len() >= WORD_CACHE {
                    cache.clear();
                }
                cache.insert(w.clone(), (docs, total));
                docs
            }
        };
        if docs >= cap {
            common.push((docs, w));
        } else {
            rare.push((docs, w));
        }
    }
    let quote = |(_, w): &(i64, String)| format!("\"{w}\"");
    // The rarest words say the most about a query and keep the ranked set small.
    rare.sort();
    common.sort();
    Ok(if rare.is_empty() {
        let all: Vec<String> = common.iter().take(4).map(quote).collect();
        (all.join(" AND "), false, total)
    } else {
        let any: Vec<String> = rare.iter().take(RANKED_TERMS).map(quote).collect();
        (any.join(" OR "), true, total)
    })
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir().map_or_else(|| PathBuf::from(p), |h| h.join(rest)),
        None if p == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}

fn expand(pattern: &str) -> Result<Vec<PathBuf>> {
    let path = expand_tilde(pattern.trim());
    let is_glob = |s: &str| s.contains(['*', '?', '[']);
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Ok(crate::canonical(&path)
            .into_iter()
            .filter(|p| p.is_dir())
            .collect());
    };
    if !is_glob(&name) {
        let p = crate::canonical(&path).map_err(|e| Error(format!("{pattern}: {e}")))?;
        return if p.is_dir() {
            Ok(vec![p])
        } else {
            Err(Error(format!("{pattern}: not a folder")))
        };
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    // shortcut: wildcards only in the last part (`dir/*`); upgrade to a walker if nested ones are asked for.
    if is_glob(&parent.to_string_lossy()) {
        return Err(Error(
            "only the last part of the path may contain wildcards".into(),
        ));
    }
    let glob = globset::Glob::new(&name)
        .map_err(|e| Error(e.to_string()))?
        .compile_matcher();
    let mut out: Vec<PathBuf> = fs::read_dir(parent)
        .map_err(|e| Error(format!("{}: {e}", parent.display())))?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir() && glob.is_match(e.file_name()))
        .filter_map(|e| crate::canonical(&e.path()).ok())
        .collect();
    out.sort();
    out.truncate(MAX_WORKSPACES);
    Ok(out)
}

/// `YYYY-MM-DD` (UTC) of a record timestamp.
pub fn date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// A folder name people can read (letters of any script kept), safe on every file system.
fn folder_name(s: &str) -> String {
    let name: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(80)
        .collect();
    let name = name.trim_matches(['.', ' ', '-']).to_owned();
    if name.is_empty() {
        "Genel".into()
    } else {
        name
    }
}

fn markdown_name(r: &Record) -> String {
    let mut slug = String::new();
    for c in r.task.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.chars().count() >= 50 {
            break;
        }
    }
    format!("{}-{}-{}.md", date(r.ts), r.id, slug.trim_end_matches('-'))
}

/// One record as Markdown with YAML front matter (Obsidian reads `tags` and `date`).
pub fn markdown(r: &Record) -> String {
    use std::fmt::Write as _;
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut out = format!(
        "---\nid: {}\ndate: {}\nproject: {}\nkind: {}\ntags: [kor, {}]\n---\n\n# {}\n\n{}\n",
        r.id,
        date(r.ts),
        q(&r.project),
        q(&r.kind),
        if r.kind.chars().all(|c| c.is_ascii_alphanumeric()) {
            r.kind.as_str()
        } else {
            "note"
        },
        r.task.replace('\n', " "),
        r.summary
    );
    for (title, items) in [
        ("Files", &r.files),
        ("Decisions", &r.decisions),
        ("Errors solved", &r.errors),
        ("Commands", &r.commands),
        ("Rules", &r.rules),
    ] {
        if !items.is_empty() {
            let _ = write!(out, "\n## {title}\n\n");
            for i in items {
                let _ = writeln!(out, "- {i}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(project: &str, task: &str, summary: &str) -> Record {
        Record {
            project: project.into(),
            task: task.into(),
            summary: summary.into(),
            files: vec!["src/auth.rs".into()],
            errors: vec!["E0502 borrow → clone the key".into()],
            ..Record::default()
        }
    }

    fn fts_sound(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.execute(
            "INSERT INTO records_fts(records_fts) VALUES ('integrity-check')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn writes_are_durable_first_and_indexed_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.sqlite");
        let kb = Knowledge::open(&path).unwrap();
        let batch: Vec<Record> = (0..1500)
            .map(|i| rec("/a", &format!("zebra task {i}"), "s"))
            .collect();
        kb.record_all(batch).unwrap();
        kb.commit().unwrap();
        assert_eq!(kb.count().unwrap(), 1500, "durable before it is indexed");
        // Forgetting records that may or may not be indexed yet keeps the index sound.
        for id in [1, 700, 1500] {
            assert!(kb.forget(id).unwrap());
        }
        kb.flush().unwrap();
        let hits = kb.search("zebra", Some("/a"), None, 200).unwrap();
        assert_eq!(hits.len(), 200);
        assert!(hits.iter().all(|h| ![1, 700, 1500].contains(&h.record.id)));
        drop(kb);
        fts_sound(&path);
        let c = Connection::open(&path).unwrap();
        let upto: i64 = c
            .query_row("SELECT upto FROM fts_state", [], |r| r.get(0))
            .unwrap();
        // Everything left is indexed (1500 may have been indexed before it was forgotten).
        assert!(upto >= 1499, "{upto}");
    }

    #[test]
    fn the_folder_view_follows_the_base_one_way() {
        let dir = tempfile::tempdir().unwrap();
        let kb = Knowledge::open(&dir.path().join("k.sqlite")).unwrap();
        kb.record(rec("/work/Şirket Sitesi", "login added", "s"))
            .unwrap();
        kb.flush().unwrap();
        let view = dir.path().join("Bilgi");
        std::fs::create_dir_all(&view).unwrap();
        std::fs::write(view.join("mine.md"), "the user's own note").unwrap();
        let (w, r) = kb.sync_folder(&view, None).unwrap();
        assert_eq!((w, r), (2, 0), "README and the note");
        let note = std::fs::read_dir(view.join("Şirket Sitesi").join("task"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            std::fs::read_to_string(&note)
                .unwrap()
                .contains("login added")
        );
        // Unchanged: nothing rewritten. Removed from the base: its file goes, the user's stays.
        assert_eq!(kb.sync_folder(&view, None).unwrap(), (0, 0));
        let id = kb.search("login", None, None, 1).unwrap()[0].record.id;
        kb.forget(id).unwrap();
        assert_eq!(kb.sync_folder(&view, None).unwrap(), (0, 1));
        assert!(!note.exists());
        assert!(view.join("mine.md").exists());
    }

    #[test]
    fn a_base_from_the_insert_trigger_era_keeps_its_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.sqlite");
        {
            // The schema before ADR 0062: an insert trigger indexed every row in its commit.
            let c = Connection::open(&path).unwrap();
            c.execute_batch(SCHEMA).unwrap();
            c.execute_batch(
                "CREATE TRIGGER records_ai AFTER INSERT ON records BEGIN
                    INSERT INTO records_fts(rowid, project, task, summary, files, decisions, errors, commands, rules)
                    VALUES (new.id, new.project, new.task, new.summary, new.files, new.decisions, new.errors, new.commands, new.rules);
                 END;",
            )
            .unwrap();
            insert(&c, &normalize(rec("/a", "old okapi record", "s"))).unwrap();
        }
        let kb = Knowledge::open(&path).unwrap();
        assert_eq!(kb.search("okapi", None, None, 5).unwrap().len(), 1);
        kb.record(rec("/a", "new okapi record", "s")).unwrap();
        kb.flush().unwrap();
        assert_eq!(kb.search("okapi", None, None, 5).unwrap().len(), 2);
        assert!(kb.forget(1).unwrap());
        drop(kb);
        fts_sound(&path);
    }

    #[test]
    fn write_search_boost_forget() {
        let dir = tempfile::tempdir().unwrap();
        let kb = Knowledge::open(&dir.path().join("knowledge.sqlite")).unwrap();
        kb.record(rec(
            "/a",
            "iyzico ödeme entegrasyonu",
            "Webhook imzası doğrulandı",
        ))
        .unwrap();
        kb.record(rec("/b", "iyzico refund", "Partial refunds via the v2 API"))
            .unwrap();
        kb.record(rec("/b", "Postgres migration", "password=hunter22 rotated"))
            .unwrap();
        kb.flush().unwrap();
        assert_eq!(kb.count().unwrap(), 3);

        let hits = kb.search("iyzico", None, Some("/b"), 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].record.project, "/b", "same project ranks first");
        let only = kb.search("IYZICO odeme", Some("/a"), None, 5).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].record.files, ["src/auth.rs"]);
        // FTS syntax in user text is just words.
        assert!(kb.search("\"NEAR( OR *", None, None, 5).unwrap().is_empty());
        assert!(kb.search("", None, None, 5).unwrap().is_empty());

        let pg = kb.search("postgres", None, None, 5).unwrap();
        let id = pg[0].record.id;
        assert!(
            !pg[0].record.summary.contains("hunter22"),
            "secrets are masked"
        );
        assert_eq!(kb.get(id).unwrap().unwrap().task, "Postgres migration");
        assert!(kb.forget(id).unwrap());
        assert!(!kb.forget(id).unwrap());
        assert!(kb.search("postgres", None, None, 5).unwrap().is_empty());
        assert_eq!(kb.recent(Some("/b"), 10).unwrap().len(), 1);
    }

    #[test]
    fn export_and_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let kb = Knowledge::open(&dir.path().join("k.sqlite")).unwrap();
        kb.record(rec(
            "/a",
            "Fix login: token refresh",
            "Refresh before expiry",
        ))
        .unwrap();
        kb.flush().unwrap();
        let md = dir.path().join("md");
        assert_eq!(kb.export(&md, ExportFormat::Markdown).unwrap(), 1);
        let file = fs::read_dir(&md).unwrap().next().unwrap().unwrap().path();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-1-fix-login-token-refresh.md"), "{name}");
        let text = fs::read_to_string(file).unwrap();
        assert!(
            text.starts_with("---\nid: 1\n") && text.contains("## Errors solved"),
            "{text}"
        );
        assert_eq!(
            kb.export(&dir.path().join("js"), ExportFormat::Json)
                .unwrap(),
            1
        );
        let json: Vec<Record> = serde_json::from_str(
            &fs::read_to_string(dir.path().join("js/knowledge.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json[0].task, "Fix login: token refresh");

        let repos = dir.path().join("repos");
        for r in ["one", "two"] {
            fs::create_dir_all(repos.join(r)).unwrap();
        }
        fs::write(repos.join("skip.txt"), "").unwrap();
        let added = kb.workspace_add(&format!("{}/*", repos.display())).unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(kb.workspaces().unwrap().len(), 2);
        assert!(
            kb.workspace_add(&format!("{}/*/x*", repos.display()))
                .is_err()
        );
        assert!(
            kb.workspace_remove(&repos.join("one").to_string_lossy())
                .unwrap()
        );
        assert!(
            !kb.workspace_remove(&repos.join("one").to_string_lossy())
                .unwrap()
        );
        assert_eq!(kb.workspaces().unwrap().len(), 1);
    }

    /// Deterministic embedder: words are mapped to concepts (so "ödeme" and "payment" agree),
    /// each concept is one dimension.
    struct Fake;

    impl Embed for Fake {
        fn model(&self) -> &'static str {
            "fake"
        }
        fn min_similarity(&self) -> f32 {
            0.3
        }
        fn embed(&self, texts: &[String], _: Kind) -> std::result::Result<Vec<Vec<f32>>, String> {
            let concept = |w: &str| match w {
                "payment" | "ödeme" | "iyzico" | "checkout" => Some(0),
                "login" | "oturum" | "auth" | "token" => Some(1),
                "database" | "postgres" | "veritabanı" | "migration" => Some(2),
                _ => None,
            };
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = [0.01f32; 4];
                    for w in terms(t) {
                        if let Some(i) = concept(&w) {
                            v[i] += 1.0;
                        }
                    }
                    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    v.iter().map(|x| x / n).collect()
                })
                .collect())
        }
    }

    #[test]
    fn hybrid_recall_finds_meaning_and_fuses_ranks() {
        let dir = tempfile::tempdir().unwrap();
        let kb = Knowledge::open_with(&dir.path().join("k.sqlite"), Some(Arc::new(Fake))).unwrap();
        assert!(kb.semantic());
        kb.record(rec("/a", "Checkout webhook", "iyzico signature verified"))
            .unwrap();
        kb.record(rec("/b", "Oturum yenileme", "token refresh before expiry"))
            .unwrap();
        kb.record(rec("/a", "Postgres migration", "added an index"))
            .unwrap();
        kb.record(rec("/b", "payment login page", "both words"))
            .unwrap();
        kb.flush().unwrap();
        kb.reindex(&|_, _| {}).unwrap();
        assert_eq!(kb.embedded().unwrap(), 4);

        // No record contains "ödeme": FTS alone finds nothing, the vectors find the payments.
        assert!(kb.search("ödeme", None, None, 5).unwrap().is_empty());
        let hits = kb.recall("ödeme", None, None, 5).unwrap();
        let tasks: Vec<&str> = hits.iter().map(|h| h.record.task.as_str()).collect();
        assert_eq!(tasks[0], "Checkout webhook", "{tasks:?}");
        assert!(tasks.contains(&"payment login page"), "{tasks:?}");
        assert!(
            !tasks.contains(&"Postgres migration"),
            "below the similarity floor"
        );

        // In both lists beats in one: "payment" is an FTS hit and a vector hit.
        let both = kb.recall("payment", None, None, 5).unwrap();
        assert_eq!(both[0].record.task, "payment login page");
        assert!(both[0].score > both[1].score);
        // Project filter and boost.
        let only = kb.recall("oturum", Some("/b"), None, 5).unwrap();
        assert!(only.iter().all(|h| h.record.project == "/b") && !only.is_empty());
        let boosted = kb.recall("ödeme", None, Some("/b"), 5).unwrap();
        assert_eq!(boosted[0].record.project, "/b");

        // Forget removes the vector from the table and the loaded index.
        let id = hits[0].record.id;
        assert!(kb.forget(id).unwrap());
        assert_eq!(kb.embedded().unwrap(), 3);
        assert!(
            kb.recall("ödeme", None, None, 5)
                .unwrap()
                .iter()
                .all(|h| h.record.id != id)
        );
    }

    #[test]
    fn new_records_are_embedded_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.sqlite");
        {
            let kb = Knowledge::open(&path).unwrap();
            kb.record(rec("/a", "Postgres migration", "old record"))
                .unwrap();
            assert!(kb.reindex(&|_, _| {}).is_err(), "no embedder");
            assert_eq!(kb.recall("veritabanı", None, None, 5).unwrap().len(), 0);
        }
        let kb = Knowledge::open_with(&path, Some(Arc::new(Fake))).unwrap();
        kb.record(rec("/a", "Login fix", "new record")).unwrap();
        kb.flush().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while kb.embedded().unwrap() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(kb.embedded().unwrap(), 2, "old and new records embedded");
        assert_eq!(
            kb.recall("veritabanı", None, None, 5).unwrap()[0]
                .record
                .task,
            "Postgres migration"
        );
    }

    #[test]
    fn rank_fusion_rewards_agreement() {
        let h = |id: i64, project: &str| Hit {
            score: 0.0,
            record: Record {
                id,
                project: project.into(),
                ..Record::default()
            },
        };
        let fused = fuse(
            vec![
                vec![h(1, "/a"), h(2, "/a"), h(3, "/a")],
                vec![h(3, "/a"), h(4, "/b")],
            ],
            None,
            10,
        );
        let ids: Vec<i64> = fused.iter().map(|h| h.record.id).collect();
        assert_eq!(ids, [3, 1, 4, 2]);
        let boosted = fuse(vec![vec![h(1, "/a"), h(2, "/b")]], Some("/b"), 1);
        assert_eq!(boosted[0].record.id, 2);
    }

    #[test]
    fn common_words_switch_to_all_of_them_newest_first() {
        assert_eq!(terms("Fix the-login! fix"), ["fix", "the", "login"]);
        assert!(terms("a ! ?").is_empty());
        let dir = tempfile::tempdir().unwrap();
        let kb = Knowledge::open(&dir.path().join("k.sqlite")).unwrap();
        for i in 0..1200 {
            let extra = if i % 400 == 0 { " zebra" } else { "" };
            kb.record(rec("/a", &format!("fix build {i}{extra}"), "error"))
                .unwrap();
        }
        kb.flush().unwrap();
        let c = kb.reader().unwrap();
        let plan_of = |q: &str| plan(&c, terms(q), &mut HashMap::new()).unwrap();
        assert_eq!(plan_of("fix zebra"), ("\"zebra\"".to_owned(), true, 1200));
        assert_eq!(
            plan_of("build fix"),
            ("\"build\" AND \"fix\"".to_owned(), false, 1200)
        );
        // The hot word index answers a second plan without probing; a cached count expires
        // after WORD_TTL new records.
        let mut cache = HashMap::new();
        let first = plan(&c, terms("build zebra"), &mut cache).unwrap();
        assert_eq!(cache["zebra"], (3, 1200));
        cache.insert("zebra".into(), (2000, 1200));
        assert!(
            !plan(&c, terms("build zebra"), &mut cache).unwrap().1,
            "cached count used"
        );
        cache.insert("zebra".into(), (2000, 1200 - WORD_TTL));
        assert_eq!(
            plan(&c, terms("build zebra"), &mut cache).unwrap(),
            first,
            "expired"
        );
        drop(c);
        let hits = kb.search("fix build", None, None, 3).unwrap();
        assert_eq!(hits[0].record.task, "fix build 1199", "newest first");
        assert_eq!(kb.search("zebra fix", None, None, 10).unwrap().len(), 3);
    }
}
