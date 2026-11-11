//! MCP servers as agent tools.
//!
//! Each configured stdio server is started once per session and kept running; its tools appear to
//! the model as `mcp__<server>__<tool>` next to the built-in ones. Configuration uses the
//! `mcpServers` format shared by Claude Code and Claude Desktop.
//!
//! Sources, later ones overriding earlier ones with the same name:
//! 1. `~/.claude.json` top-level `mcpServers` (user scope)
//! 2. `~/.claude.json` `projects[<root>].mcpServers` (local scope)
//! 3. Kor's user file `<config>/kor/mcp.json` (edited by the desktop app; `"disabled": true` there
//!    turns a server off wherever it is defined)
//! 4. The project's `.mcp.json`, only when the project is trusted (`kor trust`): a cloned repository
//!    must not be able to start programs by itself.

use kor_platform::NoWindow as _;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use kor_core::{ImageSource, ToolSpec};
use kor_providers::CancellationToken;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

const PROTOCOL_VERSION: &str = "2025-03-26";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Tool output kept for the model; larger results are cut with a note.
const MAX_OUTPUT_CHARS: usize = 100_000;
/// Model-facing tool names are limited to 64 characters by several providers.
const MAX_TOOL_NAME: usize = 64;
const STDERR_TAIL: usize = 20;
const MAX_LIST_PAGES: usize = 20;

/// A stdio server from configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Config file it came from, for messages.
    pub origin: String,
}

/// Why a configured server is not started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// HTTP/SSE servers are listed but not connected yet.
    Remote { name: String, origin: String },
    Invalid {
        name: String,
        origin: String,
        reason: String,
    },
    /// Project `.mcp.json` servers in an untrusted project.
    Untrusted { count: usize },
}

/// Where to look for configuration.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub project_root: &'a Path,
    pub project_trusted: bool,
    pub home: Option<&'a Path>,
    /// Kor's user config directory (`%APPDATA%\kor`, `~/.config/kor`).
    pub kor_config_dir: Option<&'a Path>,
}

/// `${VAR}` and `${VAR:-default}` from the environment; unknown variables become empty.
pub fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (var, default) = inner
            .split_once(":-")
            .map_or((inner, None), |(v, d)| (v, Some(d)));
        match env(var).filter(|v| !v.is_empty()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(default.unwrap_or_default()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_servers(
    value: Option<&Value>,
    origin: &str,
    env: &dyn Fn(&str) -> Option<String>,
    found: &mut BTreeMap<String, Result<ServerSpec, Skip>>,
    disabled: &mut HashSet<String>,
) {
    let Some(servers) = value.and_then(Value::as_object) else {
        return;
    };
    for (name, spec) in servers {
        // `"disabled": true` turns a server off wherever else it is defined (the desktop's switch).
        if spec.get("disabled").and_then(Value::as_bool) == Some(true) {
            disabled.insert(name.clone());
            continue;
        }
        let kind = spec.get("type").and_then(Value::as_str).unwrap_or("stdio");
        let entry =
            if spec.get("url").is_some() || matches!(kind, "http" | "sse" | "streamable-http") {
                Err(Skip::Remote {
                    name: name.clone(),
                    origin: origin.to_owned(),
                })
            } else if let Some(command) = spec.get("command").and_then(Value::as_str) {
                let strings = |v: Option<&Value>| -> Vec<String> {
                    v.and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|s| expand(s, env))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let vars = spec
                    .get("env")
                    .and_then(Value::as_object)
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), expand(v, env))))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(ServerSpec {
                    name: name.clone(),
                    command: expand(command, env),
                    args: strings(spec.get("args")),
                    env: vars,
                    origin: origin.to_owned(),
                })
            } else {
                Err(Skip::Invalid {
                    name: name.clone(),
                    origin: origin.to_owned(),
                    reason: "missing \"command\"".into(),
                })
            };
        found.insert(name.clone(), entry);
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// `~/.claude.json` keys projects by path as Claude Code wrote it; compare loosely.
fn same_path(a: &str, b: &Path) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let b = b.to_string_lossy();
    let b = b.strip_prefix(r"\\?\").unwrap_or(&b);
    norm(a) == norm(b)
}

/// Configured servers to start, and the ones that were not.
pub fn discover(
    src: &Sources<'_>,
    env: &dyn Fn(&str) -> Option<String>,
) -> (Vec<ServerSpec>, Vec<Skip>) {
    let mut found = BTreeMap::new();
    let mut disabled = HashSet::new();
    if let Some(home) = src.home {
        for (label, servers) in crate::mcp_config::other_tools_for(home) {
            parse_servers(
                Some(&Value::Object(servers)),
                label,
                env,
                &mut found,
                &mut disabled,
            );
        }
        let p = home.join(".claude.json");
        if let Some(v) = read_json(&p) {
            let origin = p.display().to_string();
            parse_servers(v.get("mcpServers"), &origin, env, &mut found, &mut disabled);
            if let Some(projects) = v.get("projects").and_then(Value::as_object) {
                for (key, project) in projects {
                    if same_path(key, src.project_root) {
                        parse_servers(
                            project.get("mcpServers"),
                            &origin,
                            env,
                            &mut found,
                            &mut disabled,
                        );
                    }
                }
            }
        }
    }
    // Kor's own file (what the desktop edits) wins over Claude Code's.
    if let Some(dir) = src.kor_config_dir {
        let p = dir.join("mcp.json");
        parse_servers(
            read_json(&p).as_ref().and_then(|v| v.get("mcpServers")),
            &p.display().to_string(),
            env,
            &mut found,
            &mut disabled,
        );
    }
    let mut skipped = Vec::new();
    let project_file = src.project_root.join(".mcp.json");
    if let Some(v) = read_json(&project_file) {
        if src.project_trusted {
            parse_servers(
                v.get("mcpServers"),
                ".mcp.json",
                env,
                &mut found,
                &mut disabled,
            );
        } else {
            let count = v
                .get("mcpServers")
                .and_then(Value::as_object)
                .map_or(0, serde_json::Map::len);
            if count > 0 {
                skipped.push(Skip::Untrusted { count });
            }
        }
    }
    found.retain(|name, _| !disabled.contains(name));
    let mut specs = Vec::new();
    for entry in found.into_values() {
        match entry {
            Ok(s) => specs.push(s),
            Err(s) => skipped.push(s),
        }
    }
    (specs, skipped)
}

// ---------------------------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------------------------

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
type Writer = Arc<tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>>;

/// A running stdio server. Dropping the last handle kills the process.
pub struct McpClient {
    pub name: String,
    stdin: Writer,
    pending: Pending,
    next_id: AtomicU64,
    stderr: Arc<Mutex<VecDeque<String>>>,
    // Held so the process lives as long as the client (`kill_on_drop`).
    child: Mutex<Option<Child>>,
    /// How it was started, for [`McpClient::restart`] (§28.15); `None` for in-memory servers.
    spawn: Mutex<Option<(ServerSpec, PathBuf)>>,
    reader: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Windows cannot start `npx` or `uvx` by bare name: they are `.cmd` shims. Look the command up on
/// `PATH` with `PATHEXT` the way a shell would; the standard library runs `.cmd` files with safe
/// argument escaping.
fn resolve_command(command: &str) -> PathBuf {
    let path = Path::new(command);
    if !cfg!(windows) || path.extension().is_some() || path.components().count() > 1 {
        return path.to_owned();
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{command}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path.to_owned()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

type Spawned = (
    Child,
    tokio::process::ChildStdin,
    tokio::process::ChildStdout,
    Option<tokio::process::ChildStderr>,
);

/// Start a server's process with its pipes.
fn spawn_server(spec: &ServerSpec, cwd: &Path) -> Result<Spawned, String> {
    let mut cmd = Command::new(resolve_command(&spec.command));
    cmd.args(&spec.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.no_window();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start `{}`: {e}", spec.command))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take();
    Ok((child, stdin, stdout, stderr))
}

impl McpClient {
    /// Start the server, complete the `initialize` handshake and send `notifications/initialized`.
    /// The server inherits Kor's environment plus its own `env`, as in Claude Code: many servers read
    /// their API key from the environment, and a process of the same user could read the keychain
    /// anyway, so withholding variables would break servers without protecting anything.
    pub async fn start(spec: &ServerSpec, cwd: &Path) -> Result<Arc<Self>, String> {
        let (child, stdin, stdout, stderr_pipe) = spawn_server(spec, cwd)?;
        let client = Self::from_streams(&spec.name, stdout, stdin, Some(child));
        *lock(&client.spawn) = Some((spec.clone(), cwd.to_path_buf()));
        client.tail_stderr(stderr_pipe);
        client.handshake().await?;
        Ok(client)
    }

    fn tail_stderr(&self, pipe: Option<tokio::process::ChildStderr>) {
        if let Some(pipe) = pipe {
            let tail = self.stderr.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut t = lock(&tail);
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            });
        }
    }

    /// Restart the server process in place (§28.15, `mcp_restart`): the old process is stopped,
    /// its pending calls fail, a new one is started with the same command and handshaken. Tools
    /// keep their handle, so the agent's tool list stays valid.
    ///
    /// # Errors
    /// The server was not started from a command, or the new process fails to start.
    pub async fn restart(&self) -> Result<(), String> {
        let (spec, cwd) = lock(&self.spawn)
            .clone()
            .ok_or("this MCP server was not started by Kor")?;
        if let Some(mut old) = lock(&self.child).take() {
            let _ = old.start_kill();
        }
        let reader = lock(&self.reader).take();
        if let Some(r) = reader {
            // The old reader drains the pending calls once the old pipe closes.
            let _ = tokio::time::timeout(Duration::from_secs(5), r).await;
        }
        lock(&self.stderr).clear();
        let (child, stdin, stdout, stderr_pipe) = spawn_server(&spec, &cwd)?;
        *self.stdin.lock().await = Box::new(stdin);
        *lock(&self.child) = Some(child);
        *lock(&self.reader) = Some(tokio::spawn(read_loop(
            BufReader::new(stdout),
            self.pending.clone(),
            self.stdin.clone(),
        )));
        self.tail_stderr(stderr_pipe);
        self.handshake().await
    }

    /// A client over any byte streams (the process's pipes, or an in-memory server in tests).
    pub fn from_streams<R>(
        name: &str,
        read: R,
        write: impl AsyncWrite + Send + Unpin + 'static,
        child: Option<Child>,
    ) -> Arc<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let stdin: Writer = Arc::new(tokio::sync::Mutex::new(Box::new(write)));
        let pending: Pending = Arc::default();
        let reader = tokio::spawn(read_loop(
            BufReader::new(read),
            pending.clone(),
            stdin.clone(),
        ));
        Arc::new(Self {
            name: name.to_owned(),
            stdin,
            pending,
            next_id: AtomicU64::new(1),
            stderr: Arc::default(),
            child: Mutex::new(child),
            spawn: Mutex::new(None),
            reader: Mutex::new(Some(reader)),
        })
    }

    /// `initialize` then `notifications/initialized`.
    pub async fn handshake(&self) -> Result<(), String> {
        let init = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "kor", "version": kor_core::VERSION},
        });
        self.request("initialize", init, START_TIMEOUT, None)
            .await?;
        self.notify("notifications/initialized", json!({})).await
    }

    /// The error plus the last lines the server wrote to stderr (usually the real reason).
    fn with_stderr(&self, e: String) -> String {
        let tail = lock(&self.stderr);
        let last: Vec<&str> = tail.iter().rev().take(3).map(String::as_str).collect();
        if last.is_empty() {
            e
        } else {
            format!(
                "{e} ({})",
                last.into_iter().rev().collect::<Vec<_>>().join(" | ")
            )
        }
    }

    async fn write(&self, body: &Value) -> Result<(), String> {
        let mut w = self.stdin.lock().await;
        w.write_all(format!("{body}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        w.flush().await.map_err(|e| e.to_string())
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    /// One request; times out, and is abandoned (with `notifications/cancelled`) when `cancel` fires.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = self.write(&body).await {
            lock(&self.pending).remove(&id);
            return Err(self.with_stderr(e));
        }
        let never = CancellationToken::new();
        let cancelled = cancel.unwrap_or(&never);
        let outcome = tokio::select! {
            r = rx => r.unwrap_or_else(|_| Err("server closed".into())),
            () = tokio::time::sleep(timeout) => Err(format!("no reply within {} s", timeout.as_secs())),
            () = cancelled.cancelled() => Err("cancelled".into()),
        };
        if outcome.is_err() && lock(&self.pending).remove(&id).is_some() {
            let _ = self
                .notify(
                    "notifications/cancelled",
                    json!({"requestId": id, "reason": "client"}),
                )
                .await;
        }
        outcome.map_err(|e| self.with_stderr(e))
    }

    /// All tools, following `nextCursor` pages.
    pub async fn list_tools(&self) -> Result<Vec<RemoteTool>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let page = self
                .request("tools/list", params, START_TIMEOUT, None)
                .await?;
            for t in page
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = t.get("name").and_then(Value::as_str) else {
                    continue;
                };
                tools.push(RemoteTool {
                    name: name.to_owned(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    input_schema: t
                        .get("inputSchema")
                        .filter(|s| s.is_object())
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                    annotations: t.get("annotations").cloned().unwrap_or(Value::Null),
                });
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }
}

/// Route responses to their waiting requests; answer the server's own requests.
async fn read_loop<R>(mut read: BufReader<R>, pending: Pending, stdin: Writer)
where
    R: AsyncRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        match read.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // Servers sometimes print logs to stdout; anything that is not JSON-RPC is ignored.
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = msg.get("id").cloned();
        if let Some(method) = msg.get("method").and_then(Value::as_str) {
            // A request from the server (has an id) needs an answer; notifications do not.
            if let Some(id) = id {
                let reply = if method == "ping" {
                    json!({"jsonrpc": "2.0", "id": id, "result": {}})
                } else {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not supported by Kor"}})
                };
                let mut w = stdin.lock().await;
                let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = w.flush().await;
            }
            continue;
        }
        let Some(id) = id.and_then(|v| v.as_u64()) else {
            continue;
        };
        let result = match msg.get("error") {
            Some(err) => Err(err
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| err.to_string(), str::to_owned)),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        if let Some(tx) = lock(&pending).remove(&id) {
            let _ = tx.send(result);
        }
    }
    for (_, tx) in lock(&pending).drain() {
        let _ = tx.send(Err("server closed".into()));
    }
}

/// A tool as the server describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// MCP tool annotations (`readOnlyHint`, `destructiveHint`…), `Null` when none.
    pub annotations: Value,
}

/// [`Tool::target`] of a tool its server marks `destructiveHint: true`; the permission check
/// treats it as dangerous, so it always asks (ADR 0038).
pub const WRITE_MARK: &str = "mcp-write";

// ---------------------------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------------------------

/// `Tool::name` is `&'static str`; MCP names arrive at run time. Each distinct name is stored once
/// for the life of the process.
pub(crate) fn intern(s: String) -> &'static str {
    static NAMES: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Mutex::default);
    let mut names = lock(&NAMES);
    if let Some(&existing) = names.get(s.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.into_boxed_str());
    names.insert(leaked);
    leaked
}

/// `mcp__<server>__<tool>`, with characters providers reject replaced, cut to 64 characters.
pub fn tool_name(server: &str, tool: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let mut name = format!("mcp__{}__{}", clean(server), clean(tool));
    name.truncate(MAX_TOOL_NAME);
    name
}

/// One server tool exposed to the model.
#[derive(Debug)]
pub struct McpTool {
    client: Arc<McpClient>,
    remote: RemoteTool,
    name: &'static str,
}

impl McpTool {
    pub fn new(client: Arc<McpClient>, remote: RemoteTool) -> Self {
        let name = intern(tool_name(&client.name, &remote.name));
        Self {
            client,
            remote,
            name,
        }
    }
}

fn call_timeout() -> Duration {
    std::env::var("MCP_TOOL_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_CALL_TIMEOUT, Duration::from_millis)
}

/// MCP `CallToolResult` → tool output: text parts joined, the first image attached, structured
/// content as JSON when there is no text.
pub fn to_output(result: &Value) -> ToolOutput {
    let mut text = Vec::new();
    let mut image = None;
    for part in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => text.push(
                part.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            Some("image") if image.is_none() => {
                if let (Some(data), Some(mime)) = (
                    part.get("data").and_then(Value::as_str),
                    part.get("mimeType").and_then(Value::as_str),
                ) {
                    image = Some(ImageSource::Base64 {
                        media_type: mime.to_owned(),
                        data: data.to_owned(),
                    });
                    text.push(format!("[image: {mime}]"));
                }
            }
            Some("resource") => {
                let r = part.get("resource");
                let body = r.and_then(|r| r.get("text")).and_then(Value::as_str);
                let uri = r
                    .and_then(|r| r.get("uri"))
                    .and_then(Value::as_str)
                    .unwrap_or("resource");
                text.push(body.map_or_else(|| format!("[{uri}]"), str::to_owned));
            }
            Some(other) => text.push(format!("[{other} content]")),
            None => {}
        }
    }
    if text.is_empty()
        && let Some(s) = result.get("structuredContent")
    {
        text.push(s.to_string());
    }
    let mut content = text.join("\n");
    if content.chars().count() > MAX_OUTPUT_CHARS {
        content = content.chars().take(MAX_OUTPUT_CHARS).collect();
        content.push_str("\n… [output truncated]");
    }
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    ToolOutput {
        content,
        is_error,
        image,
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn rule_name(&self) -> &'static str {
        self.name
    }
    /// Server tools can do anything; they ask like commands unless a rule allows them.
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        let description = if self.remote.description.is_empty() {
            format!(
                "Tool `{}` of the MCP server `{}`.",
                self.remote.name, self.client.name
            )
        } else {
            format!(
                "{} (MCP server `{}`)",
                self.remote.description, self.client.name
            )
        };
        ToolSpec {
            name: self.name.to_owned(),
            description,
            input_schema: self.remote.input_schema.clone(),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        (self.remote.annotations.get("destructiveHint") == Some(&Value::Bool(true)))
            .then(|| format!("{WRITE_MARK} {}", self.name))
    }
    fn summary(&self, input: &Value) -> String {
        let s = input.to_string();
        let short: String = s.chars().take(120).collect();
        if s.chars().count() > 120 {
            format!("{short}…")
        } else {
            short
        }
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let params = json!({"name": self.remote.name, "arguments": input});
        match self
            .client
            .request("tools/call", params, call_timeout(), Some(&ctx.cancel))
            .await
        {
            Ok(result) => to_output(&result),
            Err(e) => ToolOutput::err(format!("MCP server `{}`: {e}", self.client.name)),
        }
    }
}

/// Outcome of starting one server.
#[derive(Debug)]
pub struct Connected {
    pub server: String,
    pub result: Result<usize, String>,
    /// The running client (for `mcp_restart`).
    pub client: Option<Arc<McpClient>>,
}

/// `mcp_restart` (§28.15): the agent restarts a connected server after changing its code, so the
/// edit-and-test loop does not need the user. A command tool: it asks unless a rule allows it.
#[derive(Debug)]
pub struct McpRestart {
    clients: Vec<Arc<McpClient>>,
}

impl McpRestart {
    pub fn new(clients: Vec<Arc<McpClient>>) -> Self {
        Self { clients }
    }
}

#[async_trait]
impl Tool for McpRestart {
    fn name(&self) -> &'static str {
        "mcp_restart"
    }
    fn rule_name(&self) -> &'static str {
        "mcp_restart"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        let names: Vec<&str> = self.clients.iter().map(|c| c.name.as_str()).collect();
        ToolSpec {
            name: "mcp_restart".into(),
            description: format!(
                "Restart a connected MCP server (after you changed its code) so its tools run the new version. Servers: {}.",
                names.join(", ")
            ),
            input_schema: json!({
                "type": "object",
                "properties": {"server": {"type": "string", "enum": names}},
                "required": ["server"],
                "additionalProperties": false
            }),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["server"].as_str().map(str::to_owned)
    }
    fn summary(&self, input: &Value) -> String {
        format!(
            "restart MCP server {}",
            input["server"].as_str().unwrap_or("?")
        )
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let name = input["server"].as_str().unwrap_or_default();
        let Some(c) = self.clients.iter().find(|c| c.name == name) else {
            return ToolOutput::err(format!("no connected MCP server `{name}`"));
        };
        match c.restart().await {
            Ok(()) => match c.list_tools().await {
                Ok(t) => ToolOutput::ok(format!(
                    "MCP server `{name}` restarted; it offers {} tools. Tools whose schema changed take effect for calls from now on; new tools appear in the next session.",
                    t.len()
                )),
                Err(e) => ToolOutput::err(format!(
                    "`{name}` restarted but listing its tools failed: {e}"
                )),
            },
            Err(e) => ToolOutput::err(format!("MCP server `{name}` could not restart: {e}")),
        }
    }
}

/// Start every server in parallel and wrap their tools. A server that fails to start or list its
/// tools is reported and skipped; the others still load.
pub async fn connect_all(specs: &[ServerSpec], cwd: &Path) -> (Vec<Arc<dyn Tool>>, Vec<Connected>) {
    let starts = specs.iter().map(|spec| async move {
        let started = async {
            // MCP servers are arbitrary programs (often `npx` fetching packages): not started in
            // privacy/offline mode, whoever asks (ADR 0035).
            if let Some(b) = kor_core::net::refusal(&spec.name) {
                return Err(b.message);
            }
            let client = McpClient::start(spec, cwd).await?;
            let remote = client.list_tools().await?;
            Ok::<_, String>((client, remote))
        }
        .await;
        (spec.name.clone(), started)
    });
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    let mut report = Vec::new();
    let mut seen = HashSet::new();
    for (server, started) in futures::future::join_all(starts).await {
        match started {
            Ok((client, remote)) => {
                let mut n = 0;
                for r in remote {
                    let tool = McpTool::new(client.clone(), r);
                    // Two tools that clean up to the same name would be ambiguous to the model.
                    if seen.insert(tool.name) {
                        tools.push(Arc::new(tool));
                        n += 1;
                    }
                }
                report.push(Connected {
                    server,
                    result: Ok(n),
                    client: Some(client),
                });
            }
            Err(e) => report.push(Connected {
                server,
                result: Err(e),
                client: None,
            }),
        }
    }
    (tools, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn expands_variables_with_defaults() {
        let env = |k: &str| (k == "HOME_DIR").then(|| "/home/x".to_owned());
        assert_eq!(expand("${HOME_DIR}/data", &env), "/home/x/data");
        assert_eq!(expand("${MISSING:-fallback}", &env), "fallback");
        assert_eq!(expand("${MISSING}", &env), "");
        assert_eq!(
            expand("no vars ${unterminated", &env),
            "no vars ${unterminated"
        );
    }

    #[test]
    fn names_are_clean_and_bounded() {
        assert_eq!(
            tool_name("my server", "read.file"),
            "mcp__my_server__read_file"
        );
        assert!(tool_name(&"s".repeat(50), &"t".repeat(50)).len() <= 64);
        assert!(std::ptr::eq(
            intern("mcp__a__b".into()),
            intern("mcp__a__b".into())
        ));
    }

    #[test]
    fn untrusted_project_servers_are_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"evil":{"command":"calc.exe"},"remote":{"url":"https://x"}}}"#,
        )
        .unwrap();
        let src = |trusted| Sources {
            project_root: root,
            project_trusted: trusted,
            home: None,
            kor_config_dir: None,
        };
        let (specs, skipped) = discover(&src(false), &no_env);
        assert!(specs.is_empty());
        assert_eq!(skipped, vec![Skip::Untrusted { count: 2 }]);
        let (specs, skipped) = discover(&src(true), &no_env);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].command, "calc.exe");
        assert!(matches!(&skipped[0], Skip::Remote { name, .. } if name == "remote"));
    }

    #[test]
    fn user_sources_merge_and_kor_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cfg = dir.path().join("cfg");
        let project = dir.path().join("proj");
        for d in [&home, &cfg, &project] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            cfg.join("mcp.json"),
            r#"{"mcpServers":{"docs":{"command":"docs-v1","env":{"TOKEN":"${T:-none}"}}}}"#,
        )
        .unwrap();
        let claude = json!({
            "mcpServers": {"git": {"command": "git-mcp", "args": ["--repo", "${REPO}"]}},
            "projects": {
                project.to_string_lossy().replace('\\', "/"): {"mcpServers": {"docs": {"command": "docs-v2"}}}
            }
        });
        std::fs::write(home.join(".claude.json"), claude.to_string()).unwrap();
        let env = |k: &str| (k == "REPO").then(|| "kor".to_owned());
        let (specs, skipped) = discover(
            &Sources {
                project_root: &project,
                project_trusted: false,
                home: Some(&home),
                kor_config_dir: Some(&cfg),
            },
            &env,
        );
        assert!(skipped.is_empty());
        let by: HashMap<_, _> = specs.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(
            by["docs"].command, "docs-v1",
            "Kor's own file overrides Claude Code's"
        );
        assert_eq!(by["git"].args, vec!["--repo", "kor"]);
    }

    #[test]
    fn disabled_in_kor_file_hides_server_from_other_sources() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cfg = dir.path().join("cfg");
        for d in [&home, &cfg] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            cfg.join("mcp.json"),
            r#"{"mcpServers":{"big":{"disabled":true},"off":{"command":"x","disabled":true}}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"big":{"command":"big-mcp"},"keep":{"command":"k"}}}"#,
        )
        .unwrap();
        let (specs, skipped) = discover(
            &Sources {
                project_root: dir.path(),
                project_trusted: false,
                home: Some(&home),
                kor_config_dir: Some(&cfg),
            },
            &no_env,
        );
        assert!(skipped.is_empty());
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["keep"]);
    }

    /// In-memory server: interleaves a notification and its own `ping` request before each reply,
    /// pages `tools/list`, fails `tools/call` for `boom`, never answers `slow`, and records what
    /// the client sent back.
    async fn fake_server(io: tokio::io::DuplexStream, seen: Arc<Mutex<Vec<Value>>>) {
        let (r, mut w) = tokio::io::split(io);
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            lock(&seen).push(msg.clone());
            let Some(id) = msg.get("id").cloned() else {
                continue;
            };
            if msg.get("method").is_none() {
                continue; // the client's answer to our ping
            }
            let method = msg["method"].as_str().unwrap_or_default();
            let noise = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info"}});
            let ping = json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"});
            let reply = match method {
                "initialize" => {
                    json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": PROTOCOL_VERSION}})
                }
                "tools/list" if msg["params"].get("cursor").is_none() => {
                    json!({"jsonrpc": "2.0", "id": id,
                    "result": {"tools": [{"name": "echo", "description": "Echo back", "inputSchema": {"type": "object"}}], "nextCursor": "p2"}})
                }
                "tools/list" => {
                    json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{"name": "boom"}, {"name": "slow"}]}})
                }
                "tools/call" if msg["params"]["name"] == "boom" => {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -1, "message": "exploded"}})
                }
                "tools/call" if msg["params"]["name"] == "slow" => continue,
                "tools/call" => {
                    json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": msg["params"]["arguments"]["text"]}]}})
                }
                _ => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            };
            for m in [noise, ping, reply] {
                let _ = w.write_all(format!("{m}\n").as_bytes()).await;
            }
            let _ = w.flush().await;
        }
    }

    #[tokio::test]
    async fn restart_tool_needs_a_started_server() {
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake_server(b, Arc::default()));
        let (r, w) = tokio::io::split(a);
        let client = McpClient::from_streams("fake", r, w, None);
        client.handshake().await.unwrap();
        assert!(
            client
                .restart()
                .await
                .unwrap_err()
                .contains("not started by Kor")
        );
        let tool = McpRestart::new(vec![client]);
        assert_eq!(
            tool.spec().input_schema["properties"]["server"]["enum"],
            json!(["fake"])
        );
        assert_eq!(tool.kind(), ToolKind::Execute);
        let ctx = ToolCtx::new(&std::env::temp_dir()).unwrap();
        assert!(tool.call(json!({"server": "other"}), &ctx).await.is_error);
        assert!(
            tool.call(json!({"server": "fake"}), &ctx)
                .await
                .content
                .contains("could not restart")
        );
    }

    #[tokio::test]
    async fn client_routes_replies_answers_pings_and_cancels() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let seen = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(fake_server(b, seen.clone()));
        let (r, w) = tokio::io::split(a);
        let client = McpClient::from_streams("fake", r, w, None);
        client.handshake().await.unwrap();

        let tools = client.list_tools().await.unwrap();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["echo", "boom", "slow"], "both pages are read");
        assert_eq!(tools[0].description, "Echo back");

        let echo = McpTool::new(client.clone(), tools[0].clone());
        assert_eq!(echo.name(), "mcp__fake__echo");
        let ctx = ToolCtx::new(&std::env::temp_dir()).unwrap();
        let out = echo.call(json!({"text": "merhaba"}), &ctx).await;
        assert_eq!(out.content, "merhaba");

        let boom = McpTool::new(client.clone(), tools[1].clone());
        let out = boom.call(json!({}), &ctx).await;
        assert!(
            out.is_error && out.content.contains("exploded"),
            "{}",
            out.content
        );

        let slow = McpTool::new(client.clone(), tools[2].clone());
        let mut ctx = ToolCtx::new(&std::env::temp_dir()).unwrap();
        ctx.cancel = CancellationToken::new();
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let out = slow.call(json!({}), &ctx).await;
        assert!(
            out.is_error && out.content.contains("cancelled"),
            "{}",
            out.content
        );

        tokio::time::sleep(Duration::from_millis(50)).await;
        let seen = lock(&seen).clone();
        assert!(
            seen.iter()
                .any(|m| m["method"] == "notifications/initialized")
        );
        assert!(
            seen.iter()
                .any(|m| m["id"] == "srv-1" && m.get("result").is_some()),
            "ping answered"
        );
        assert!(
            seen.iter()
                .any(|m| m["method"] == "notifications/cancelled")
        );
    }

    #[tokio::test]
    async fn pending_requests_fail_when_the_server_exits() {
        let (a, b) = tokio::io::duplex(1024);
        let (r, w) = tokio::io::split(a);
        let client = McpClient::from_streams("gone", r, w, None);
        drop(b);
        let err = client
            .request("tools/list", json!({}), Duration::from_secs(5), None)
            .await
            .unwrap_err();
        assert!(
            err.contains("closed") || err.contains("pipe") || err.contains("broken"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_not_fatal() {
        let spec = ServerSpec {
            name: "nope".into(),
            command: "kor-no-such-mcp-server".into(),
            args: vec![],
            env: vec![],
            origin: "test".into(),
        };
        let (tools, report) = connect_all(&[spec], &std::env::temp_dir()).await;
        assert!(tools.is_empty());
        assert!(
            report[0]
                .result
                .as_ref()
                .is_err_and(|e| e.contains("cannot start"))
        );
    }

    #[test]
    fn call_results_become_tool_output() {
        let out = to_output(
            &json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}),
        );
        assert_eq!(out.content, "a\nb");
        assert!(!out.is_error);
        let out = to_output(
            &json!({"content": [{"type": "image", "data": "AAA", "mimeType": "image/png"}], "isError": true}),
        );
        assert!(out.is_error);
        assert!(matches!(out.image, Some(ImageSource::Base64 { .. })));
        let out = to_output(&json!({"content": [], "structuredContent": {"n": 1}}));
        assert_eq!(out.content, r#"{"n":1}"#);
    }
}
