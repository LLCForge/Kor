//! Faz 5 acceptance (§14, ADR 0033): with the sandbox on, a command the model runs tries to write
//! outside the project. The OS blocks it, the tool result says so, and the audit log gets a
//! `sandbox_denied` entry that verifies in the hash chain.
//!
//! Runs against this machine's native backend: `AppContainer` on Windows, bubblewrap on Linux,
//! Seatbelt on macOS (CI). Skips, with a message, where none is installed. The branch is chosen
//! at run time (`kor_sandbox::native()`), so no `cfg` is needed outside kor-platform.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, Permissions};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_sandbox::SandboxKind;
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::json;

struct Scripted {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
}

#[async_trait]
impl Provider for Scripted {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        _req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("extra model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

struct AllowAll;

#[async_trait]
impl Approver for AllowAll {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
}

fn bash(command: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: "c1".into(),
            name: "Bash".into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: json!({ "command": command }).to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

fn say(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta { text: text.into() },
        StreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ]
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one acceptance scenario reads better unsplit
async fn write_outside_the_project_is_blocked_and_audited() {
    let kind = kor_sandbox::native();
    if !kind.is_enforcing() {
        eprintln!("skipped: no native sandbox backend (bwrap / sandbox-exec / AppContainer) here");
        return;
    }
    let project = tempfile::tempdir().unwrap();
    // Outside the project and outside the sandbox's private temp: the user's temp folder on
    // Windows (it is in the profile), the home folder elsewhere (bwrap and Seatbelt let `/tmp`
    // through).
    let outside = if kind == SandboxKind::AppContainer {
        tempfile::tempdir().unwrap()
    } else {
        let home = std::env::var_os("HOME").expect("HOME");
        tempfile::tempdir_in(home).unwrap()
    };
    let target = outside.path().join("planted.txt");
    let command = if kind == SandboxKind::AppContainer {
        format!(
            "Set-Content -LiteralPath '{}' -Value pwned",
            target.display()
        )
    } else {
        format!("echo pwned > '{}'", target.display())
    };
    let log_dir = tempfile::tempdir().unwrap();
    let log = log_dir.path().join(kor_agent::audit::FILE);
    kor_agent::audit::set_path(Some(log.clone()));

    let mut ctx = ToolCtx::new(project.path()).unwrap();
    ctx.sandbox = kind;
    ctx.network = false;
    let provider = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(vec![bash(&command), say("done")].into()),
    });
    let mut agent = Agent {
        provider,
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::BypassPermissions, &[], &[], &[]).unwrap(),
        ctx,
        system: "sys".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 5,
        max_tokens: None,
        reasoning: None,
        redactor: Redactor::default(),
        hooks: None,
        rules: Vec::new(),
        auto_compact: false,
        context_window: kor_agent::compact::DEFAULT_CONTEXT_WINDOW,
        memory_extract: false,
        memory_base: None,
        session_notes: None,
        output_budget: None,
        dream: false,
        tool_mode: kor_agent::toolcall::ToolMode::Native,
        verify: kor_agent::verify::Verify::default(),
        stats: kor_agent::repair::ToolStats::default(),
        context: kor_agent::context::Budget::default(),
        cost: None,
        shadow: None,
        repair: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent
        .send(
            "plant a file".into(),
            &tx,
            &AllowAll,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    kor_agent::audit::set_path(None);

    // 1. The OS blocked the write.
    assert!(
        !target.exists(),
        "the sandbox let a write through to {}",
        target.display()
    );

    // 2. The tool result and a notice tell the user.
    let (mut result, mut notice) = (String::new(), String::new());
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::ToolEnd { output, .. } => result = output,
            AgentEvent::Notice(n) if n.starts_with("sandbox:") => notice = n,
            _ => {}
        }
    }
    let blocked_by = format!("[sandbox] blocked by {}", kind.as_str());
    assert!(result.contains(&blocked_by), "{result}");
    assert!(notice.contains(kind.as_str()), "{notice:?}");

    // 3. The audit log has the block, and its hash chain holds.
    let page = kor_agent::audit::read(
        &log,
        &kor_agent::audit::Query {
            action: Some("sandbox_denied".into()),
            limit: 10,
            ..kor_agent::audit::Query::default()
        },
    )
    .unwrap();
    assert_eq!(page.total, 1, "{:?}", page.entries);
    let e = &page.entries[0];
    assert_eq!(e["backend"], kind.as_str());
    assert_eq!(e["kind"], "file");
    assert_eq!(e["tool"], "Bash");
    let path = e["path"].as_str().unwrap_or_default();
    assert!(path.ends_with("planted.txt"), "{e}");
    let v = kor_agent::audit::verify(&log).unwrap();
    assert!(v.ok() && v.entries >= 3, "{v:?}"); // tool, sandbox_denied, task
}
