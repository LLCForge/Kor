//! Knowledge base in the agent loop (§31.3) with a scripted model: a finished task leaves a
//! record, the next task gets it in its prompt as the `memory` context category.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::recall::Recall;
use kor_agent::{Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, Permissions};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_memory::{Knowledge, ProjectMemory};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::json;

struct Scripted {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
    seen: Mutex<Vec<ChatRequest>>,
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
        req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        self.seen.lock().unwrap().push(req);
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("extra model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

struct Yes;

#[async_trait]
impl Approver for Yes {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
}

fn say(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta { text: text.into() },
        StreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ]
}

fn write_call(path: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: "w1".into(),
            name: "Write".into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: json!({"file_path": path, "content": "merhaba\n"}).to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end scenario reads better unsplit
async fn finished_task_is_recalled_by_the_next_one() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let kb = Arc::new(Knowledge::open(&home.path().join("knowledge.sqlite")).unwrap());
    let provider = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(
            vec![
                write_call("greeting.txt"),
                say("Wrote greeting.txt with a Turkish greeting.\nNothing else changed."),
                say("Same as before."),
            ]
            .into(),
        ),
        seen: Mutex::new(Vec::new()),
    });
    let mem = ProjectMemory::new(home.path(), root.path());
    let mut agent = Agent {
        provider: provider.clone(),
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::BypassPermissions, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(root.path()).unwrap(),
        system: "sys".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 10,
        max_tokens: None,
        reasoning: None,
        redactor: Redactor::default(),
        hooks: None,
        rules: Vec::new(),
        auto_compact: false,
        context_window: 100_000,
        memory_extract: false,
        memory_base: None,
        session_notes: None,
        output_budget: None,
        dream: false,
        tool_mode: kor_agent::toolcall::ToolMode::Native,
        verify: kor_agent::verify::Verify::default(),
        stats: kor_agent::repair::ToolStats::default(),
        context: kor_agent::context::Budget {
            recall: Some(Recall::new(
                kb.clone(),
                Some(mem.clone()),
                root.path(),
                true,
                500,
                true,
            )),
            ..kor_agent::context::Budget::default()
        },
        cost: None,
        shadow: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    agent
        .send(
            "Write a greeting file".into(),
            &tx,
            &Yes,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    kb.flush().unwrap();
    let hits = kb.search("greeting", None, None, 5).unwrap();
    assert_eq!(hits.len(), 1);
    let r = &hits[0].record;
    assert_eq!(r.task, "Write a greeting file");
    assert_eq!(r.files, ["greeting.txt"]);
    assert!(r.summary.starts_with("Wrote greeting.txt"), "{}", r.summary);
    assert_eq!(r.project, kor_memory::project_key(root.path()));
    let sessions: Vec<_> = std::fs::read_dir(mem.sessions_dir()).unwrap().collect();
    assert_eq!(sessions.len(), 1);

    agent
        .send(
            "greeting again please".into(),
            &tx,
            &Yes,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let seen = provider.seen.lock().unwrap();
    let system = seen.last().unwrap().system.clone().unwrap_or_default();
    assert!(system.contains("# Recalled knowledge"), "{system}");
    assert!(system.contains("Write a greeting file"), "{system}");
    assert_eq!(system.matches("# Recalled knowledge").count(), 1);
    drop(seen);
    let mut memory = 0;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::Context(b) = ev {
            memory = b.memory;
        }
    }
    assert!(
        memory > 0,
        "recalled records are their own context category"
    );
    // A plain conversation turn (no tool call) is not written as knowledge.
    kb.flush().unwrap();
    assert_eq!(kb.count().unwrap(), 1);
}
