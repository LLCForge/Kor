//! MCP developer mode (Master Build Prompt §28.15, ADR 0063): `kor mcp dev ./server` runs an MCP
//! server under development, reloads it when its files change, shows what changed in its tool
//! schemas, lets the developer call tools / read resources / get prompts by hand, and logs every
//! JSON-RPC message both ways with errors explained. The desktop's MCP inspector panel and the
//! TUI use the same session.
//!
//! The server is started like any stdio MCP server (newline-delimited JSON-RPC 2.0 on
//! stdin/stdout); its stderr is shown as log lines. A line on stdout that is not JSON breaks the
//! protocol, so it is logged as a problem with an explanation.

use kor_platform::NoWindow as _;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

const PROTOCOL: &str = "2025-06-18";
const LOG_KEPT: usize = 2_000;
const MAX_LINE: usize = 16 * 1024 * 1024;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// How to start the server under development.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Watched for changes (the file or folder given).
    pub watch: PathBuf,
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// The command for a server file or folder: `.js/.mjs/.cjs` → node, `.ts` → `npx tsx`, `.py` →
/// python, a folder with `package.json` (its `main`/`bin`), `Cargo.toml` (`cargo run -q`),
/// `pyproject.toml` / `server.py` / `main.py`, or `index.js`.
///
/// # Errors
/// When nothing runnable is found (give the command after `--`).
pub fn detect(path: &Path) -> Result<Target, String> {
    let abs = kor_tools::without_verbatim(
        &path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?,
    );
    let py = if which("python3") {
        "python3"
    } else {
        "python"
    };
    let file = |f: &Path, cwd: &Path| -> Option<Target> {
        let ext = f.extension()?.to_str()?.to_ascii_lowercase();
        let s = f.to_string_lossy().into_owned();
        let (command, args): (&str, Vec<String>) = match ext.as_str() {
            "js" | "mjs" | "cjs" => ("node", vec![s]),
            "ts" | "mts" => ("npx", vec!["-y".into(), "tsx".into(), s]),
            "py" => (py, vec![s]),
            _ => return None,
        };
        Some(Target {
            command: command.into(),
            args,
            cwd: cwd.to_path_buf(),
            watch: abs.clone(),
        })
    };
    if abs.is_file() {
        let cwd = abs.parent().unwrap_or(&abs).to_path_buf();
        return file(&abs, &cwd).ok_or_else(|| {
            format!(
                "{}: use a .js, .ts or .py file, a project folder, or `-- <command>`",
                path.display()
            )
        });
    }
    let d = &abs;
    if let Some(pkg) = read_json(&d.join("package.json")) {
        let bin = match &pkg["bin"] {
            Value::String(b) => Some(b.clone()),
            Value::Object(m) => m.values().next().and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        let main = bin.or_else(|| pkg["main"].as_str().map(str::to_owned));
        if let Some(m) = main
            && let Some(t) = file(&d.join(m), d)
        {
            return Ok(t);
        }
        if pkg["scripts"]["start"].is_string() {
            return Ok(Target {
                command: "npm".into(),
                args: vec!["run".into(), "--silent".into(), "start".into()],
                cwd: d.clone(),
                watch: d.clone(),
            });
        }
    }
    if d.join("Cargo.toml").is_file() {
        return Ok(Target {
            command: "cargo".into(),
            args: vec!["run".into(), "-q".into()],
            cwd: d.clone(),
            watch: d.join("src"),
        });
    }
    for f in [
        "server.py",
        "main.py",
        "src/server.py",
        "index.js",
        "server.js",
        "index.ts",
        "src/index.ts",
    ] {
        if d.join(f).is_file()
            && let Some(t) = file(&d.join(f), d)
        {
            return Ok(Target {
                watch: d.clone(),
                ..t
            });
        }
    }
    if d.join("pyproject.toml").is_file() {
        let name = d
            .file_name()
            .map(|n| n.to_string_lossy().replace('-', "_"))
            .unwrap_or_default();
        return Ok(Target {
            command: py.into(),
            args: vec!["-m".into(), name],
            cwd: d.clone(),
            watch: d.clone(),
        });
    }
    Err(format!(
        "{}: no server entry found; give the command after `--`",
        path.display()
    ))
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join(cmd).is_file() || d.join(format!("{cmd}.exe")).is_file())
    })
}

/// Direction of one logged message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    /// Kor → server.
    Out,
    /// Server → Kor.
    In,
    /// The server's stderr.
    Stderr,
    /// A protocol problem or a lifecycle note (started, exited, reloaded).
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Traffic {
    /// Milliseconds since the session started.
    pub ms: u64,
    pub dir: Dir,
    pub text: String,
}

/// A JSON-RPC error answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<String>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}): {}",
            self.message,
            self.code,
            explain(self.code)
        )
    }
}

/// What a JSON-RPC / MCP error code means and what usually causes it.
pub fn explain(code: i64) -> &'static str {
    match code {
        -32700 => "parse error: the server received text that is not JSON",
        -32600 => "invalid request: the message is not a valid JSON-RPC request",
        -32601 => {
            "method not found: the server does not implement this method (check its capabilities)"
        }
        -32602 => "invalid params: the arguments do not match the tool's inputSchema",
        -32603 => "internal error: the server threw while handling the call (see its stderr)",
        -32002 => "resource not found",
        -32800 => "request cancelled",
        -32001 => "request timed out on the server",
        c if (-32099..=-32000).contains(&c) => {
            "server error: implementation-defined (see its stderr)"
        }
        _ => "application error returned by the server",
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;
type Writer = Arc<tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>>;
/// Called for every logged message (live view).
pub type Sink = Arc<dyn Fn(&Traffic) + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One running server with its traffic log.
pub struct Session {
    writer: Writer,
    pending: Pending,
    next: AtomicU64,
    log: Arc<Mutex<VecDeque<Traffic>>>,
    sink: Sink,
    started: std::time::Instant,
    child: Mutex<Option<tokio::process::Child>>,
    /// `initialize` result: server info and capabilities.
    pub server: Value,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

fn push(
    log: &Mutex<VecDeque<Traffic>>,
    sink: &Sink,
    started: std::time::Instant,
    dir: Dir,
    text: String,
) {
    let t = Traffic {
        ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        dir,
        text,
    };
    sink(&t);
    let mut l = lock(log);
    if l.len() == LOG_KEPT {
        l.pop_front();
    }
    l.push_back(t);
}

impl Session {
    /// Start `target` and complete `initialize`.
    ///
    /// # Errors
    /// The process cannot start or the handshake fails.
    pub async fn start(target: &Target, sink: Sink) -> Result<Self, String> {
        let mut cmd = tokio::process::Command::new(&target.command);
        cmd.no_window();
        cmd.args(&target.args)
            .current_dir(&target.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start `{}`: {e}", target.command))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take();
        let s = Self::from_streams(stdout, stdin, sink, Some(child));
        if let Some(e) = stderr {
            let (log, sink, started) = (s.log.clone(), s.sink.clone(), s.started);
            tokio::spawn(async move {
                let mut lines = BufReader::new(e).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    push(&log, &sink, started, Dir::Stderr, l);
                }
            });
        }
        s.note(format!(
            "started: {} {}",
            target.command,
            target.args.join(" ")
        ));
        s.initialize().await
    }

    /// A session over any streams (tests, or a server already running).
    pub fn from_streams(
        read: impl AsyncRead + Send + Unpin + 'static,
        write: impl AsyncWrite + Send + Unpin + 'static,
        sink: Sink,
        child: Option<tokio::process::Child>,
    ) -> Self {
        let s = Self {
            writer: Arc::new(tokio::sync::Mutex::new(Box::new(write))),
            pending: Arc::default(),
            next: AtomicU64::new(1),
            log: Arc::default(),
            sink,
            started: std::time::Instant::now(),
            child: Mutex::new(child),
            server: Value::Null,
        };
        let (pending, log, sink, started) =
            (s.pending.clone(), s.log.clone(), s.sink.clone(), s.started);
        tokio::spawn(async move {
            let mut r = BufReader::new(read);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = line.trim_end().to_owned();
                if text.is_empty() {
                    continue;
                }
                if text.len() > MAX_LINE {
                    push(
                        &log,
                        &sink,
                        started,
                        Dir::Note,
                        "a message over 16 MB was dropped".into(),
                    );
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    push(
                        &log,
                        &sink,
                        started,
                        Dir::Note,
                        format!(
                            "not JSON on stdout (this breaks the protocol; log to stderr instead): {}",
                            text.chars().take(300).collect::<String>()
                        ),
                    );
                    continue;
                };
                push(&log, &sink, started, Dir::In, text);
                let Some(id) = v.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                if v.get("method").is_some() {
                    continue; // a request from the server (sampling, roots): logged only
                }
                let tx = lock(&pending).remove(&id);
                if let Some(tx) = tx {
                    let r = match v.get("error") {
                        Some(e) => Err(RpcError {
                            code: e["code"].as_i64().unwrap_or(0),
                            message: e["message"].as_str().unwrap_or("error").to_owned(),
                            data: e.get("data").map(Value::to_string),
                        }),
                        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = tx.send(r);
                }
            }
            push(
                &log,
                &sink,
                started,
                Dir::Note,
                "server closed its output (exited)".into(),
            );
            lock(&pending).clear();
        });
        s
    }

    fn note(&self, text: String) {
        push(&self.log, &self.sink, self.started, Dir::Note, text);
    }

    async fn send(&self, body: &Value) -> Result<(), String> {
        let text = body.to_string();
        push(&self.log, &self.sink, self.started, Dir::Out, text.clone());
        let mut w = self.writer.lock().await;
        w.write_all(format!("{text}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        w.flush().await.map_err(|e| e.to_string())
    }

    /// One request and its result.
    ///
    /// # Errors
    /// The server's error (explained), a closed pipe or a timeout.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(e.to_string()),
            Ok(Err(_)) => Err("the server exited before answering (see its stderr)".into()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(format!("no answer within {} s", timeout.as_secs()))
            }
        }
    }

    async fn initialize(mut self) -> Result<Self, String> {
        let r = self
            .request(
                "initialize",
                json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "kor-mcp-dev", "version": kor_core::VERSION}}),
                Duration::from_secs(30),
            )
            .await?;
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}))
            .await?;
        self.server = r;
        Ok(self)
    }

    /// [`Self::from_streams`] plus `initialize` (tests).
    ///
    /// # Errors
    /// When the handshake fails.
    pub async fn connect(
        read: impl AsyncRead + Send + Unpin + 'static,
        write: impl AsyncWrite + Send + Unpin + 'static,
        sink: Sink,
    ) -> Result<Self, String> {
        Self::from_streams(read, write, sink, None)
            .initialize()
            .await
    }

    async fn list_all(&self, method: &str, key: &str) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let r = self.request(method, params, CALL_TIMEOUT).await?;
            out.extend(r[key].as_array().cloned().unwrap_or_default());
            cursor = r["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Tools, resources and prompts the server offers (those its capabilities announce).
    ///
    /// # Errors
    /// A list request failed.
    pub async fn inspect(&self) -> Result<Inventory, String> {
        let caps = &self.server["capabilities"];
        let tools = if caps.get("tools").is_some() {
            self.list_all("tools/list", "tools").await?
        } else {
            Vec::new()
        };
        let resources = if caps.get("resources").is_some() {
            self.list_all("resources/list", "resources").await?
        } else {
            Vec::new()
        };
        let prompts = if caps.get("prompts").is_some() {
            self.list_all("prompts/list", "prompts").await?
        } else {
            Vec::new()
        };
        Ok(Inventory {
            server: self.server.clone(),
            tools,
            resources,
            prompts,
        })
    }

    /// `tools/call`.
    ///
    /// # Errors
    /// See [`Self::request`].
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, String> {
        self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
            CALL_TIMEOUT,
        )
        .await
    }

    /// `resources/read`.
    ///
    /// # Errors
    /// See [`Self::request`].
    pub async fn read_resource(&self, uri: &str) -> Result<Value, String> {
        self.request("resources/read", json!({"uri": uri}), CALL_TIMEOUT)
            .await
    }

    /// `prompts/get`.
    ///
    /// # Errors
    /// See [`Self::request`].
    pub async fn get_prompt(&self, name: &str, arguments: Value) -> Result<Value, String> {
        self.request(
            "prompts/get",
            json!({"name": name, "arguments": arguments}),
            CALL_TIMEOUT,
        )
        .await
    }

    /// The traffic so far, oldest first.
    pub fn traffic(&self) -> Vec<Traffic> {
        lock(&self.log).iter().cloned().collect()
    }

    /// Stop the server process.
    pub fn stop(&self) {
        if let Some(mut c) = lock(&self.child).take() {
            let _ = c.start_kill();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Inventory {
    pub server: Value,
    pub tools: Vec<Value>,
    pub resources: Vec<Value>,
    pub prompts: Vec<Value>,
}

/// What changed in the tool list between two reloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SchemaDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// `(tool, what changed)`.
    pub changed: Vec<(String, String)>,
}

impl SchemaDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Compare two `tools/list` results by name: added, removed, and changed description or schema
/// (with the schema properties added/removed).
pub fn schema_diff(old: &[Value], new: &[Value]) -> SchemaDiff {
    let by = |v: &[Value]| -> std::collections::BTreeMap<String, Value> {
        v.iter()
            .filter_map(|t| Some((t["name"].as_str()?.to_owned(), t.clone())))
            .collect()
    };
    let (o, n) = (by(old), by(new));
    let mut d = SchemaDiff {
        added: n.keys().filter(|k| !o.contains_key(*k)).cloned().collect(),
        removed: o.keys().filter(|k| !n.contains_key(*k)).cloned().collect(),
        ..SchemaDiff::default()
    };
    for (name, a) in &o {
        let Some(b) = n.get(name) else { continue };
        if a["description"] != b["description"] {
            d.changed.push((name.clone(), "description".into()));
        }
        if a["inputSchema"] != b["inputSchema"] {
            let props = |v: &Value| -> std::collections::BTreeSet<String> {
                v["inputSchema"]["properties"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default()
            };
            let (pa, pb) = (props(a), props(b));
            let plus: Vec<String> = pb.difference(&pa).map(|p| format!("+{p}")).collect();
            let minus: Vec<String> = pa.difference(&pb).map(|p| format!("-{p}")).collect();
            let what = if plus.is_empty() && minus.is_empty() {
                "inputSchema".to_owned()
            } else {
                format!(
                    "inputSchema {}",
                    plus.into_iter().chain(minus).collect::<Vec<_>>().join(" ")
                )
            };
            d.changed.push((name.clone(), what));
        }
    }
    d
}

/// Watch `path` and call `on_change` (debounced, 400 ms) when a source file changes; build
/// output, VCS and dependency folders are ignored. Dropping the watcher stops it.
///
/// # Errors
/// When the watcher cannot start.
pub fn watch(
    path: &Path,
    on_change: impl Fn() + Send + 'static,
) -> Result<notify::RecommendedWatcher, String> {
    use notify::Watcher as _;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut w = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
        if let Ok(ev) = ev
            && ev.kind.is_modify() | ev.kind.is_create() | ev.kind.is_remove()
            && ev.paths.iter().any(|p| !ignored(p))
        {
            let _ = tx.send(());
        }
    })
    .map_err(|e| e.to_string())?;
    let mode = if path.is_dir() {
        notify::RecursiveMode::Recursive
    } else {
        notify::RecursiveMode::NonRecursive
    };
    w.watch(path, mode).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Debounce: an editor save is several events.
            while rx.recv_timeout(Duration::from_millis(400)).is_ok() {}
            on_change();
        }
    });
    Ok(w)
}

fn ignored(p: &Path) -> bool {
    p.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(
                "node_modules"
                    | "target"
                    | ".git"
                    | "__pycache__"
                    | ".venv"
                    | "dist"
                    | "build"
                    | ".kor"
            )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny MCP server over a duplex pipe: one tool `echo`, one resource, one prompt; `boom`
    /// answers -32602; a stray non-JSON line comes first.
    async fn fake(io: tokio::io::DuplexStream, version: u32) {
        let (r, mut w) = tokio::io::split(io);
        let mut lines = BufReader::new(r).lines();
        let _ = w.write_all(b"debug: starting\n").await;
        while let Ok(Some(l)) = lines.next_line().await {
            let req: Value = serde_json::from_str(&l).unwrap_or_default();
            let Some(id) = req.get("id").cloned() else {
                continue;
            };
            let props = if version == 1 {
                json!({"text": {"type": "string"}})
            } else {
                json!({"text": {"type": "string"}, "loud": {"type": "boolean"}})
            };
            let body = match req["method"].as_str().unwrap_or("") {
                "initialize" => {
                    json!({"result": {"protocolVersion": PROTOCOL, "serverInfo": {"name": "fake", "version": "1"}, "capabilities": {"tools": {}, "resources": {}, "prompts": {}}}})
                }
                "tools/list" => {
                    json!({"result": {"tools": [{"name": "echo", "description": "Echo", "inputSchema": {"type": "object", "properties": props}}]}})
                }
                "resources/list" => {
                    json!({"result": {"resources": [{"uri": "mem://a", "name": "a"}]}})
                }
                "prompts/list" => json!({"result": {"prompts": [{"name": "greet"}]}}),
                "tools/call" if req["params"]["name"] == "boom" => {
                    json!({"error": {"code": -32602, "message": "bad args"}})
                }
                "tools/call" => {
                    json!({"result": {"content": [{"type": "text", "text": req["params"]["arguments"]["text"]}]}})
                }
                "resources/read" => {
                    json!({"result": {"contents": [{"uri": "mem://a", "text": "A"}]}})
                }
                "prompts/get" => json!({"result": {"messages": []}}),
                _ => json!({"error": {"code": -32601, "message": "nope"}}),
            };
            let mut body = body;
            body["jsonrpc"] = json!("2.0");
            body["id"] = id;
            let _ = w.write_all(format!("{body}\n").as_bytes()).await;
        }
    }

    async fn session(version: u32, seen: Arc<Mutex<Vec<Traffic>>>) -> Session {
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake(b, version));
        let (r, w) = tokio::io::split(a);
        let sink: Sink = Arc::new(move |t: &Traffic| lock(&seen).push(t.clone()));
        Session::connect(r, w, sink).await.unwrap()
    }

    #[tokio::test]
    async fn inspect_call_and_log() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = session(1, seen.clone()).await;
        assert_eq!(s.server["serverInfo"]["name"], "fake");
        let inv = s.inspect().await.unwrap();
        assert_eq!(
            (inv.tools.len(), inv.resources.len(), inv.prompts.len()),
            (1, 1, 1)
        );
        let r = s.call_tool("echo", json!({"text": "hi"})).await.unwrap();
        assert_eq!(r["content"][0]["text"], "hi");
        let e = s.call_tool("boom", json!({})).await.unwrap_err();
        assert!(e.contains("-32602") && e.contains("inputSchema"), "{e}");
        assert_eq!(
            s.read_resource("mem://a").await.unwrap()["contents"][0]["text"],
            "A"
        );
        assert!(s.get_prompt("greet", json!({})).await.is_ok());
        let t = s.traffic();
        assert!(
            t.iter()
                .any(|x| x.dir == Dir::Note && x.text.contains("not JSON"))
        );
        assert!(
            t.iter()
                .any(|x| x.dir == Dir::Out && x.text.contains("tools/call"))
        );
        assert!(
            t.iter()
                .any(|x| x.dir == Dir::In && x.text.contains("bad args"))
        );
        assert_eq!(
            lock(&seen).len(),
            t.len(),
            "the live sink saw every message"
        );
    }

    #[tokio::test]
    async fn reload_diffs_the_schema() {
        let a = session(1, Arc::default()).await.inspect().await.unwrap();
        let b = session(2, Arc::default()).await.inspect().await.unwrap();
        let d = schema_diff(&a.tools, &b.tools);
        assert_eq!(
            d.changed,
            vec![("echo".to_owned(), "inputSchema +loud".to_owned())]
        );
        assert!(schema_diff(&a.tools, &a.tools).is_empty());
        let none: Vec<Value> = Vec::new();
        assert_eq!(schema_diff(&none, &a.tools).added, vec!["echo".to_owned()]);
    }

    #[test]
    fn detects_entries() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("package.json"),
            r#"{"main": "dist/index.js"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(d.path().join("dist")).unwrap();
        std::fs::write(d.path().join("dist/index.js"), "").unwrap();
        let t = detect(d.path()).unwrap();
        assert_eq!(t.command, "node");
        assert!(t.args[0].ends_with("index.js"));
        let p = tempfile::tempdir().unwrap();
        std::fs::write(p.path().join("server.py"), "").unwrap();
        assert!(detect(p.path()).unwrap().args[0].ends_with("server.py"));
        let e = tempfile::tempdir().unwrap();
        assert!(detect(e.path()).is_err());
        assert!(explain(-32601).contains("method not found"));
        assert!(ignored(Path::new("a/node_modules/x.js")) && !ignored(Path::new("src/x.ts")));
    }
}
