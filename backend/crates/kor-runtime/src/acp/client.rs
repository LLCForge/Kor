//! Kor as an ACP **client** (spec §28.8, ADR 0057): another ACP agent (any program speaking the
//! Agent Client Protocol on stdio) runs as a sub-agent inside Kor. Kor starts it, initializes
//! protocol v1, opens a session in the project, sends the prompt, shows its streamed updates and
//! answers its requests:
//!
//! - `fs/read_text_file` / `fs/write_text_file`: inside the project root only (Kor's jail), and a
//!   write that would put a secret into the team memory folder is refused like Kor's own writes;
//! - `session/request_permission`: asked through [`ClientUi::permission`] (the CLI asks the user;
//!   the sub-agent tool rejects, since a tool cannot ask mid-call);
//! - terminals are not offered (the sub-agent's own commands run in its own process; Kor cannot
//!   sandbox them, so it does not lend it a terminal).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::rpc::{self, Handler, INVALID_PARAMS, METHOD_NOT_FOUND, RESOURCE_NOT_FOUND, Rpc, RpcError};

/// An ACP agent program (`[acp.agents.<name>]` in the config, or given on the command line).
pub use kor_config::AcpAgent as AgentSpec;

/// What the user sees and answers.
#[async_trait]
pub trait ClientUi: Send + Sync {
    /// One `session/update` body.
    fn update(&self, update: &Value);
    /// The `optionId` to pick for a permission request, `None` = cancelled.
    async fn permission(&self, tool_call: &Value, options: &[Value]) -> Option<String>;
}

struct ClientSide {
    root: PathBuf,
    ui: Arc<dyn ClientUi>,
}

fn param<'a>(p: &'a Value, k: &str) -> Result<&'a str, RpcError> {
    p[k].as_str()
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("{k} is required")))
}

impl ClientSide {
    fn path(&self, p: &str) -> Result<PathBuf, RpcError> {
        let rel = Path::new(p)
            .strip_prefix(&self.root)
            .map_or_else(|_| p.to_owned(), |r| r.to_string_lossy().into_owned());
        kor_tools::resolve(&self.root, &rel).map_err(|e| RpcError::new(INVALID_PARAMS, e))
    }
}

#[async_trait]
impl Handler for ClientSide {
    async fn request(&self, _rpc: Arc<Rpc>, method: &str, p: Value) -> Result<Value, RpcError> {
        match method {
            "session/request_permission" => {
                let options = p["options"].as_array().cloned().unwrap_or_default();
                let picked = self.ui.permission(&p["toolCall"], &options).await;
                Ok(match picked {
                    Some(id) if options.iter().any(|o| o["optionId"] == id.as_str()) => {
                        json!({ "outcome": { "outcome": "selected", "optionId": id } })
                    }
                    _ => json!({ "outcome": { "outcome": "cancelled" } }),
                })
            }
            "fs/read_text_file" => {
                let path = self.path(param(&p, "path")?)?;
                let text = tokio::fs::read_to_string(&path).await.map_err(|_| {
                    RpcError::new(RESOURCE_NOT_FOUND, "file not found")
                        .with_data(json!({ "path": path }))
                })?;
                let start = usize::try_from(p["line"].as_u64().unwrap_or(1).max(1) - 1).unwrap_or(0);
                let content = match p["limit"].as_u64() {
                    Some(n) => text
                        .lines()
                        .skip(start)
                        .take(usize::try_from(n).unwrap_or(usize::MAX))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None if start > 0 => text.lines().skip(start).collect::<Vec<_>>().join("\n"),
                    None => text,
                };
                Ok(json!({ "content": content }))
            }
            "fs/write_text_file" => {
                let path = self.path(param(&p, "path")?)?;
                let content = param(&p, "content")?;
                // Same rule as Kor's own writes: no secrets into the shared team memory.
                if crate::memory_layout(&self.root)
                    .is_some_and(|l| path.starts_with(l.auto_dir.join("team")))
                    && !kor_tools::scan_secrets(content).is_empty()
                {
                    return Err(RpcError::new(INVALID_PARAMS, "refused: the content looks like a secret"));
                }
                if let Some(d) = path.parent() {
                    tokio::fs::create_dir_all(d)
                        .await
                        .map_err(|e| RpcError::new(rpc::INTERNAL_ERROR, e.to_string()))?;
                }
                tokio::fs::write(&path, content)
                    .await
                    .map_err(|e| RpcError::new(rpc::INTERNAL_ERROR, e.to_string()))?;
                Ok(json!({}))
            }
            _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("unknown method {method}"))),
        }
    }

    async fn notification(&self, _rpc: Arc<Rpc>, method: &str, p: Value) {
        if method == "session/update" {
            self.ui.update(&p["update"]);
        }
    }
}

/// A running ACP agent with one open session.
pub struct AcpClient {
    rpc: Arc<Rpc>,
    child: Option<tokio::process::Child>,
    pub agent_info: Value,
    pub session: String,
}

impl std::fmt::Debug for AcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpClient")
            .field("agent", &self.agent_info)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl AcpClient {
    /// Over any byte streams (tests), then `initialize` + `session/new` in `root`.
    ///
    /// # Errors
    /// When the agent does not speak protocol v1 or refuses the session.
    pub async fn over<R, W>(reader: R, writer: W, root: &Path, ui: Arc<dyn ClientUi>) -> Result<Self, String>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let handler = Arc::new(ClientSide {
            root: root.to_path_buf(),
            ui,
        });
        let (rpc, run) = rpc::connect(reader, writer, handler);
        tokio::spawn(run);
        let init = rpc
            .request(
                "initialize",
                json!({ "protocolVersion": super::agent::PROTOCOL_VERSION,
                        "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
                        "clientInfo": { "name": "kor", "title": "Kor", "version": kor_core::VERSION } }),
            )
            .await
            .map_err(|e| e.to_string())?;
        if init["protocolVersion"].as_u64() != Some(super::agent::PROTOCOL_VERSION) {
            return Err(format!(
                "the agent speaks ACP {} (Kor speaks {})",
                init["protocolVersion"],
                super::agent::PROTOCOL_VERSION
            ));
        }
        let s = rpc
            .request("session/new", json!({ "cwd": root, "mcpServers": [] }))
            .await
            .map_err(|e| e.to_string())?;
        let session = s["sessionId"].as_str().ok_or("the agent gave no session id")?.to_owned();
        Ok(Self {
            rpc,
            child: None,
            agent_info: init["agentInfo"].clone(),
            session,
        })
    }

    /// Start `spec` in `root` (privacy/offline mode: refused — the agent is a program that may
    /// reach the network).
    ///
    /// # Errors
    /// When it cannot start or does not initialize.
    pub async fn spawn(name: &str, spec: &AgentSpec, root: &Path, ui: Arc<dyn ClientUi>) -> Result<Self, String> {
        if let Some(b) = kor_core::net::refusal(name) {
            return Err(b.message);
        }
        if spec.command.trim().is_empty() {
            return Err(format!("ACP agent {name} has no command"));
        }
        let mut cmd = tokio::process::Command::new(&spec.command);
        cmd.args(&spec.args)
            .envs(&spec.env)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", spec.command))?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let inp = child.stdin.take().ok_or("no stdin")?;
        let mut c = Self::over(out, inp, root, ui).await?;
        c.child = Some(child);
        Ok(c)
    }

    /// One prompt; the stop reason (`end_turn`, `cancelled`, …).
    ///
    /// # Errors
    /// The agent's error.
    pub async fn prompt(&self, text: &str) -> Result<String, String> {
        let r = self
            .rpc
            .request(
                "session/prompt",
                json!({ "sessionId": self.session, "prompt": [{ "type": "text", "text": text }] }),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(r["stopReason"].as_str().unwrap_or("end_turn").to_owned())
    }

    pub fn cancel(&self) {
        self.rpc
            .notify("session/cancel", json!({ "sessionId": self.session }));
    }
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.start_kill();
        }
    }
}

/// Collects the agent's answer text; permission requests get `answer` (or are rejected).
#[derive(Debug, Default)]
pub struct Collector {
    pub text: std::sync::Mutex<String>,
    pub tools: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl ClientUi for Collector {
    fn update(&self, u: &Value) {
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                if let Some(t) = u["content"]["text"].as_str()
                    && let Ok(mut s) = self.text.lock()
                {
                    s.push_str(t);
                }
            }
            Some("tool_call") => {
                if let Some(t) = u["title"].as_str()
                    && let Ok(mut v) = self.tools.lock()
                {
                    v.push(t.to_owned());
                }
            }
            _ => {}
        }
    }

    async fn permission(&self, _tool_call: &Value, options: &[Value]) -> Option<String> {
        // A tool cannot ask the user mid-call: reject (the sub-agent reports what it needed).
        options
            .iter()
            .find(|o| o["kind"] == "reject_once")
            .and_then(|o| o["optionId"].as_str().map(str::to_owned))
    }
}

/// The `AcpAgent` tool: run a configured ACP agent as a sub-agent with a prompt; its answer is
/// the tool output. An `execute` tool, so the user approves each run.
pub struct AcpAgentTool {
    pub agents: std::collections::BTreeMap<String, AgentSpec>,
}

impl std::fmt::Debug for AcpAgentTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAgentTool")
            .field("agents", &self.agents.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[async_trait]
impl kor_tools::Tool for AcpAgentTool {
    fn name(&self) -> &'static str {
        "AcpAgent"
    }
    fn rule_name(&self) -> &'static str {
        "AcpAgent"
    }
    fn kind(&self) -> kor_tools::ToolKind {
        kor_tools::ToolKind::Execute
    }
    fn spec(&self) -> kor_core::ToolSpec {
        let names: Vec<&str> = self.agents.keys().map(String::as_str).collect();
        kor_core::ToolSpec {
            name: "AcpAgent".into(),
            description: format!(
                "Run another coding agent that speaks the Agent Client Protocol as a sub-agent in \
                 this project and return its answer. It can read and write project files through \
                 Kor; its permission requests are rejected. Configured agents: {}.",
                names.join(", ")
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "enum": names },
                    "prompt": { "type": "string", "description": "The task for the sub-agent." }
                },
                "required": ["agent", "prompt"],
                "additionalProperties": false
            }),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["agent"].as_str().map(|a| format!("acp:{a}"))
    }
    fn summary(&self, input: &Value) -> String {
        format!(
            "{}: {}",
            input["agent"].as_str().unwrap_or_default(),
            kor_agent::audit::clip(input["prompt"].as_str().unwrap_or_default())
        )
    }
    async fn call(&self, input: Value, ctx: &kor_tools::ToolCtx) -> kor_tools::ToolOutput {
        let name = input["agent"].as_str().unwrap_or_default();
        let Some(spec) = self.agents.get(name) else {
            return kor_tools::ToolOutput::err(format!("no ACP agent named {name}"));
        };
        let prompt = input["prompt"].as_str().unwrap_or_default();
        let ui = Arc::new(Collector::default());
        let client = match AcpClient::spawn(name, spec, &ctx.root, ui.clone()).await {
            Ok(c) => c,
            Err(e) => return kor_tools::ToolOutput::err(e),
        };
        let stop = tokio::select! {
            r = client.prompt(prompt) => r,
            () = ctx.cancel.cancelled() => {
                client.cancel();
                Ok("cancelled".into())
            }
        };
        let text = ui.text.lock().map(|s| s.clone()).unwrap_or_default();
        match stop {
            Ok(reason) => kor_tools::ToolOutput::ok(format!("{text}\n[{name}: {reason}]")),
            Err(e) => kor_tools::ToolOutput::err(format!("{text}\n[{name}: {e}]")),
        }
    }
}
