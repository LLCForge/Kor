//! Kor as an ACP agent (`kor acp`, spec §28.8, ADR 0057): editors such as Zed drive Kor over
//! stdio with the Agent Client Protocol v1 — sessions, prompts, streamed messages and tool calls,
//! permission requests answered in the editor (mapped to Kor's approval flow, diffs included),
//! and the editor's file system and terminal where it offers them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::mcp_tools::ServerSpec;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest, StopReason};
use kor_core::{ContentPart, Message, PermissionMode, Role};
use kor_i18n::Localizer;
use kor_providers::CancellationToken;
use kor_tools::EditorBridge;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::rpc::{
    Handler, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, RESOURCE_NOT_FOUND,
    Rpc, RpcError,
};

/// The ACP major version Kor speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// One Kor conversation as ACP drives it. Production: [`KorEngine`]; tests script it.
#[async_trait]
pub trait Session: Send {
    fn id(&self) -> String;
    fn history(&self) -> Vec<Message>;
    fn mode(&self) -> PermissionMode;
    fn set_mode(&mut self, mode: PermissionMode);
    async fn prompt(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<StopReason, String>;
}

/// Opens sessions.
#[async_trait]
pub trait Engine: Send + Sync {
    /// `resume`: an existing session id (`session/load`).
    async fn open(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        mcp: Vec<ServerSpec>,
        editor: Arc<dyn EditorBridge>,
    ) -> Result<Box<dyn Session>, String>;
}

/// Modes an editor can switch between (ids are Kor's permission modes).
const MODES: [PermissionMode; 3] = [
    PermissionMode::Default,
    PermissionMode::AcceptEdits,
    PermissionMode::Plan,
];

fn mode_key(m: PermissionMode) -> (&'static str, &'static str) {
    match m {
        PermissionMode::AcceptEdits => ("acp-mode-accept-edits", "acp-mode-accept-edits-about"),
        PermissionMode::Plan => ("acp-mode-plan", "acp-mode-plan-about"),
        _ => ("acp-mode-default", "acp-mode-default-about"),
    }
}

fn modes_json(l: &Localizer, current: PermissionMode) -> Value {
    let available: Vec<Value> = MODES
        .iter()
        .map(|m| {
            let (n, d) = mode_key(*m);
            json!({ "id": m.as_str(), "name": l.t(n), "description": l.t(d) })
        })
        .collect();
    let current = if MODES.contains(&current) {
        current
    } else {
        PermissionMode::Default
    };
    json!({ "currentModeId": current.as_str(), "availableModes": available })
}

/// ACP `ToolKind` of a Kor tool.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "Read" | "list" | "LSP" | "OCR" | "NotebookRead" => "read",
        "Glob" | "Grep" | "ToolSearch" => "search",
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch" => "edit",
        "Delete" => "delete",
        "Move" => "move",
        "Bash" | "PowerShell" | "RunElevated" => "execute",
        "WebFetch" | "WebSearch" => "fetch",
        "EnterPlanMode" | "ExitPlanMode" => "switch_mode",
        "TodoWrite" => "think",
        _ => "other",
    }
}

/// The prompt's content blocks as one message: text as is, embedded resources as fenced
/// blocks, links as `@path` mentions (Kor reads them itself). Images and audio are not in
/// Kor's prompt capabilities, so a client does not send them.
pub fn prompt_text(blocks: &[Value]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for b in blocks {
        match b["type"].as_str() {
            Some("text") => parts.push(b["text"].as_str().unwrap_or_default().to_owned()),
            Some("resource_link") => {
                let uri = b["uri"].as_str().unwrap_or_default();
                parts.push(format!("@{}", uri.strip_prefix("file://").unwrap_or(uri)));
            }
            Some("resource") => {
                let r = &b["resource"];
                let uri = r["uri"].as_str().unwrap_or_default().replace('"', "%22");
                parts.push(match r["text"].as_str() {
                    Some(text) => format!(
                        "<context uri=\"{uri}\">
{text}
</context>"
                    ),
                    None => format!("[binary resource {uri}]"),
                });
            }
            _ => {}
        }
    }
    parts
        .join(
            "
",
        )
        .trim_end()
        .to_owned()
}

fn stop_reason(r: &StopReason) -> &'static str {
    match r {
        StopReason::Done(kor_core::FinishReason::MaxTokens) => "max_tokens",
        StopReason::Done(kor_core::FinishReason::ContentFiltered) | StopReason::Blocked(_) => {
            "refusal"
        }
        StopReason::Done(_) => "end_turn",
        StopReason::Cancelled => "cancelled",
        StopReason::MaxTurns | StopReason::Budget => "max_turn_requests",
    }
}

/// `session/update` bodies for one Kor event (`None`: nothing to show).
pub fn update_for(ev: AgentEvent, notices: bool) -> Option<Value> {
    Some(match ev {
        AgentEvent::Text(t) => {
            json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } })
        }
        AgentEvent::ToolStart { id, name, summary } => json!({
            "sessionUpdate": "tool_call", "toolCallId": id,
            "title": if summary.trim().is_empty() { name.clone() } else { summary },
            "kind": tool_kind(&name), "status": "pending", "rawInput": { "tool": name },
        }),
        AgentEvent::ToolProgress { id, message, .. } => {
            let mut u = json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": "in_progress" });
            if let Some(m) = message {
                u["content"] =
                    json!([{ "type": "content", "content": { "type": "text", "text": m } }]);
            }
            u
        }
        AgentEvent::ToolEnd {
            id,
            output,
            is_error,
            ..
        } => json!({
            "sessionUpdate": "tool_call_update", "toolCallId": id,
            "status": if is_error { "failed" } else { "completed" },
            "content": [{ "type": "content", "content": { "type": "text", "text": output } }],
        }),
        AgentEvent::Notice(t) if notices => {
            json!({ "sessionUpdate": "notice", "severity": "info", "title": t })
        }
        // Without `session.notices`, notices travel as thoughts.
        AgentEvent::Thinking(t) | AgentEvent::Notice(t) => {
            json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": t } })
        }
        // Usage, context, status and estimates have no ACP update.
        _ => return None,
    })
}

/// Replay of a loaded session's history.
fn history_updates(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        for p in &m.parts {
            match (&m.role, p) {
                (Role::User, ContentPart::Text { text }) => out.push(json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": text } })),
                (Role::Assistant, ContentPart::Text { text }) => out.push(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } })),
                (Role::Assistant, ContentPart::Thinking { text, .. }) => out.push(json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": text } })),
                (_, ContentPart::ToolUse { id, name, input }) => out.push(json!({
                    "sessionUpdate": "tool_call", "toolCallId": id, "title": name, "kind": tool_kind(name),
                    "status": "completed", "rawInput": input,
                })),
                (_, ContentPart::ToolResult { tool_use_id, content, is_error }) => out.push(json!({
                    "sessionUpdate": "tool_call_update", "toolCallId": tool_use_id,
                    "status": if *is_error { "failed" } else { "completed" },
                    "content": [{ "type": "content", "content": { "type": "text", "text": content } }],
                })),
                _ => {}
            }
        }
    }
    out
}

/// What the client offered in `initialize`.
#[allow(clippy::struct_excessive_bools)] // one flag per ACP client capability
#[derive(Debug, Clone, Copy, Default)]
struct Caps {
    read: bool,
    write: bool,
    terminal: bool,
    notices: bool,
}

/// The editor side of one session: fs and terminal requests to the client.
#[derive(Debug)]
pub struct AcpEditor {
    rpc: Arc<Rpc>,
    session: OnceLock<String>,
    caps: Caps,
    /// Tool call the next command belongs to (its terminal is attached to it).
    current_call: Mutex<Option<String>>,
}

impl AcpEditor {
    fn sid(&self) -> Result<String, String> {
        self.session
            .get()
            .cloned()
            .ok_or_else(|| "no session".to_owned())
    }
}

#[async_trait]
impl EditorBridge for AcpEditor {
    fn can_read(&self) -> bool {
        self.caps.read
    }
    fn can_write(&self) -> bool {
        self.caps.write
    }
    fn can_terminal(&self) -> bool {
        self.caps.terminal
    }

    async fn read_text(&self, path: &Path) -> Result<String, String> {
        let v = self
            .rpc
            .request(
                "fs/read_text_file",
                json!({ "sessionId": self.sid()?, "path": path }),
            )
            .await
            .map_err(|e| e.to_string())?;
        v["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "no content".into())
    }

    async fn write_text(&self, path: &Path, content: &str) -> Result<(), String> {
        self.rpc
            .request(
                "fs/write_text_file",
                json!({ "sessionId": self.sid()?, "path": path, "content": content }),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<(String, Option<i32>), String> {
        let sid = self.sid()?;
        let env: Vec<Value> = env
            .iter()
            .map(|(k, v)| json!({ "name": k, "value": v }))
            .collect();
        let t = self
            .rpc
            .request(
                "terminal/create",
                json!({ "sessionId": sid, "command": program, "args": args, "env": env,
                        "cwd": cwd, "outputByteLimit": 1_000_000 }),
            )
            .await
            .map_err(|e| e.to_string())?;
        let tid = t["terminalId"].as_str().ok_or("no terminal id")?.to_owned();
        let call = self
            .current_call
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(call) = call {
            self.rpc.notify(
                "session/update",
                json!({ "sessionId": sid, "update": { "sessionUpdate": "tool_call_update", "toolCallId": call,
                        "status": "in_progress", "content": [{ "type": "terminal", "terminalId": tid }] } }),
            );
        }
        let ids = json!({ "sessionId": sid, "terminalId": tid });
        let waited = tokio::select! {
            r = self.rpc.request("terminal/wait_for_exit", ids.clone()) => Some(r),
            () = tokio::time::sleep(timeout) => None,
            () = cancel.cancelled() => None,
        };
        if waited.is_none() {
            let _ = self.rpc.request("terminal/kill", ids.clone()).await;
        }
        let out = self
            .rpc
            .request("terminal/output", ids.clone())
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.rpc.request("terminal/release", ids).await;
        let code = match waited {
            Some(Ok(v)) => v["exitCode"].as_i64().and_then(|c| i32::try_from(c).ok()),
            _ => None,
        };
        Ok((out["output"].as_str().unwrap_or_default().to_owned(), code))
    }
}

/// Permission cards as `session/request_permission`.
/// Turns queued agent events into `session/update`s. The forwarder and the approver both drain
/// through it, so a permission request never overtakes the tool call it is about.
struct Pump {
    rx: Mutex<mpsc::UnboundedReceiver<AgentEvent>>,
    rpc: Arc<Rpc>,
    session: String,
    notices: bool,
    editor: Arc<AcpEditor>,
}

impl Pump {
    /// Send what is queued; `true` once the sender is gone and the queue is empty.
    fn drain(&self) -> bool {
        let mut rx = lock(&self.rx);
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if let AgentEvent::ToolStart { id, .. } = &ev {
                        *lock(&self.editor.current_call) = Some(id.clone());
                    }
                    if let Some(u) = update_for(ev, self.notices) {
                        self.rpc.notify(
                            "session/update",
                            json!({ "sessionId": self.session, "update": u }),
                        );
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => return false,
                Err(mpsc::error::TryRecvError::Disconnected) => return true,
            }
        }
    }
}

struct AcpApprover {
    rpc: Arc<Rpc>,
    session: String,
    cwd: PathBuf,
    l: Localizer,
    pump: Arc<Pump>,
}

#[async_trait]
impl Approver for AcpApprover {
    fn surface(&self) -> &'static str {
        "editor"
    }

    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.pump.drain();
        let content: Vec<Value> = req
            .files
            .iter()
            .map(|f| {
                json!({ "type": "diff", "path": self.cwd.join(&f.path), "oldText": f.before, "newText": f.after.clone().unwrap_or_default() })
            })
            .collect();
        let mut options = vec![
            json!({ "optionId": "allow", "name": self.l.t("acp-allow-once"), "kind": "allow_once" }),
        ];
        // A destructive call is never remembered as "always".
        if req.destructive.is_none() {
            options.push(json!({ "optionId": "always", "name": self.l.t("acp-allow-always"), "kind": "allow_always" }));
        }
        options.push(
            json!({ "optionId": "reject", "name": self.l.t("acp-reject"), "kind": "reject_once" }),
        );
        let title = if req.summary.trim().is_empty() {
            req.tool.clone()
        } else {
            req.summary.clone()
        };
        let r = self
            .rpc
            .request(
                "session/request_permission",
                json!({ "sessionId": self.session, "options": options,
                        "toolCall": { "toolCallId": req.id, "title": title, "kind": tool_kind(&req.tool),
                                      "status": "pending", "content": content } }),
            )
            .await;
        let picked = r
            .ok()
            .filter(|v| v["outcome"]["outcome"] == "selected")
            .and_then(|v| v["outcome"]["optionId"].as_str().map(str::to_owned));
        match picked.as_deref() {
            Some("allow") => PermissionReply::Allow,
            Some("always") => PermissionReply::AllowAlways,
            _ => PermissionReply::Deny,
        }
    }
}

struct Entry {
    session: Arc<tokio::sync::Mutex<Box<dyn Session>>>,
    cancel: Arc<Mutex<Option<CancellationToken>>>,
    cwd: PathBuf,
    editor: Arc<AcpEditor>,
}

/// The ACP agent handler.
pub struct AcpAgent {
    l: Localizer,
    engine: Arc<dyn Engine>,
    caps: Mutex<Option<Caps>>,
    sessions: Mutex<HashMap<String, Arc<Entry>>>,
}

impl std::fmt::Debug for AcpAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAgent").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, msg)
}

/// `mcpServers` of `session/new|load`: stdio servers (HTTP/SSE are not advertised).
fn mcp_specs(v: &Value) -> Result<Vec<ServerSpec>, RpcError> {
    let Some(list) = v.as_array() else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|s| {
            if s.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t != "stdio")
            {
                return Err(invalid("only stdio MCP servers are supported"));
            }
            Ok(ServerSpec {
                name: s["name"]
                    .as_str()
                    .ok_or_else(|| invalid("mcp server without name"))?
                    .to_owned(),
                command: s["command"]
                    .as_str()
                    .ok_or_else(|| invalid("mcp server without command"))?
                    .to_owned(),
                args: s["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                env: s["env"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                Some((
                                    e["name"].as_str()?.to_owned(),
                                    e["value"].as_str()?.to_owned(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                origin: "ACP client".into(),
            })
        })
        .collect()
}

impl AcpAgent {
    pub fn new(l: Localizer, engine: Arc<dyn Engine>) -> Self {
        Self {
            l,
            engine,
            caps: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn entry(&self, params: &Value) -> Result<Arc<Entry>, RpcError> {
        let id = params["sessionId"].as_str().unwrap_or_default();
        lock(&self.sessions).get(id).cloned().ok_or_else(|| {
            RpcError::new(RESOURCE_NOT_FOUND, "unknown session")
                .with_data(json!({ "sessionId": id }))
        })
    }

    async fn open(
        &self,
        rpc: Arc<Rpc>,
        params: &Value,
        resume: Option<&str>,
    ) -> Result<(String, Arc<Entry>), RpcError> {
        let cwd = PathBuf::from(
            params["cwd"]
                .as_str()
                .ok_or_else(|| invalid("cwd is required"))?,
        );
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(invalid("cwd must be an existing absolute directory"));
        }
        let mcp = mcp_specs(&params["mcpServers"])?;
        let caps = lock(&self.caps).unwrap_or_default();
        let editor = Arc::new(AcpEditor {
            rpc,
            session: OnceLock::new(),
            caps,
            current_call: Mutex::new(None),
        });
        let s = self
            .engine
            .open(&cwd, resume, mcp, editor.clone())
            .await
            .map_err(|e| RpcError::new(INTERNAL_ERROR, e))?;
        let id = s.id();
        let _ = editor.session.set(id.clone());
        let entry = Arc::new(Entry {
            session: Arc::new(tokio::sync::Mutex::new(s)),
            cancel: Arc::default(),
            cwd,
            editor,
        });
        lock(&self.sessions).insert(id.clone(), entry.clone());
        Ok((id, entry))
    }

    async fn prompt(&self, rpc: Arc<Rpc>, params: &Value) -> Result<Value, RpcError> {
        let e = self.entry(params)?;
        let sid = params["sessionId"].as_str().unwrap_or_default().to_owned();
        let blocks = params["prompt"]
            .as_array()
            .ok_or_else(|| invalid("prompt is required"))?;
        let text = prompt_text(blocks);
        if text.is_empty() {
            return Err(invalid("empty prompt"));
        }
        let cancel = CancellationToken::new();
        *lock(&e.cancel) = Some(cancel.clone());
        let notices = lock(&self.caps).is_some_and(|c| c.notices);
        let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
        let pump = Arc::new(Pump {
            rx: Mutex::new(rx),
            rpc: rpc.clone(),
            session: sid.clone(),
            notices,
            editor: e.editor.clone(),
        });
        let fwd = pump.clone();
        let forward = tokio::spawn(async move {
            while !fwd.drain() {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        });
        let approver = AcpApprover {
            rpc,
            session: sid.clone(),
            cwd: e.cwd.clone(),
            l: Localizer::new(self.l.lang()),
            pump,
        };
        let mut s = e.session.lock().await;
        let r = s.prompt(text, &tx, &approver, cancel.clone()).await;
        drop(s);
        drop(tx);
        // Every update of the turn reaches the client before the response.
        let _ = forward.await;
        *lock(&e.cancel) = None;
        if cancel.is_cancelled() {
            // An aborted turn is `cancelled`, never an error (ACP prompt-turn rules).
            return Ok(json!({ "stopReason": "cancelled" }));
        }
        match r {
            Ok(stop) => Ok(json!({ "stopReason": stop_reason(&stop) })),
            Err(msg) => Err(RpcError::new(INTERNAL_ERROR, msg)),
        }
    }
}

#[async_trait]
impl Handler for AcpAgent {
    async fn request(&self, rpc: Arc<Rpc>, method: &str, params: Value) -> Result<Value, RpcError> {
        if method == "initialize" {
            let asked = params["protocolVersion"]
                .as_u64()
                .unwrap_or(PROTOCOL_VERSION);
            let c = &params["clientCapabilities"];
            *lock(&self.caps) = Some(Caps {
                read: c["fs"]["readTextFile"].as_bool() == Some(true),
                write: c["fs"]["writeTextFile"].as_bool() == Some(true),
                terminal: c["terminal"].as_bool() == Some(true),
                notices: c["session"]["notices"].as_bool() == Some(true),
            });
            return Ok(json!({
                "protocolVersion": asked.min(PROTOCOL_VERSION),
                "agentCapabilities": {
                    "loadSession": true,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": true },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": { "name": "kor", "title": "Kor", "version": kor_core::VERSION },
                "authMethods": [],
            }));
        }
        if lock(&self.caps).is_none() {
            return Err(RpcError::new(INVALID_REQUEST, "initialize first"));
        }
        match method {
            "authenticate" => Ok(json!({})),
            "session/new" => {
                let (id, e) = self.open(rpc, &params, None).await?;
                let mode = e.session.lock().await.mode();
                Ok(json!({ "sessionId": id, "modes": modes_json(&self.l, mode) }))
            }
            "session/load" => {
                let want = params["sessionId"].as_str().unwrap_or_default().to_owned();
                if !crate::valid_session_id(&want) {
                    return Err(RpcError::new(RESOURCE_NOT_FOUND, "unknown session")
                        .with_data(json!({ "sessionId": want })));
                }
                let (id, e) = self.open(rpc.clone(), &params, Some(&want)).await?;
                let s = e.session.lock().await;
                for u in history_updates(&s.history()) {
                    rpc.notify("session/update", json!({ "sessionId": id, "update": u }));
                }
                Ok(json!({ "modes": modes_json(&self.l, s.mode()) }))
            }
            "session/prompt" => self.prompt(rpc, &params).await,
            "session/set_mode" => {
                let e = self.entry(&params)?;
                let want = params["modeId"].as_str().unwrap_or_default();
                let mode = MODES
                    .into_iter()
                    .find(|m| m.as_str() == want)
                    .ok_or_else(|| invalid(format!("unknown mode {want}")))?;
                e.session.lock().await.set_mode(mode);
                rpc.notify(
                    "session/update",
                    json!({ "sessionId": params["sessionId"], "update": { "sessionUpdate": "current_mode_update", "currentModeId": mode.as_str() } }),
                );
                Ok(json!({}))
            }
            _ => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("unknown method {method}"),
            )),
        }
    }

    async fn notification(&self, _rpc: Arc<Rpc>, method: &str, params: Value) {
        if method == "session/cancel"
            && let Ok(e) = self.entry(&params)
            && let Some(c) = lock(&e.cancel).as_ref()
        {
            c.cancel();
        }
    }
}

/// Production sessions: `prepare` in the editor's directory, the configured and the editor's
/// MCP servers, the editor bridge on the tools, the transcript saved after every turn.
pub struct KorEngine {
    pub l: Localizer,
    pub loaded: kor_config::Loaded,
}

impl std::fmt::Debug for KorEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KorEngine").finish_non_exhaustive()
    }
}

struct KorSession {
    id: String,
    l: Localizer,
    prepared: crate::Prepared,
}

#[async_trait]
impl Session for KorSession {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn history(&self) -> Vec<Message> {
        self.prepared.agent.messages.clone()
    }
    fn mode(&self) -> PermissionMode {
        self.prepared.agent.permissions.mode
    }
    fn set_mode(&mut self, mode: PermissionMode) {
        self.prepared.agent.permissions.mode = mode;
        if let Ok(mut p) = self.prepared.agent.ctx.plan.lock() {
            *p = mode;
        }
    }
    async fn prompt(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<StopReason, String> {
        let p = &mut self.prepared;
        crate::refresh_memory(&mut p.agent);
        let r = p.agent.send(text, events, approver, cancel).await;
        if let Some(path) = &p.path {
            kor_agent::session::save(path, &p.agent.messages).map_err(|e| e.to_string())?;
        }
        r.map_err(|e| crate::provider_error(&self.l, &e))
    }
}

#[async_trait]
impl Engine for KorEngine {
    async fn open(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        mcp: Vec<ServerSpec>,
        editor: Arc<dyn EditorBridge>,
    ) -> Result<Box<dyn Session>, String> {
        let root = cwd.canonicalize().map_err(|e| e.to_string())?;
        if let Some(id) = resume {
            let exists =
                crate::sessions_dir().is_some_and(|d| d.join(format!("{id}.jsonl")).is_file());
            if !exists {
                return Err(format!("no session {id}"));
            }
        }
        let opts = crate::Options {
            root: kor_tools::without_verbatim(&root),
            model: None,
            mode: None,
            resume: resume.map(|id| crate::Resume::Id(id.to_owned())),
            new_id: None,
            surface: kor_agent::prompt::Surface::Terminal,
        };
        let mut notes = Vec::new();
        let mut p = crate::prepare(&self.l, &self.loaded, &opts, &mut notes)?;
        let _ = crate::attach_mcp(&self.l, &mut p.agent, &p.compat).await;
        if !mcp.is_empty() {
            let (tools, _) = kor_agent::mcp_tools::connect_all(&mcp, &p.agent.ctx.root).await;
            p.agent.tools.extend(tools);
        }
        p.agent.ctx.editor = Some(editor);
        let id = p
            .path
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| resume.map(str::to_owned))
            .unwrap_or_else(|| {
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                format!("acp-{ms}")
            });
        Ok(Box::new(KorSession {
            id,
            l: Localizer::new(self.l.lang()),
            prepared: p,
        }))
    }
}
