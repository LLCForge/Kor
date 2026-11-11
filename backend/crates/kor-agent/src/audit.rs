//! Append-only audit log with a hash chain (spec §6.7): `<kor home>/audit.jsonl`.
//!
//! Every line is one JSON object. `prev` is the `hash` of the line before it (64 zeros for the
//! very first) and `hash` is the SHA-256 of the line without its `hash` field (keys sorted), so
//! editing, deleting or reordering a line breaks the chain at that point ([`verify`]).
//! Values are integers, booleans and strings only (no floats; costs are decimal strings), so a
//! line re-serializes byte for byte. Writing is best effort: a full disk never stops the agent.
//!
//! Hardening (ADR 0036):
//! - Appends from several processes are serialized by an OS file lock (`audit.lock`).
//! - Size-based rotation: once `audit.jsonl` reaches [`ROTATE_BYTES`] it is renamed to
//!   `audit-000001.jsonl`, `audit-000002.jsonl`, …; the chain continues into the new file (its
//!   first `prev` is the archive's last `hash`) and [`verify`] walks every file in order.
//! - Head: `audit.head` holds the last hash and the total entry count, replaced atomically after
//!   every append. Lines removed from the end no longer verify against it (`truncated`).

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub const FILE: &str = "audit.jsonl";
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Longest text kept per field (prompts, command summaries).
pub const MAX_TEXT: usize = 2_000;
/// The active file is archived once it reaches this size.
pub const ROTATE_BYTES: u64 = 32 * 1024 * 1024;

static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where [`record`] writes; `None` turns auditing off (tests, tools that run without a home).
pub fn set_path(path: Option<PathBuf>) {
    *PATH.lock().unwrap_or_else(PoisonError::into_inner) = path;
}

pub fn path() -> Option<PathBuf> {
    PATH.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Cut `text` to [`MAX_TEXT`] characters.
pub fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

/// Append one entry to the configured log; errors are ignored (see module docs).
pub fn record(action: &str, fields: Value) {
    let Some(p) = path() else { return };
    let _guard = PATH.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = append(&p, action, fields);
}

/// Append `{ ...fields, ts, action, prev, hash }` to `path` and return the new hash.
pub fn append(path: &Path, action: &str, fields: Value) -> std::io::Result<String> {
    append_with(path, action, fields, ROTATE_BYTES)
}

/// [`append`] with the rotation size as a parameter (tests use a small one).
pub fn append_with(
    path: &Path,
    action: &str,
    fields: Value,
    rotate_at: u64,
) -> std::io::Result<String> {
    let mut map = match fields {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("data".into(), other);
            m
        }
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Held until the head is written: read tail, append and head are one step for every process.
    let _lock = kor_platform::lock_file(&path.with_extension("lock"))?;
    if std::fs::metadata(path).is_ok_and(|m| m.len() >= rotate_at) {
        let next = archives(path)?.last().map_or(1, |(n, _)| n + 1);
        std::fs::rename(path, archive_path(path, next))?;
    }
    let all = files(path)?;
    let mut prev = None;
    for f in all.iter().rev() {
        prev = last_hash(f)?;
        if prev.is_some() {
            break;
        }
    }
    let prev = prev.unwrap_or_else(|| GENESIS.to_owned());
    // The head normally tracks the tail; after an interrupted write (or for a log older than
    // the head file) the count is taken from the files once.
    let count = match read_head(path) {
        Some(h) if h.hash == prev => h.count,
        _ => count_entries(&all)?,
    } + 1;
    map.insert("ts".into(), json!(chrono::Utc::now().to_rfc3339()));
    map.insert("action".into(), json!(action));
    map.insert("prev".into(), json!(prev));
    map.remove("hash");
    let hash = digest(&map);
    map.insert("hash".into(), json!(hash));
    let mut line = Value::Object(map).to_string();
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())?;
    write_head(
        path,
        &Head {
            hash: hash.clone(),
            count,
        },
    )?;
    Ok(hash)
}

/// `audit.head`: the last hash and how many entries the whole log holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
struct Head {
    hash: String,
    count: usize,
}

fn read_head(path: &Path) -> Option<Head> {
    let text = std::fs::read_to_string(path.with_extension("head")).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_head(path: &Path, head: &Head) -> std::io::Result<()> {
    let tmp = path.with_extension("head.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string(head).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, path.with_extension("head"))
}

/// `audit-000007.jsonl` next to `audit.jsonl`.
fn archive_path(path: &Path, n: usize) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}-{n:06}.jsonl"))
}

/// Archived segments, oldest first.
fn archives(path: &Path) -> std::io::Result<Vec<(usize, PathBuf)>> {
    let Some(dir) = path.parent() else {
        return Ok(Vec::new());
    };
    let stem = format!(
        "{}-",
        path.file_stem().unwrap_or_default().to_string_lossy()
    );
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for e in entries {
        let name = e?.file_name().to_string_lossy().into_owned();
        if let Some(n) = name
            .strip_prefix(&stem)
            .and_then(|r| r.strip_suffix(".jsonl"))
            .filter(|n| n.len() == 6)
            .and_then(|n| n.parse::<usize>().ok())
        {
            out.push((n, path.with_file_name(name)));
        }
    }
    out.sort();
    Ok(out)
}

/// `first` (the oldest file present) is a later archive than `audit-000001.jsonl`: older
/// archives were removed. shortcut: removing every archive leaves `audit.jsonl` starting
/// mid-chain, which verifies as broken; retention keeps at least the newest archive.
fn starts_after_archives(path: &Path, first: &Path) -> bool {
    first != path && first != archive_path(path, 1)
}

/// Every file of the log in chain order: the archives, then the active file (if present).
pub fn files(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = archives(path)?.into_iter().map(|(_, p)| p).collect();
    if path.is_file() {
        out.push(path.to_owned());
    }
    Ok(out)
}

fn count_entries(files: &[PathBuf]) -> std::io::Result<usize> {
    let mut n = 0;
    for f in files {
        for line in BufReader::new(std::fs::File::open(f)?).lines() {
            if !line?.trim().is_empty() {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Same value with every object's keys in sorted order, so the hash does not depend on whether
/// `serde_json`'s `preserve_order` feature is on in the build that writes or checks the line.
fn canon(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canon(&m[k])))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        other => other.clone(),
    }
}

fn digest(body: &Map<String, Value>) -> String {
    let text = canon(&Value::Object(body.clone())).to_string();
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `hash` of the last line; `None` for a missing or empty file.
fn last_hash(path: &Path) -> std::io::Result<Option<String>> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    // Lines are a few KB at most; the tail holds the last one.
    let start = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(start))?;
    let mut tail = String::new();
    f.take(64 * 1024).read_to_string(&mut tail).ok();
    Ok(tail
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|v| v["hash"].as_str().map(str::to_owned)))
}

/// Result of [`verify`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verify {
    /// Lines checked across all files (blank lines are not counted).
    pub entries: usize,
    /// Files walked (archives and the active file).
    pub files: usize,
    /// First broken line (1-based within [`Verify::file`]) and why: `json`, `prev` or `hash`.
    pub broken: Option<(usize, &'static str)>,
    /// Name of the file that holds the broken line.
    pub file: Option<String>,
    /// The log against `audit.head`: `ok`; `behind` (the last write did not update the head:
    /// interrupted, harmless); `missing` (no head yet: a log written before it existed, or the
    /// head was deleted); `truncated` (the entry the head vouches for is gone or rewritten).
    pub head: &'static str,
    /// The oldest file does not start the chain: older archives were removed (retention).
    pub trimmed: bool,
}

impl Verify {
    pub fn ok(&self) -> bool {
        self.broken.is_none() && self.head != "truncated"
    }
}

/// Walk the chain through every file and compare the end with the head.
/// A missing log is an empty, intact one. Whole archives removed from the start (retention) are
/// accepted and reported as `trimmed`; a gap anywhere later breaks the chain.
pub fn verify(path: &Path) -> std::io::Result<Verify> {
    let all = files(path)?;
    let head = read_head(path);
    let mut prev: Option<String> = None;
    let mut trimmed = false;
    let mut entries = 0;
    // 0 = head hash not seen, else the entry number where it was seen.
    let mut head_at = 0;
    for f in &all {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for (i, line) in BufReader::new(std::fs::File::open(f)?).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            entries += 1;
            let at = trimmed;
            let broken = |why| {
                Ok(Verify {
                    entries,
                    files: all.len(),
                    broken: Some((i + 1, why)),
                    file: Some(name.clone()),
                    head: "ok",
                    trimmed: at,
                })
            };
            let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(&line) else {
                return broken("json");
            };
            let Some(Value::String(hash)) = map.remove("hash") else {
                return broken("hash");
            };
            let line_prev = map.get("prev").and_then(Value::as_str);
            match &prev {
                Some(p) if line_prev != Some(p.as_str()) => return broken("prev"),
                Some(_) => {}
                // Only a log whose first archive is gone may start mid-chain.
                None if line_prev == Some(GENESIS) => {}
                None if starts_after_archives(path, f) => trimmed = true,
                None => return broken("prev"),
            }
            if digest(&map) != hash {
                return broken("hash");
            }
            if head.as_ref().is_some_and(|h| h.hash == hash) {
                head_at = entries;
            }
            prev = Some(hash);
        }
    }
    let head = match head {
        None if entries == 0 => "ok",
        None => "missing",
        Some(_) if head_at == 0 => "truncated",
        Some(_) if head_at < entries => "behind",
        Some(_) => "ok",
    };
    Ok(Verify {
        entries,
        files: all.len(),
        broken: None,
        file: None,
        head,
        trimmed,
    })
}

/// Call `f` with every parsed entry, oldest first, across all files. Unparsable lines are
/// skipped (verify reports them).
pub fn each(path: &Path, mut f: impl FnMut(Value)) -> std::io::Result<()> {
    for file in files(path)? {
        for line in BufReader::new(std::fs::File::open(file)?).lines() {
            if let Ok(v) = serde_json::from_str::<Value>(&line?) {
                f(v);
            }
        }
    }
    Ok(())
}

/// Filters for [`read`]; empty fields match everything.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Exact `action` (`tool`, `task`, `model`, …).
    pub action: Option<String>,
    /// Substring of `model`.
    pub model: Option<String>,
    /// Substring of `target` (file path or command).
    pub file: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

/// One page of entries, newest first, and the number of matches.
#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub total: usize,
    pub entries: Vec<Value>,
    /// Distinct actions and models in the whole log, for filter menus.
    pub actions: Vec<String>,
    pub models: Vec<String>,
}

/// shortcut: scans every file per page; fine for local logs of a few hundred MB, index if
/// the team server ever serves big central logs from here.
pub fn read(path: &Path, q: &Query) -> std::io::Result<Page> {
    let has = |field: &Value, needle: &Option<String>| match needle.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(n) => field
            .as_str()
            .is_some_and(|v| v.to_lowercase().contains(&n.to_lowercase())),
    };
    let mut hits = Vec::new();
    let mut actions = std::collections::BTreeSet::new();
    let mut models = std::collections::BTreeSet::new();
    each(path, |v| {
        if let Some(a) = v["action"].as_str() {
            actions.insert(a.to_owned());
        }
        if let Some(m) = v["model"].as_str() {
            models.insert(m.to_owned());
        }
        let action_ok = q
            .action
            .as_deref()
            .filter(|a| !a.is_empty())
            .is_none_or(|a| v["action"].as_str() == Some(a));
        if action_ok && has(&v["model"], &q.model) && has(&v["target"], &q.file) {
            hits.push(v);
        }
    })?;
    let total = hits.len();
    let entries = hits
        .into_iter()
        .rev()
        .skip(q.offset)
        .take(q.limit.clamp(1, 500))
        .collect();
    Ok(Page {
        total,
        entries,
        actions: actions.into_iter().collect(),
        models: models.into_iter().collect(),
    })
}

/// What kind of action a tool call is, for the compliance report (§6.7: files read/written,
/// commands run, data sent off the machine by a tool).
pub fn tool_kind(tool: &str) -> &'static str {
    match tool {
        "Read" | "Glob" | "Grep" | "list" | "LSP" | "OCR" => "read",
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch" | "Delete" | "Move" => {
            "write"
        }
        "Bash" | "PowerShell" | "RunElevated" => "command",
        "WebFetch" | "WebSearch" => "network",
        t if ["mcp__", "browser_", "chrome_", "ssh_", "db_"]
            .iter()
            .any(|p| t.starts_with(p)) =>
        {
            "network"
        }
        _ => "tool",
    }
}

/// `0.001234` (6 decimals) for the `cost_usd` field; floats are not hashed.
pub fn usd(cost: f64) -> String {
    format!("{cost:.6}")
}

/// What the agent loop records. Text goes through the session's redactor first.
impl crate::agent::Agent {
    fn audit_model(&self) -> String {
        format!("{}/{}", self.provider.id(), self.model)
    }

    /// One tool call: `allow`/`deny` by rule, `approved`/`rejected` by the user, and its result.
    pub(crate) fn audit_tool(
        &self,
        tool: &str,
        summary: &str,
        decision: crate::permissions::Decision,
        allowed: bool,
        ok: Option<bool>,
    ) {
        if path().is_none() {
            return;
        }
        let asked = decision == crate::permissions::Decision::Ask;
        let decision = match (asked, allowed) {
            (true, true) => "approved",
            (true, false) => "rejected",
            (false, true) => "allow",
            (false, false) => "deny",
        };
        let mut v = json!({
            "tool": tool,
            "kind": tool_kind(tool),
            "target": clip(&self.redactor.redact(summary)),
            "decision": decision,
            "model": self.audit_model(),
            "root": self.ctx.root.to_string_lossy(),
        });
        if let Some(ok) = ok {
            v["ok"] = json!(ok);
        }
        record("tool", v);
    }

    /// One model call: tokens, its cost when the ledger knows it (`cost_estimated` when it
    /// comes from catalog prices, shown with ≈), the approximate request size and how many
    /// secrets the request builder masked (approximate when sub-agents call in parallel).
    pub(crate) fn audit_call(
        &self,
        usage: &kor_core::Usage,
        cost: Option<(f64, bool)>,
        bytes: u64,
        masked: u64,
    ) {
        if path().is_none() {
            return;
        }
        let mut v = json!({
            "model": self.audit_model(),
            "provider": self.provider.id(),
            "input": usage.input,
            "output": usage.output,
            "bytes": bytes,
            "masked": masked,
            "root": self.ctx.root.to_string_lossy(),
        });
        if let Some((c, estimated)) = cost {
            v["cost_usd"] = json!(usd(c));
            v["cost_estimated"] = json!(estimated);
        }
        record("model", v);
    }

    /// One user message: the prompt, the model and the tokens it used.
    pub(crate) fn audit_task(
        &self,
        prompt: &str,
        before: kor_core::Usage,
        result: &Result<crate::agent::StopReason, kor_core::ProviderError>,
    ) {
        if path().is_none() {
            return;
        }
        let outcome = match result {
            Ok(crate::agent::StopReason::Done(_)) => "done",
            Ok(crate::agent::StopReason::Cancelled) => "cancelled",
            Ok(crate::agent::StopReason::MaxTurns) => "max_turns",
            Ok(crate::agent::StopReason::Budget) => "budget",
            Ok(crate::agent::StopReason::Blocked(_)) => "blocked",
            Err(_) => "error",
        };
        record(
            "task",
            json!({
                "text": clip(&self.redactor.redact(prompt)),
                "model": self.audit_model(),
                "root": self.ctx.root.to_string_lossy(),
                "outcome": outcome,
                "input": self.usage.input.saturating_sub(before.input),
                "output": self.usage.output.saturating_sub(before.output),
            }),
        );
    }
}

/// Per model: calls, tokens, request bytes and cost.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelUse {
    pub model: String,
    pub provider: String,
    pub calls: u64,
    pub input: u64,
    pub output: u64,
    pub bytes: u64,
    /// Sum of the calls whose cost is known.
    pub cost_usd: f64,
    /// Some of that cost comes from catalog prices, not the provider.
    pub cost_estimated: bool,
    /// Calls without a known cost.
    pub cost_unknown: u64,
}

/// Per provider; the runtime fills in where it runs and its data policy from the catalog.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProviderUse {
    pub provider: String,
    pub calls: u64,
    pub bytes: u64,
    pub input: u64,
    pub output: u64,
    /// Runs on this machine (nothing left it); `None` when the provider is not in the catalog.
    pub local: Option<bool>,
    pub jurisdiction: Option<String>,
    pub training_on_data: Option<bool>,
    pub retention: Option<String>,
}

/// Where the log lives and how big it is.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Retention {
    pub files: usize,
    pub bytes: u64,
    pub rotate_bytes: u64,
    /// Oldest entry still in the log.
    pub oldest: Option<String>,
}

/// The KVKK/GDPR compliance summary (§6.7) built from the log: what was done with which data,
/// never the data itself (no prompts, no file contents).
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub since: Option<String>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub entries: usize,
    pub tasks: usize,
    pub models: Vec<ModelUse>,
    pub providers: Vec<ProviderUse>,
    pub files_read: Vec<String>,
    pub files_written: Vec<String>,
    pub reads: u64,
    pub writes: u64,
    pub commands: u64,
    pub network_tools: u64,
    pub approved: u64,
    pub rejected: u64,
    pub denied: u64,
    pub masked: u64,
    pub sandbox_denied: u64,
    pub verify: Verify,
    pub retention: Retention,
}

/// Longest list of distinct file targets kept in a report (the counts stay exact).
pub const REPORT_FILES: usize = 200;

/// Build the report for entries at or after `since` (`YYYY-MM-DD` or RFC 3339, UTC).
#[allow(clippy::too_many_lines)]
pub fn report(path: &Path, since: Option<&str>) -> std::io::Result<Report> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut r = Report {
        since: since.map(str::to_owned),
        first: None,
        last: None,
        entries: 0,
        tasks: 0,
        models: Vec::new(),
        providers: Vec::new(),
        files_read: Vec::new(),
        files_written: Vec::new(),
        reads: 0,
        writes: 0,
        commands: 0,
        network_tools: 0,
        approved: 0,
        rejected: 0,
        denied: 0,
        masked: 0,
        sandbox_denied: 0,
        verify: verify(path)?,
        retention: Retention {
            rotate_bytes: ROTATE_BYTES,
            ..Retention::default()
        },
    };
    for f in files(path)? {
        r.retention.files += 1;
        r.retention.bytes += std::fs::metadata(&f).map_or(0, |m| m.len());
    }
    let mut models: BTreeMap<String, ModelUse> = BTreeMap::new();
    let (mut read, mut written) = (BTreeSet::new(), BTreeSet::new());
    each(path, |v| {
        let ts = v["ts"].as_str().unwrap_or_default();
        if r.retention.oldest.is_none() && !ts.is_empty() {
            r.retention.oldest = Some(ts.to_owned());
        }
        if since.is_some_and(|s| ts < s) {
            return;
        }
        r.entries += 1;
        if r.first.is_none() {
            r.first = Some(ts.to_owned());
        }
        r.last = Some(ts.to_owned());
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        match v["action"].as_str().unwrap_or_default() {
            "task" => r.tasks += 1,
            "model" => {
                let name = v["model"].as_str().unwrap_or_default().to_owned();
                let m = models.entry(name.clone()).or_insert_with(|| ModelUse {
                    provider: v["provider"].as_str().map_or_else(
                        || name.split('/').next().unwrap_or_default().to_owned(),
                        str::to_owned,
                    ),
                    model: name,
                    ..ModelUse::default()
                });
                m.calls += 1;
                m.input += n("input");
                m.output += n("output");
                m.bytes += n("bytes");
                match v["cost_usd"].as_str().and_then(|c| c.parse::<f64>().ok()) {
                    Some(c) => {
                        m.cost_usd += c;
                        m.cost_estimated |= v["cost_estimated"].as_bool() == Some(true);
                    }
                    None => m.cost_unknown += 1,
                }
                r.masked += n("masked");
            }
            "tool" => {
                let tool = v["tool"].as_str().unwrap_or_default();
                let kind = v["kind"].as_str().unwrap_or_else(|| tool_kind(tool));
                let target = v["target"].as_str().unwrap_or_default().to_owned();
                let ran = !matches!(v["decision"].as_str(), Some("deny" | "rejected"));
                match v["decision"].as_str() {
                    Some("approved") => r.approved += 1,
                    Some("rejected") => r.rejected += 1,
                    Some("deny") => r.denied += 1,
                    _ => {}
                }
                if ran {
                    match kind {
                        "read" => {
                            r.reads += 1;
                            read.insert(target);
                        }
                        "write" => {
                            r.writes += 1;
                            written.insert(target);
                        }
                        "command" => r.commands += 1,
                        "network" => r.network_tools += 1,
                        _ => {}
                    }
                }
            }
            "sandbox_denied" => r.sandbox_denied += 1,
            _ => {}
        }
    })?;
    let mut providers: BTreeMap<String, ProviderUse> = BTreeMap::new();
    for m in models.values() {
        let p = providers
            .entry(m.provider.clone())
            .or_insert_with(|| ProviderUse {
                provider: m.provider.clone(),
                ..ProviderUse::default()
            });
        p.calls += m.calls;
        p.bytes += m.bytes;
        p.input += m.input;
        p.output += m.output;
    }
    r.models = models.into_values().collect();
    r.providers = providers.into_values().collect();
    r.files_read = read.into_iter().take(REPORT_FILES).collect();
    r.files_written = written.into_iter().take(REPORT_FILES).collect();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_verifies_and_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        assert!(verify(&p).unwrap().ok());
        let h1 = append(
            &p,
            "tool",
            json!({"tool": "Read", "target": "src/a.rs", "model": "m1"}),
        )
        .unwrap();
        append(
            &p,
            "tool",
            json!({"tool": "Bash", "target": "cargo test", "model": "m2", "ok": true}),
        )
        .unwrap();
        append(
            &p,
            "task",
            json!({"model": "m1", "text": "ünicode ✓ prompt", "input": 12}),
        )
        .unwrap();
        let v = verify(&p).unwrap();
        assert_eq!((v.entries, v.broken, v.head), (3, None, "ok"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.lines().nth(1).unwrap().contains(&h1));

        // Edit a value in the middle: that line's hash no longer matches.
        std::fs::write(&p, text.replacen("cargo test", "rm -rf /", 1)).unwrap();
        let v = verify(&p).unwrap();
        assert_eq!(
            (v.broken, v.file.as_deref()),
            (Some((2, "hash")), Some(FILE))
        );
        // Delete the first line: the next one points at a missing predecessor.
        let rest: String = text.lines().skip(1).flat_map(|l| [l, "\n"]).collect();
        std::fs::write(&p, rest).unwrap();
        assert_eq!(verify(&p).unwrap().broken, Some((1, "prev")));
        std::fs::write(&p, "not json\n").unwrap();
        assert_eq!(verify(&p).unwrap().broken, Some((1, "json")));
    }

    #[test]
    fn head_detects_tail_truncation_and_recovers_from_a_stale_head() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        for i in 0..4 {
            append(&p, "task", json!({"input": i})).unwrap();
        }
        assert_eq!(verify(&p).unwrap().head, "ok");
        let text = std::fs::read_to_string(&p).unwrap();
        let first3: String = text.lines().take(3).flat_map(|l| [l, "\n"]).collect();
        std::fs::write(&p, &first3).unwrap();
        let v = verify(&p).unwrap();
        assert_eq!((v.broken, v.head, v.ok()), (None, "truncated", false));
        // Truncating and appending again does not hide it either: the head no longer matches
        // the entry at its position.
        std::fs::write(&p, &text).unwrap();
        let head = std::fs::read_to_string(p.with_extension("head")).unwrap();
        std::fs::write(&p, &first3).unwrap();
        append(&p, "task", json!({})).unwrap();
        std::fs::write(p.with_extension("head"), &head).unwrap();
        assert_eq!(verify(&p).unwrap().head, "truncated");

        // A head older than the log (interrupted write) is only "behind", and the next append
        // recounts and fixes it.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        append(&p, "task", json!({})).unwrap();
        let stale = std::fs::read_to_string(p.with_extension("head")).unwrap();
        append(&p, "task", json!({})).unwrap();
        std::fs::write(p.with_extension("head"), &stale).unwrap();
        assert_eq!(verify(&p).unwrap().head, "behind");
        assert!(verify(&p).unwrap().ok());
        append(&p, "task", json!({})).unwrap();
        assert_eq!(verify(&p).unwrap().head, "ok");
        std::fs::remove_file(p.with_extension("head")).unwrap();
        assert_eq!(verify(&p).unwrap().head, "missing");
    }

    #[test]
    fn rotation_keeps_one_chain_across_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        for i in 0..7 {
            append_with(
                &p,
                "tool",
                json!({"tool": "Read", "target": format!("f{i}")}),
                300,
            )
            .unwrap();
        }
        let all = files(&p).unwrap();
        assert!(all.len() >= 4, "{all:?}");
        assert_eq!(all.last().unwrap(), &p);
        assert!(all[0].ends_with("audit-000001.jsonl"));
        let v = verify(&p).unwrap();
        assert_eq!((v.entries, v.files, v.ok()), (7, all.len(), true));
        let page = read(
            &p,
            &Query {
                limit: 10,
                ..Query::default()
            },
        )
        .unwrap();
        assert_eq!(page.total, 7);
        assert_eq!(page.entries[0]["target"], "f6");
        // Retention: removing the oldest archive is accepted and reported.
        std::fs::remove_file(&all[0]).unwrap();
        let v = verify(&p).unwrap();
        assert_eq!((v.ok(), v.trimmed, v.head), (true, true, "ok"));
        // A gap in the middle breaks the chain where the next file starts.
        std::fs::remove_file(&all[2]).unwrap();
        let v = verify(&p).unwrap();
        assert_eq!(v.broken.map(|b| b.1), Some("prev"));
    }

    #[test]
    fn concurrent_appends_keep_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        append(&p, "task", json!({"t": t, "i": i})).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let v = verify(&p).unwrap();
        assert_eq!((v.entries, v.broken, v.head), (40, None, "ok"));
    }

    #[test]
    fn report_counts_actions_costs_and_files_without_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let call = |model: &str, cost: Option<(&str, bool)>, masked: u64| {
            let mut v = json!({"model": model, "provider": model.split('/').next().unwrap(),
                               "input": 100, "output": 20, "bytes": 4000, "masked": masked});
            if let Some((c, est)) = cost {
                v["cost_usd"] = json!(c);
                v["cost_estimated"] = json!(est);
            }
            append(&p, "model", v).unwrap();
        };
        append(
            &p,
            "task",
            json!({"text": "secret prompt text", "model": "openai/gpt"}),
        )
        .unwrap();
        call("openai/gpt", Some(("0.010000", false)), 2);
        call("openai/gpt", Some(("0.005000", true)), 0);
        call("ollama/llama", None, 0);
        let tool = |tool: &str, target: &str, decision: &str| {
            append(
                &p,
                "tool",
                json!({"tool": tool, "target": target, "decision": decision}),
            )
            .unwrap();
        };
        tool("Read", "src/a.rs", "allow");
        tool("Read", "src/a.rs", "allow");
        tool("Edit", "src/b.rs", "approved");
        tool("Write", "src/c.rs", "rejected");
        tool("Bash", "cargo test", "allow");
        tool("Bash", "rm -rf /", "deny");
        tool("WebFetch", "https://example.com", "allow");
        append(&p, "sandbox_denied", json!({"target": "/etc/passwd"})).unwrap();

        let r = report(&p, None).unwrap();
        assert_eq!((r.entries, r.tasks), (12, 1));
        assert_eq!(r.files_read, ["src/a.rs"]);
        assert_eq!(r.files_written, ["src/b.rs"]);
        assert_eq!(
            (r.reads, r.writes, r.commands, r.network_tools),
            (2, 1, 1, 1)
        );
        assert_eq!((r.approved, r.rejected, r.denied), (1, 1, 1));
        assert_eq!((r.masked, r.sandbox_denied), (2, 1));
        let gpt = r.models.iter().find(|m| m.model == "openai/gpt").unwrap();
        assert_eq!((gpt.calls, gpt.bytes, gpt.cost_estimated), (2, 8000, true));
        assert!((gpt.cost_usd - 0.015).abs() < 1e-9);
        let llama = r.models.iter().find(|m| m.provider == "ollama").unwrap();
        assert_eq!(llama.cost_unknown, 1);
        assert_eq!(r.providers.len(), 2);
        assert!(r.verify.ok());
        assert!(!serde_json::to_string(&r).unwrap().contains("secret prompt"));

        let none = report(&p, Some("2999-01-01")).unwrap();
        assert_eq!((none.entries, none.retention.files), (0, 1));
        assert!(none.retention.oldest.is_some());
    }

    #[test]
    fn read_pages_newest_first_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        for i in 0..5 {
            let model = if i % 2 == 0 {
                "openai/gpt"
            } else {
                "local/llama"
            };
            append(
                &p,
                "tool",
                json!({"tool": "Edit", "target": format!("src/f{i}.rs"), "model": model}),
            )
            .unwrap();
        }
        append(&p, "task", json!({"model": "openai/gpt"})).unwrap();
        let all = read(
            &p,
            &Query {
                limit: 2,
                ..Query::default()
            },
        )
        .unwrap();
        assert_eq!(all.total, 6);
        assert_eq!(all.entries.len(), 2);
        assert_eq!(all.entries[0]["action"], "task");
        assert_eq!(all.actions, ["task", "tool"]);
        assert_eq!(all.models, ["local/llama", "openai/gpt"]);
        let q = Query {
            action: Some("tool".into()),
            model: Some("GPT".into()),
            offset: 1,
            limit: 10,
            ..Query::default()
        };
        let page = read(&p, &q).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.entries[0]["target"], "src/f2.rs");
        let f = read(
            &p,
            &Query {
                file: Some("f4".into()),
                limit: 10,
                ..Query::default()
            },
        )
        .unwrap();
        assert_eq!(f.total, 1);
        assert_eq!(
            read(&dir.path().join("none"), &Query::default())
                .unwrap()
                .total,
            0
        );
    }

    #[test]
    fn clip_is_char_safe() {
        assert_eq!(clip("kısa"), "kısa");
        let long = "ğ".repeat(MAX_TEXT + 5);
        assert_eq!(clip(&long).chars().count(), MAX_TEXT + 1);
    }

    #[test]
    fn tool_kinds() {
        assert_eq!(tool_kind("Read"), "read");
        assert_eq!(tool_kind("apply_patch"), "write");
        assert_eq!(tool_kind("PowerShell"), "command");
        assert_eq!(tool_kind("mcp__github__x"), "network");
        assert_eq!(tool_kind("TodoWrite"), "tool");
        assert_eq!(usd(1.5), "1.500000");
    }
}
