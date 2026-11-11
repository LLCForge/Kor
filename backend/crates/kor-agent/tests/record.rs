//! Session recording round trip (§15.9, ADR 0043): a scripted session is written to
//! `.kor/sessions/<id>/` with prompt, text, diff, approval, tool calls and cost, every secret
//! masked, the cast valid asciinema v2, and the front end still gets every event.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, Permissions};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_secrets::{Redactor, SecretString};
use kor_tools::record::{CAST, DIR, EVENTS, Recorder};
use kor_tools::{ToolCtx, default_tools};
use serde_json::{Value, json};

const VAULT: &str = "hunter2-vault-value";
const KEY_HEAD: &str = "sk-ant-api03-";
const KEY_TAIL: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
        _r: ChatRequest,
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

struct Yes;

#[async_trait]
impl Approver for Yes {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
}

fn turns() -> VecDeque<Vec<StreamEvent>> {
    let args = json!({"file_path": "s.txt", "content": format!("token={VAULT}\n")});
    [
        vec![
            // The key arrives split across two deltas.
            StreamEvent::TextDelta {
                text: format!("key {KEY_HEAD}"),
            },
            StreamEvent::TextDelta {
                text: format!("{KEY_TAIL} then write"),
            },
            StreamEvent::ToolCallStart {
                index: 0,
                id: "c1".into(),
                name: "Write".into(),
            },
            StreamEvent::ToolCallDelta {
                index: 0,
                json: args.to_string(),
            },
            StreamEvent::ToolCallEnd { index: 0 },
            StreamEvent::Usage {
                usage: Usage {
                    input: 100,
                    output: 20,
                    ..Usage::default()
                },
            },
            StreamEvent::Finish {
                reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamEvent::TextDelta {
                text: "done".into(),
            },
            StreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ],
    ]
    .into()
}

async fn session(mode: PermissionMode) -> (tempfile::TempDir, Vec<Value>, String, Vec<AgentEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolCtx::new(dir.path()).unwrap();
    ctx.record = Some(Arc::new(Recorder::create(&ctx.root, "rec-1").unwrap()));
    let mut redactor = Redactor::default();
    redactor.register(SecretString::from(VAULT.to_owned()));
    let mut a = Agent {
        provider: Arc::new(Scripted {
            manifest: Registry::builtin().get("local").unwrap().clone(),
            turns: Mutex::new(turns()),
        }),
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(mode, &[], &[], &[]).unwrap(),
        ctx,
        system: "sys".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 5,
        max_tokens: None,
        reasoning: None,
        redactor,
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
    a.send(
        format!("use {VAULT} please"),
        &tx,
        &Yes,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let mut seen = Vec::new();
    while let Ok(e) = rx.try_recv() {
        seen.push(e);
    }
    let base = dir.path().join(DIR).join("rec-1");
    let events = std::fs::read_to_string(base.join(EVENTS)).unwrap();
    let cast = std::fs::read_to_string(base.join(CAST)).unwrap();
    for text in [&events, &cast] {
        assert!(
            !text.contains(VAULT) && !text.contains(KEY_TAIL),
            "secret recorded:\n{text}"
        );
    }
    let events = events
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // The written file itself is not masked (only the recording is).
    assert!(
        std::fs::read_to_string(dir.path().join("s.txt"))
            .unwrap()
            .contains(VAULT)
    );
    (dir, events, cast, seen)
}

fn kinds(events: &[Value]) -> Vec<&str> {
    events.iter().map(|e| e["kind"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn asked_edit_is_recorded_in_order_with_diff_approval_and_cost() {
    let (_dir, events, cast, seen) = session(PermissionMode::Default).await;
    assert_eq!(
        kinds(&events),
        [
            "prompt",
            "text",
            "usage",
            "diff",
            "ask",
            "approval",
            "tool_start",
            "tool_end",
            "text",
            "end"
        ]
    );
    assert!(events[0]["text"].as_str().unwrap().contains("[REDACTED]"));
    assert_eq!(events[1]["text"], "key [REDACTED] then write");
    assert_eq!(events[2]["input"], 100);
    assert_eq!(events[3]["files"][0]["path"], "s.txt");
    assert_eq!(events[3]["files"][0]["before"], Value::Null);
    assert_eq!(events[3]["files"][0]["after"], "token=[REDACTED]\n");
    assert_eq!(events[5]["reply"], "allow");
    let times: Vec<f64> = events.iter().map(|e| e["t"].as_f64().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");

    // asciinema v2: header object, then [time, "o", text] arrays.
    let mut lines = cast.lines();
    let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(head["version"], 2);
    for l in lines {
        let e: Value = serde_json::from_str(l).unwrap();
        assert!(e[0].is_number() && e[1] == "o" && e[2].is_string(), "{l}");
    }
    assert!(cast.contains("● Write"));

    // The front end got the stream unchanged.
    assert!(
        seen.iter()
            .any(|e| matches!(e, AgentEvent::ToolStart { name, .. } if name == "Write"))
    );
    assert!(seen.iter().any(|e| matches!(e, AgentEvent::Usage(_))));
    let text: String = seen
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains(KEY_TAIL),
        "front end text is not the recording's business"
    );
}

#[tokio::test]
async fn edit_without_asking_still_records_its_diff() {
    let (_dir, events, _cast, _) = session(PermissionMode::AcceptEdits).await;
    let k = kinds(&events);
    assert!(!k.contains(&"ask"), "{k:?}");
    let diff = events.iter().find(|e| e["kind"] == "diff").unwrap();
    assert_eq!(diff["files"][0]["after"], "token=[REDACTED]\n");
    let at = |kind: &str| k.iter().position(|x| *x == kind).unwrap();
    assert!(at("diff") < at("tool_start"));
}
