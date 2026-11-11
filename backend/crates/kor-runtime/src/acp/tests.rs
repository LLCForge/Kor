//! An ACP client harness against Kor's ACP agent (scripted engine, no model): initialize and
//! version negotiation, sessions, prompts with streamed updates, permission requests with
//! diffs, the editor file system, modes, cancellation and errors; then Kor's own ACP client
//! driving the same agent as a sub-agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::mcp_tools::ServerSpec;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest, StopReason};
use kor_core::{FinishReason, Message, PermissionMode};
use kor_providers::CancellationToken;
use kor_tools::EditorBridge;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;

use super::agent::{AcpAgent, Engine, Session};
use super::client::{AcpClient, Collector};
use super::rpc;

struct Scripted {
    cwd: PathBuf,
    editor: Arc<dyn EditorBridge>,
    mode: PermissionMode,
}

#[async_trait]
impl Session for Scripted {
    fn id(&self) -> String {
        "s-1".into()
    }
    fn history(&self) -> Vec<Message> {
        vec![Message::text(kor_core::Role::User, "earlier question")]
    }
    fn mode(&self) -> PermissionMode {
        self.mode
    }
    fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }
    async fn prompt(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<StopReason, String> {
        if text.contains("wait") {
            cancel.cancelled().await;
            return Ok(StopReason::Cancelled);
        }
        let seen = if self.editor.can_read() {
            self.editor.read_text(&self.cwd.join("in.txt")).await.unwrap_or_default()
        } else {
            String::new()
        };
        let _ = events.send(AgentEvent::Text(format!("echo: {text} | {seen}")));
        let _ = events.send(AgentEvent::ToolStart {
            id: "t1".into(),
            name: "Write".into(),
            summary: "a.txt".into(),
        });
        let req = PermissionRequest {
            id: "t1".into(),
            tool: "Write".into(),
            summary: "a.txt".into(),
            files: vec![kor_tools::Proposal {
                path: "a.txt".into(),
                before: None,
                after: Some("new text".into()),
            }],
            ..PermissionRequest::default()
        };
        let allowed = approver.review(&req).await.reply != PermissionReply::Deny;
        if allowed {
            self.editor.write_text(&self.cwd.join("a.txt"), "new text").await?;
        }
        let _ = events.send(AgentEvent::ToolEnd {
            id: "t1".into(),
            name: "Write".into(),
            output: if allowed { "wrote a.txt" } else { "denied" }.into(),
            is_error: !allowed,
        });
        Ok(StopReason::Done(FinishReason::Stop))
    }
}

struct ScriptedEngine;

#[async_trait]
impl Engine for ScriptedEngine {
    async fn open(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        _mcp: Vec<ServerSpec>,
        editor: Arc<dyn EditorBridge>,
    ) -> Result<Box<dyn Session>, String> {
        if resume.is_some_and(|r| r != "s-1") {
            return Err("no such session".into());
        }
        Ok(Box::new(Scripted {
            cwd: cwd.to_path_buf(),
            editor,
            mode: PermissionMode::Default,
        }))
    }
}

fn agent() -> Arc<AcpAgent> {
    Arc::new(AcpAgent::new(
        kor_i18n::Localizer::new(kor_i18n::Lang::En),
        Arc::new(ScriptedEngine),
    ))
}

/// The editor end, speaking raw JSON-RPC lines.
struct Harness {
    w: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    next: u64,
}

impl Harness {
    fn start() -> Self {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        let (ar, aw) = tokio::io::split(theirs);
        let (_rpc, run) = rpc::connect(ar, aw, agent());
        tokio::spawn(run);
        let (r, w) = tokio::io::split(ours);
        Self {
            w,
            lines: BufReader::new(r).lines(),
            next: 1,
        }
    }

    async fn send(&mut self, v: Value) {
        let mut s = v.to_string();
        s.push('\n');
        self.w.write_all(s.as_bytes()).await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let l = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("agent answers in time")
            .unwrap()
            .unwrap();
        serde_json::from_str(&l).unwrap()
    }

    async fn call(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    /// Messages until the response to `id`.
    async fn until(&mut self, id: u64) -> (Vec<Value>, Value) {
        let mut other = Vec::new();
        loop {
            let v = self.recv().await;
            if v["id"] == id && v.get("method").is_none() {
                return (other, v);
            }
            other.push(v);
        }
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn acp_client_harness_against_kor() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let mut h = Harness::start();

    // Before initialize everything but initialize is refused.
    let id = h.call("session/new", json!({ "cwd": cwd, "mcpServers": [] })).await;
    assert_eq!(h.until(id).await.1["error"]["code"], -32_600);

    // Version negotiation: a newer client gets Kor's version back.
    let id = h
        .call(
            "initialize",
            json!({ "protocolVersion": 7, "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
                    "clientInfo": { "name": "harness", "version": "1" } }),
        )
        .await;
    let init = h.until(id).await.1["result"].clone();
    assert_eq!(init["protocolVersion"], 1);
    assert_eq!(init["agentCapabilities"]["loadSession"], true);
    assert_eq!(init["agentCapabilities"]["promptCapabilities"]["embeddedContext"], true);
    assert_eq!(init["agentInfo"]["name"], "kor");
    assert!(init["authMethods"].as_array().unwrap().is_empty());

    // Sessions need an absolute, existing cwd.
    let id = h.call("session/new", json!({ "cwd": "relative/dir", "mcpServers": [] })).await;
    assert_eq!(h.until(id).await.1["error"]["code"], -32_602);
    let id = h.call("session/new", json!({ "cwd": cwd, "mcpServers": [] })).await;
    let new = h.until(id).await.1["result"].clone();
    let sid = new["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(new["modes"]["currentModeId"], "default");
    assert_eq!(new["modes"]["availableModes"].as_array().unwrap().len(), 3);

    // A prompt: the agent reads the editor's buffer, streams, asks permission with a diff,
    // writes through the editor once allowed.
    let id = h
        .call(
            "session/prompt",
            json!({ "sessionId": sid, "prompt": [
                { "type": "text", "text": "hello" },
                { "type": "resource", "resource": { "uri": "file:///x/notes.md", "text": "ctx" } } ] }),
        )
        .await;
    let read = h.recv().await;
    assert_eq!(read["method"], "fs/read_text_file");
    assert_eq!(read["params"]["sessionId"], sid.as_str());
    h.send(json!({ "jsonrpc": "2.0", "id": read["id"], "result": { "content": "unsaved buffer" } })).await;
    let mut updates = Vec::new();
    let perm = loop {
        let v = h.recv().await;
        if v["method"] == "session/request_permission" {
            break v;
        }
        updates.push(v);
    };
    let chunk = updates.iter().find(|u| u["params"]["update"]["sessionUpdate"] == "agent_message_chunk").unwrap();
    let text = chunk["params"]["update"]["content"]["text"].as_str().unwrap();
    assert!(text.contains("hello") && text.contains("ctx") && text.contains("unsaved buffer"), "{text}");
    let call = updates.iter().find(|u| u["params"]["update"]["sessionUpdate"] == "tool_call").unwrap();
    assert_eq!(call["params"]["update"]["kind"], "edit");
    let p = &perm["params"];
    assert_eq!(p["toolCall"]["toolCallId"], "t1");
    assert_eq!(p["toolCall"]["content"][0]["type"], "diff");
    assert_eq!(p["toolCall"]["content"][0]["newText"], "new text");
    assert!(p["toolCall"]["content"][0]["oldText"].is_null());
    let kinds: Vec<&str> = p["options"].as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["allow_once", "allow_always", "reject_once"]);
    h.send(json!({ "jsonrpc": "2.0", "id": perm["id"], "result": { "outcome": { "outcome": "selected", "optionId": "allow" } } })).await;
    let write = h.recv().await;
    assert_eq!(write["method"], "fs/write_text_file");
    assert_eq!(write["params"]["content"], "new text");
    h.send(json!({ "jsonrpc": "2.0", "id": write["id"], "result": {} })).await;
    let (updates_after, response) = h.until(id).await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    let done = updates_after.iter().find(|u| u["params"]["update"]["sessionUpdate"] == "tool_call_update").unwrap();
    assert_eq!(done["params"]["update"]["status"], "completed");

    // Rejected: nothing written, the tool call fails.
    let id = h.call("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "again" }] })).await;
    let read2 = h.recv().await;
    h.send(json!({ "jsonrpc": "2.0", "id": read2["id"], "error": { "code": -32_002, "message": "not open" } })).await;
    let asked = loop {
        let v = h.recv().await;
        if v["method"] == "session/request_permission" {
            break v;
        }
    };
    h.send(json!({ "jsonrpc": "2.0", "id": asked["id"], "result": { "outcome": { "outcome": "cancelled" } } })).await;
    let (after, answer) = h.until(id).await;
    assert_eq!(answer["result"]["stopReason"], "end_turn");
    assert!(after.iter().any(|u| u["params"]["update"]["status"] == "failed"));
    assert!(!after.iter().any(|u| u["method"] == "fs/write_text_file"));

    // Cancellation: `session/cancel` ends the turn with `cancelled`.
    let id = h.call("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "wait here" }] })).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": sid } })).await;
    assert_eq!(h.until(id).await.1["result"]["stopReason"], "cancelled");

    // Modes.
    let id = h.call("session/set_mode", json!({ "sessionId": sid, "modeId": "plan" })).await;
    let (n, r) = h.until(id).await;
    assert!(r.get("result").is_some());
    assert_eq!(n[0]["params"]["update"]["currentModeId"], "plan");
    let id = h.call("session/set_mode", json!({ "sessionId": sid, "modeId": "yolo" })).await;
    assert_eq!(h.until(id).await.1["error"]["code"], -32_602);

    // Load replays history; unknown sessions and methods are errors.
    let id = h.call("session/load", json!({ "sessionId": "s-1", "cwd": cwd, "mcpServers": [] })).await;
    let (n, r) = h.until(id).await;
    assert_eq!(r["result"]["modes"]["currentModeId"], "default");
    assert_eq!(n[0]["params"]["update"]["sessionUpdate"], "user_message_chunk");
    let id = h.call("session/prompt", json!({ "sessionId": "nope", "prompt": [] })).await;
    assert_eq!(h.until(id).await.1["error"]["code"], -32_002);
    let id = h.call("session/fly", json!({})).await;
    assert_eq!(h.until(id).await.1["error"]["code"], -32_601);
}

#[tokio::test]
async fn kor_runs_an_acp_agent_as_a_sub_agent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("in.txt"), "from disk").unwrap();
    // The "other" agent is Kor's own ACP agent (scripted) on an in-memory pipe.
    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let (ar, aw) = tokio::io::split(theirs);
    let (_rpc, run) = rpc::connect(ar, aw, agent());
    tokio::spawn(run);
    let (r, w) = tokio::io::split(ours);
    let ui = Arc::new(Collector::default());
    let c = AcpClient::over(r, w, &root, ui.clone()).await.unwrap();
    assert_eq!(c.agent_info["name"], "kor");
    assert_eq!(c.prompt("summarize").await.unwrap(), "end_turn");
    let text = ui.text.lock().unwrap().clone();
    // The sub-agent read the project file through Kor's client fs.
    assert!(text.contains("from disk"), "{text}");
    // Its write needed permission; a tool cannot ask, so it was rejected and nothing written.
    assert!(!root.join("a.txt").exists());
    assert_eq!(ui.tools.lock().unwrap().as_slice(), ["a.txt"]);
}
