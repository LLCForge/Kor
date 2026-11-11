//! Smart cost router and budget through the real agent loop, with scripted models: a docs task
//! goes to its configured cheap model, the reviewer checks the draft, the provider-reported cost
//! reaches the ledger, decisions are logged with outcomes, and limits pause or ask.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{
    Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, Permissions, StopReason,
};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::cost::{Control, Files, Prices, Target};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};

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
            .expect("unexpected model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn scripted(turns: Vec<Vec<StreamEvent>>) -> Arc<Scripted> {
    Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(turns.into()),
        seen: Mutex::new(Vec::new()),
    })
}

fn say(text: &str, cost: Option<f64>) -> Vec<StreamEvent> {
    let mut v = vec![StreamEvent::TextDelta { text: text.into() }];
    if let Some(c) = cost {
        v.push(StreamEvent::Raw {
            value: serde_json::json!({"prompt_tokens": 100, "cost": c}),
        });
    }
    v.push(StreamEvent::Usage {
        usage: Usage {
            input: 100,
            output: 10,
            ..Usage::default()
        },
    });
    v.push(StreamEvent::Finish {
        reason: FinishReason::Stop,
    });
    v
}

fn target(p: &Arc<Scripted>, model: &str) -> Target {
    Target {
        provider: p.clone(),
        model: model.into(),
        tool_mode: kor_agent::toolcall::ToolMode::Native,
    }
}

fn agent(root: &Path, p: &Arc<Scripted>, cost: Arc<Control>) -> Agent {
    Agent {
        provider: p.clone(),
        model: "default".into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(root).unwrap(),
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
        cost: Some(cost),
        shadow: None,
    }
}

struct Reply(PermissionReply, Mutex<Vec<PermissionRequest>>);

#[async_trait]
impl Approver for Reply {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.1.lock().unwrap().push(req.clone());
        self.0
    }
}

fn files(dir: &Path) -> Files {
    Files {
        ledger: Some(dir.join("usage.jsonl")),
        log: Some(dir.join("routing.jsonl")),
    }
}

#[tokio::test]
async fn docs_task_is_drafted_cheaply_then_verified_and_costed() {
    let dir = tempfile::tempdir().unwrap();
    let default = scripted(Vec::new());
    let cheap = scripted(vec![say("drafted", Some(0.01))]);
    let big = scripted(vec![say("verified", None)]);
    let routing = kor_config::Routing {
        enabled: true,
        docs: "local/cheap".into(),
        verify_with: "local/big".into(),
        ..kor_config::Routing::default()
    };
    let control = Arc::new(Control::new(
        kor_i18n::Lang::En,
        routing,
        kor_config::Budget::default(),
        target(&default, "default"),
        vec![target(&cheap, "cheap"), target(&big, "big")],
        Prices::default(),
        files(dir.path()),
    ));
    let mut a = agent(dir.path(), &default, control.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = Reply(PermissionReply::Deny, Mutex::new(Vec::new()));
    let stop = a
        .send(
            "update the README".into(),
            &tx,
            &approver,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(stop, StopReason::Done(_)));
    assert!(default.seen.lock().unwrap().is_empty());
    assert_eq!(cheap.seen.lock().unwrap().len(), 1);
    {
        let reviewed = big.seen.lock().unwrap();
        assert_eq!(reviewed.len(), 1);
        assert!(
            reviewed[0]
                .messages
                .iter()
                .any(|m| m.plain_text().contains("Draft-then-verify"))
        );
    }
    drop(tx);
    let mut notices = Vec::new();
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Notice(n) = ev {
            notices.push(n);
        }
    }
    assert!(
        notices
            .iter()
            .any(|n| n.contains("local/cheap") && n.contains("docs")),
        "{notices:?}"
    );

    let ledger = kor_runtime::usage::read(&dir.path().join("usage.jsonl"));
    assert_eq!(ledger.len(), 2);
    assert_eq!(ledger[0].model, "local/cheap");
    assert_eq!(ledger[0].cost_usd, Some(0.01));
    let log = kor_runtime::cost::decisions(&dir.path().join("routing.jsonl"));
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].model, "local/cheap");
    assert_eq!(log[0].outcome, Some(true));
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let summary = control.summary_lines(&l).join("\n");
    assert!(
        summary.contains("local/cheap") && summary.contains("$0.01"),
        "{summary}"
    );
}

#[tokio::test]
async fn routing_off_keeps_the_default_model() {
    let dir = tempfile::tempdir().unwrap();
    let default = scripted(vec![say("hi", None)]);
    let control = Arc::new(Control::new(
        kor_i18n::Lang::En,
        kor_config::Routing {
            docs: "local/cheap".into(),
            ..kor_config::Routing::default()
        },
        kor_config::Budget::default(),
        target(&default, "default"),
        Vec::new(),
        Prices::default(),
        files(dir.path()),
    ));
    let mut a = agent(dir.path(), &default, control);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = Reply(PermissionReply::Deny, Mutex::new(Vec::new()));
    a.send(
        "update the README".into(),
        &tx,
        &approver,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(default.seen.lock().unwrap().len(), 1);
    assert!(kor_runtime::cost::decisions(&dir.path().join("routing.jsonl")).is_empty());
}

async fn over_limit(
    on_limit: kor_config::OnLimit,
    reply: PermissionReply,
) -> (StopReason, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    kor_runtime::usage::record_call(&dir.path().join("usage.jsonl"), "x/y", 1, 1, Some(2.0))
        .unwrap();
    let default = scripted(vec![say("hi", None)]);
    let control = Arc::new(Control::new(
        kor_i18n::Lang::En,
        kor_config::Routing::default(),
        kor_config::Budget {
            daily_limit_usd: 1.0,
            on_limit,
            ..kor_config::Budget::default()
        },
        target(&default, "default"),
        Vec::new(),
        Prices::default(),
        files(dir.path()),
    ));
    let mut a = agent(dir.path(), &default, control);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = Reply(reply, Mutex::new(Vec::new()));
    let stop = a
        .send("hello".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let asked = approver.1.lock().unwrap().len();
    let calls = default.seen.lock().unwrap().len();
    (stop, asked, calls)
}

#[tokio::test]
async fn budget_limit_pauses_or_asks() {
    let (stop, asked, calls) = over_limit(kor_config::OnLimit::Pause, PermissionReply::Allow).await;
    assert_eq!((stop, asked, calls), (StopReason::Budget, 0, 0));
    let (stop, asked, calls) = over_limit(kor_config::OnLimit::Ask, PermissionReply::Deny).await;
    assert_eq!((stop, asked, calls), (StopReason::Budget, 1, 0));
    let (stop, asked, calls) = over_limit(kor_config::OnLimit::Ask, PermissionReply::Allow).await;
    assert!(matches!(stop, StopReason::Done(_)));
    assert_eq!((asked, calls), (1, 1));
}
