//! End-to-end agent loop against a scripted model: edits a real file, runs a command,
//! honours permission decisions and cancellation.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{
    Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, Permissions, StopReason,
};
use kor_core::{
    ContentPart, FinishReason, PermissionMode, ProviderError, Role, StreamEvent, Usage,
};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::json;

/// Replays one scripted response per call and records every request.
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
            .expect("unexpected extra model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn tool_call(id: &str, name: &str, args: &serde_json::Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: args.to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Usage {
            usage: Usage {
                input: 10,
                output: 5,
                ..Usage::default()
            },
        },
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

struct Answers(
    Mutex<VecDeque<PermissionReply>>,
    Mutex<Vec<PermissionRequest>>,
);

#[async_trait]
impl Approver for Answers {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.1.lock().unwrap().push(req.clone());
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(PermissionReply::Deny)
    }
}

fn agent(
    root: &std::path::Path,
    mode: PermissionMode,
    script: Vec<Vec<StreamEvent>>,
) -> (Agent, Arc<Scripted>) {
    let provider = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(script.into()),
        seen: Mutex::new(Vec::new()),
    });
    let a = Agent {
        provider: provider.clone(),
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(mode, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(root).unwrap(),
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
    };
    (a, provider)
}

fn results(a: &Agent) -> Vec<(String, bool)> {
    a.messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn edits_a_file_runs_a_command_and_finishes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.txt"), "status = broken\n").unwrap();
    let (mut a, provider) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            tool_call("c1", "Read", &json!({"file_path": "lib.txt"})),
            tool_call(
                "c2",
                "Edit",
                &json!({"file_path": "lib.txt", "old_string": "broken", "new_string": "fixed"}),
            ),
            tool_call("c3", "Bash", &json!({"command": "cat lib.txt"})),
            say("Fixed it."),
        ],
    );
    let approver = Answers(
        Mutex::new([PermissionReply::Allow, PermissionReply::AllowAlways].into()),
        Mutex::new(Vec::new()),
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = a
        .send(
            "fix lib.txt".into(),
            &tx,
            &approver,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(stop, StopReason::Done(FinishReason::Stop));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("lib.txt")).unwrap(),
        "status = fixed\n"
    );

    // Read was free; edit and bash asked.
    let asked: Vec<_> = approver
        .1
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.tool.clone())
        .collect();
    assert_eq!(asked, ["Edit", "Bash"]);

    let r = results(&a);
    assert_eq!(r.len(), 3);
    assert!(r[0].0.contains("status = broken") && !r[0].1);
    assert!(r[2].0.contains("status = fixed") && !r[2].1, "{}", r[2].0);

    // The model saw each tool result on the following call, and the system prompt each time.
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert!(
        seen.iter()
            .all(|r| r.system.as_deref() == Some("sys") && r.tools.len() == default_tools().len())
    );
    assert_eq!(seen[1].messages.last().unwrap().role, Role::User);
    assert_eq!(a.usage.input, 30);

    let mut texts = String::new();
    let mut ends = 0;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::Text(t) => texts.push_str(&t),
            AgentEvent::ToolEnd { .. } => ends += 1,
            _ => {}
        }
    }
    assert_eq!((texts.as_str(), ends), ("Fixed it.", 3));
}

#[tokio::test]
async fn denied_and_plan_mode_calls_return_errors_to_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Plan,
        vec![
            tool_call("c1", "Write", &json!({"path": "x.txt", "content": "x"})),
            tool_call("c2", "nope", &json!({})),
            say("ok"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();

    assert!(!dir.path().join("x.txt").exists());
    assert!(
        approver.1.lock().unwrap().is_empty(),
        "plan mode must not even ask"
    );
    let r = results(&a);
    // §28.2: plan mode tells the model how to leave it instead of a bare "permission denied".
    assert!(r[0].1 && r[0].0.contains("plan mode"), "{}", r[0].0);
    assert!(r[1].1 && r[1].0.contains("unknown tool"));
}

#[tokio::test]
async fn max_turns_and_cancellation_stop_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let looping = || tool_call("c", "list", &json!({}));
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![looping(), looping(), looping()],
    );
    a.max_turns = 2;
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    assert_eq!(
        a.send("go".into(), &tx, &approver, CancellationToken::new())
            .await
            .unwrap(),
        StopReason::MaxTurns
    );

    let (mut a, _) = agent(dir.path(), PermissionMode::Default, vec![looping()]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        a.send("go".into(), &tx, &approver, cancel).await.unwrap(),
        StopReason::Cancelled
    );
    // The pending tool call still got a result, keeping the transcript valid.
    assert_eq!(results(&a).len(), 1);
}

#[tokio::test]
async fn tool_output_is_redacted_before_the_model_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        "OPENAI_API_KEY=sk-proj-AbCdEfGhIjKlMnOpQrStUvWx\n",
    )
    .unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            tool_call("c1", "Read", &json!({"path": ".env"})),
            say("done"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("read".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    assert!(!r[0].0.contains("sk-proj-AbCd"), "{}", r[0].0);
    assert!(r[0].0.contains("[REDACTED]"));
}

#[tokio::test]
async fn hooks_block_and_annotate_and_scoped_rules_attach_once() {
    use kor_agent::{HookRunner, ScopedRule};
    use kor_compat::{Eco, Hook, Scope, Source};

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("src/b.rs"), "fn b() {}\n").unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            tool_call("c1", "Bash", &json!({"command": "echo hi"})),
            tool_call("c2", "Read", &json!({"path": "src/a.rs"})),
            tool_call("c3", "Read", &json!({"path": "src/b.rs"})),
            say("done"),
        ],
    );
    let src = Source {
        eco: Eco::Claude,
        scope: Scope::User,
        path: "settings.json".into(),
    };
    let (block, note) = if cfg!(windows) {
        (
            "[Console]::Error.WriteLine('no shell today'); exit 2",
            "[Console]::Out.WriteLine('{\"hookSpecificOutput\":{\"additionalContext\":\"lint ok\"}}')",
        )
    } else {
        (
            "echo 'no shell today' >&2; exit 2",
            "echo '{\"hookSpecificOutput\":{\"additionalContext\":\"lint ok\"}}'",
        )
    };
    a.hooks = Some(HookRunner {
        hooks: vec![
            Hook {
                event: "PreToolUse".into(),
                matcher: Some("Bash".into()),
                command: block.into(),
                timeout_secs: Some(20),
                source: src.clone(),
                kor: None,
            },
            Hook {
                event: "PostToolUse".into(),
                matcher: Some("Read".into()),
                command: note.into(),
                timeout_secs: Some(20),
                source: src.clone(),
                kor: None,
            },
        ],
        shell: kor_tools::Shell::detect(),
        root: a.ctx.root.clone(),
        session_id: "s".into(),
        transcript: None,
        model: None,
    });
    std::fs::create_dir_all(dir.path().join(".claude/rules")).unwrap();
    std::fs::write(
        dir.path().join(".claude/rules/rust.md"),
        "---\npaths: [\"src/**/*.rs\"]\n---\nUse thiserror.",
    )
    .unwrap();
    let compat = kor_compat::load(&kor_compat::LoadOptions {
        root: a.ctx.root.clone(),
        home: None,
        codex_home: None,
        trusted: false,
    });
    a.rules = ScopedRule::from_compat(&a.ctx.root, &compat).0;
    assert_eq!(a.rules.len(), 1);

    // Approve everything so only the hook can stop the bash call.
    let approver = Answers(
        Mutex::new([PermissionReply::Allow; 3].into()),
        Mutex::new(Vec::new()),
    );
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();

    let r = results(&a);
    assert!(r[0].1 && r[0].0.contains("no shell today"), "{}", r[0].0);
    assert!(
        approver.1.lock().unwrap().is_empty(),
        "a hook block must not prompt"
    );
    assert!(r[1].0.contains("[PostToolUse hook] lint ok"), "{}", r[1].0);
    assert!(r[1].0.contains("Use thiserror."), "{}", r[1].0);
    assert!(!r[2].0.contains("Use thiserror."), "rule attaches once");
}

#[tokio::test]
async fn compact_boundary_when_the_estimate_passes_the_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, scripted) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![say("summary of the work"), say("continued")],
    );
    a.auto_compact = true;
    a.context_window = kor_agent::compact::AUTOCOMPACT_BUFFER_TOKENS + 8;
    a.messages.push(kor_core::Message::text(
        kor_core::Role::User,
        "x".repeat(80),
    ));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("next".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let text = a
        .messages
        .iter()
        .map(kor_core::Message::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains(kor_agent::compact::BOUNDARY), "{text}");
    assert!(text.contains("continued"), "{text}");
    let seen = scripted.seen.lock().unwrap();
    assert!(seen[0].tools.is_empty(), "compact call has no tools");
}

#[tokio::test]
async fn context_budget_reports_and_summarizes_with_the_cheap_model() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, scripted) = agent(dir.path(), PermissionMode::Default, vec![say("continued")]);
    let cheap = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(vec![say("cheap summary")].into()),
        seen: Mutex::new(Vec::new()),
    });
    a.auto_compact = true;
    a.context_window = 100_000;
    // History passes 50% of the window, far below the built-in buffer threshold.
    a.context.auto_percent = 50;
    a.context.warn_percent = 1;
    a.context.summarizer = Some((cheap.clone(), "small-model".into()));
    a.messages.push(kor_core::Message::text(
        kor_core::Role::User,
        "x".repeat(240_000),
    ));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("next".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(cheap.seen.lock().unwrap()[0].model, "small-model");
    assert_eq!(scripted.seen.lock().unwrap().len(), 1, "main model: one turn");
    let mut report = None;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::Context(b) = ev {
            report = Some(b);
        }
    }
    let b = report.expect("a Context event per request");
    assert!(b.estimated && b.window == 100_000 && b.history > 0 && b.history < 1_000);
    assert_eq!(b.warning.as_deref(), b.dominant(1).map(|(k, _)| k));
    assert_eq!(a.context.last.as_ref(), Some(&b));
}

#[tokio::test]
async fn agent_tool_returns_the_subagent_text() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(
            vec![
                tool_call(
                    "a1",
                    "Agent",
                    &json!({"prompt": "find the main", "subagent_type": "Explore", "description": "search"}),
                ),
                say("found main.rs"),
                say("the entry is main.rs"),
            ]
            .into(),
        ),
        seen: Mutex::new(Vec::new()),
    });
    let base = default_tools();
    let mut tools = base.clone();
    tools.push(Arc::new(kor_agent::AgentTool {
        provider: provider.clone(),
        model: "m".into(),
        tools: base,
        permissions: Permissions::new(PermissionMode::Plan, &[], &[], &[]).unwrap(),
        agents: Vec::new(),
        explore_plan: true,
    }));
    let mut a = Agent {
        provider,
        model: "m".into(),
        tools,
        permissions: Permissions::new(PermissionMode::Plan, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(dir.path()).unwrap(),
        system: "sys".into(),
        messages: Vec::new(),
        usage: kor_core::Usage::default(),
        max_turns: 6,
        max_tokens: None,
        reasoning: None,
        redactor: kor_secrets::Redactor::default(),
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
    };
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("look".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let text = a
        .messages
        .iter()
        .flat_map(|m| m.parts.clone())
        .filter_map(|p| match p {
            kor_core::ContentPart::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("found main.rs"), "{text}");
}

#[tokio::test]
async fn session_memory_compact_sends_its_own_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("summary.md");
    std::fs::write(&notes, "Current state: porting compact.").unwrap();
    let (mut a, scripted) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![say("kept going"), say("after")],
    );
    a.auto_compact = true;
    a.context_window = kor_agent::compact::AUTOCOMPACT_BUFFER_TOKENS + 8;
    a.session_notes = Some(notes);
    a.messages.push(kor_core::Message::text(
        kor_core::Role::User,
        "y".repeat(80),
    ));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let system = scripted.seen.lock().unwrap()[0]
        .system
        .clone()
        .unwrap_or_default();
    assert!(
        system.contains("Recent messages are preserved verbatim."),
        "{system}"
    );
    assert!(
        a.messages.iter().any(|m| m
            .plain_text()
            .contains(kor_agent::compact::SESSION_BOUNDARY)),
        "boundary missing"
    );
}

#[tokio::test]
async fn token_budget_stops_before_another_tool_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
    let mut first = tool_call("t1", "Read", &json!({"file_path": "a.txt"}));
    for ev in &mut first {
        if let StreamEvent::Usage { usage } = ev {
            usage.output = 100;
        }
    }
    let (mut a, scripted) = agent(
        dir.path(),
        PermissionMode::AcceptEdits,
        vec![first, say("should not be requested")],
    );
    a.output_budget = Some(10);
    let approver = Answers(
        Mutex::new([PermissionReply::Allow].into()),
        Mutex::new(Vec::new()),
    );
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = a
        .send("read".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(stop, StopReason::Budget);
    assert_eq!(scripted.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn dream_gate_calls_the_provider_only_when_open() {
    let dir = tempfile::tempdir().unwrap();
    let (mut closed, scripted) = agent(dir.path(), PermissionMode::Default, vec![say("done")]);
    closed.dream = false;
    closed.memory_base = Some(dir.path().to_path_buf());
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    closed
        .send("hi".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(scripted.seen.lock().unwrap().len(), 1);

    let home = dir.path().join("home");
    let (mut open, scripted) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            say("done"),
            say("FILE: dream.md\nTITLE: Dream\nBODY: consolidated"),
        ],
    );
    open.dream = true;
    open.memory_base = Some(home.clone());
    open.send("hi".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(scripted.seen.lock().unwrap().len(), 2);
    let system = scripted.seen.lock().unwrap()[1]
        .system
        .clone()
        .unwrap_or_default();
    assert!(system.contains("performing a dream"), "{system}");
    let lay = kor_agent::memory::layout(&home, &open.ctx.root);
    let index = kor_agent::memory::read_entrypoint(&lay).unwrap();
    assert!(index.contains("dream.md"), "{index}");
}

fn two_calls(
    name: &str,
    first: &serde_json::Value,
    second: &serde_json::Value,
) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: "a".into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: first.to_string(),
        },
        StreamEvent::ToolCallStart {
            index: 1,
            id: "b".into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 1,
            json: second.to_string(),
        },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

#[tokio::test]
async fn progress_is_sent_before_the_tool_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            tool_call("c1", "Read", &json!({"file_path": "a.txt"})),
            say("done"),
        ],
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let work = async move {
        a.send("go".into(), &tx, &approver, CancellationToken::new())
            .await
    };
    let mut order = Vec::new();
    let recv = async {
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::ToolProgress { .. } => order.push("progress"),
                AgentEvent::ToolEnd { .. } => order.push("end"),
                _ => {}
            }
        }
    };
    let (result, ()) = tokio::join!(work, recv);
    result.unwrap();
    let p = order.iter().position(|e| *e == "progress").unwrap();
    let e = order.iter().position(|e| *e == "end").unwrap();
    assert!(p < e, "{order:?}");
}

#[tokio::test]
async fn two_reads_finish_together() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            two_calls("Sleep", &json!({"seconds": 0.4}), &json!({"seconds": 0.4})),
            say("done"),
        ],
    );
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let started = std::time::Instant::now();
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(700),
        "reads ran serially: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn compact_warning_fires_inside_the_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(dir.path(), PermissionMode::Default, vec![say("done")]);
    a.context_window = 13_000 + 20_000;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let work = async move {
        a.send("go".into(), &tx, &approver, CancellationToken::new())
            .await
    };
    let mut notes = Vec::new();
    let recv = async {
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Notice(n) = ev {
                notes.push(n);
            }
        }
    };
    let (result, ()) = tokio::join!(work, recv);
    result.unwrap();
    assert!(notes.iter().any(|n| n.contains("20000")), "{notes:?}");
}

/// Text a model writes when it uses the text tool protocol.
fn text_call(name: &str, args: &serde_json::Value) -> Vec<StreamEvent> {
    say(&format!(
        "I will check.\n<tool_call>\n{}\n</tool_call>",
        json!({"name": name, "arguments": args})
    ))
}

#[tokio::test]
async fn text_protocol_model_calls_tools_without_native_support() {
    use kor_agent::toolcall::ToolMode;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.txt"), "status = broken\n").unwrap();
    let (mut a, provider) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            // A foreign tool name with foreign argument names, written as text.
            text_call("read_file", &json!({"target_file": "lib.txt"})),
            say("It says broken."),
        ],
    );
    a.tool_mode = ToolMode::Text;
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = a
        .send("what does lib.txt say?".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(stop, StopReason::Done(FinishReason::Stop));

    let seen = provider.seen.lock().unwrap();
    assert!(seen.iter().all(|r| r.tools.is_empty()), "text mode sends no native tools");
    assert!(seen[0].system.as_deref().unwrap().contains("<tool_call>"));
    let result = seen[1].messages.last().unwrap().plain_text();
    assert!(result.contains("<tool_result name=\"Read\">"), "{result}");
    assert!(result.contains("status = broken"), "{result}");
}

#[tokio::test]
async fn native_call_with_foreign_name_and_arguments_runs_the_kor_tool() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            tool_call("c1", "view_file", &json!({"filename": "a.txt", "explanation": "look"})),
            say("ok"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new()).await.unwrap();
    let r = results(&a);
    assert!(!r[0].1 && r[0].0.contains("hello"), "{}", r[0].0);
}

/// Rejects any request that carries native tools, like an endpoint without tool support.
struct NoTools {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
    seen: Mutex<Vec<ChatRequest>>,
}

#[async_trait]
impl Provider for NoTools {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(&self, req: ChatRequest, _c: CancellationToken) -> Result<EventStream, ProviderError> {
        let has_tools = !req.tools.is_empty();
        self.seen.lock().unwrap().push(req);
        if has_tools {
            return Err(ProviderError::new(
                "mock",
                kor_core::ProviderErrorKind::InvalidRequest,
                "this model does not support tools",
            ));
        }
        let evs = self.turns.lock().unwrap().pop_front().expect("unexpected call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

#[tokio::test]
async fn provider_rejecting_native_tools_switches_to_text() {
    use kor_agent::toolcall::ToolMode;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let (mut a, _) = agent(dir.path(), PermissionMode::Default, Vec::new());
    let p = Arc::new(NoTools {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(
            vec![text_call("Read", &json!({"file_path": "a.txt"})), say("done")].into(),
        ),
        seen: Mutex::new(Vec::new()),
    });
    a.provider = p.clone();
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = a.send("go".into(), &tx, &approver, CancellationToken::new()).await.unwrap();
    assert_eq!(stop, StopReason::Done(FinishReason::Stop));
    assert_eq!(a.tool_mode, ToolMode::Text);
    assert_eq!(p.seen.lock().unwrap().len(), 3, "rejected native call, then two text calls");
    let mut noticed = false;
    while let Ok(ev) = rx.try_recv() {
        noticed |= matches!(ev, AgentEvent::Notice(n) if n.contains("text tool protocol"));
    }
    assert!(noticed);
}

/// A read tool that returns an image, like `screenshot`.
#[derive(Debug)]
struct Snap;

#[async_trait]
impl kor_tools::Tool for Snap {
    fn name(&self) -> &'static str {
        "snap"
    }
    fn rule_name(&self) -> &'static str {
        "snap"
    }
    fn kind(&self) -> kor_tools::ToolKind {
        kor_tools::ToolKind::Read
    }
    fn spec(&self) -> kor_core::ToolSpec {
        kor_core::ToolSpec {
            name: "snap".into(),
            description: "image".into(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }
    fn target(&self, _input: &serde_json::Value) -> Option<String> {
        None
    }
    async fn call(&self, _input: serde_json::Value, _ctx: &ToolCtx) -> kor_tools::ToolOutput {
        let mut out = kor_tools::ToolOutput::ok("captured");
        out.image = Some(kor_core::ImageSource::Base64 { media_type: "image/png".into(), data: "AAAA".into() });
        out
    }
}

#[tokio::test]
async fn tool_images_reach_the_model_after_the_results() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, provider) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![tool_call("c1", "snap", &json!({})), say("I see it")],
    );
    a.tools.push(Arc::new(Snap));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("look".into(), &tx, &approver, CancellationToken::new()).await.unwrap();
    let seen = provider.seen.lock().unwrap();
    let last = seen[1].messages.last().unwrap();
    assert!(matches!(last.parts[0], ContentPart::ToolResult { .. }));
    assert!(matches!(&last.parts[1], ContentPart::Image { source: kor_core::ImageSource::Base64 { data, .. } } if data == "AAAA"));
}

#[tokio::test]
async fn models_without_vision_get_ocr_text_instead_of_images() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(dir.path(), PermissionMode::Default, Vec::new());
    let mut manifest = Registry::builtin().get("local").unwrap().clone();
    manifest.vision = false;
    let provider = Arc::new(Scripted {
        manifest,
        turns: Mutex::new(vec![tool_call("c1", "snap", &json!({})), say("ok")].into()),
        seen: Mutex::new(Vec::new()),
    });
    a.provider = provider.clone();
    a.tools.push(Arc::new(Snap));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("look".into(), &tx, &approver, CancellationToken::new()).await.unwrap();
    let seen = provider.seen.lock().unwrap();
    let last = seen[1].messages.last().unwrap();
    assert!(!last.parts.iter().any(|p| matches!(p, ContentPart::Image { .. })));
    // "AAAA" is not a decodable image, so the OCR engine reports a failure in the same place.
    assert!(matches!(&last.parts[0], ContentPart::ToolResult { content, .. }
        if content.starts_with("captured") && content.contains("cannot see images; OCR text")));
}

#[tokio::test]
async fn undo_reverts_every_file_change_of_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.txt"), "status = broken\n").unwrap();
    let (mut a, _) = agent(
        &root,
        PermissionMode::BypassPermissions,
        vec![
            tool_call("c1", "Read", &json!({"file_path": "lib.txt"})),
            tool_call(
                "c2",
                "Edit",
                &json!({"file_path": "lib.txt", "old_string": "broken", "new_string": "fixed"}),
            ),
            tool_call("c3", "Write", &json!({"file_path": "src/new.txt", "content": "yeni"})),
            say("Done."),
        ],
    );
    let base = dir.path().join("undo");
    let journal =
        kor_tools::undo::Journal::open(&base, "s1", std::slice::from_ref(&a.ctx.root)).unwrap();
    *a.ctx.undo.lock().unwrap() = Some(journal);
    let approver = Answers(Mutex::new(VecDeque::default()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("fix".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("lib.txt")).unwrap(),
        "status = fixed\n"
    );
    assert!(root.join("src/new.txt").is_file());

    let r = kor_tools::undo::undo(&base.join("s1")).unwrap();
    assert_eq!((r.restored.len(), r.deleted.len()), (1, 1), "{r:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("lib.txt")).unwrap(),
        "status = broken\n"
    );
    assert!(!root.join("src").exists());
}

// --- Verified edit loop (8.1), tool-call repair (15.12), schema gate (15.13) ---

/// A one-file Rust crate whose `f` returns `body`.
fn crate_with(root: &std::path::Path, body: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"v\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), format!("pub fn f() -> u32 {{ {body} }}\n")).unwrap();
}

/// Edit needs a fresh full Read before every change.
fn reread() -> Vec<StreamEvent> {
    tool_call("rr", "Read", &json!({"file_path": "src/lib.rs"}))
}

fn edit(id: &str, from: &str, to: &str) -> Vec<StreamEvent> {
    tool_call(
        id,
        "Edit",
        &json!({"file_path": "src/lib.rs", "old_string": from, "new_string": to}),
    )
}

#[tokio::test]
async fn broken_edit_gets_the_checker_error_and_is_fixed_in_round_two() {
    let dir = tempfile::tempdir().unwrap();
    crate_with(dir.path(), "1");
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::BypassPermissions,
        vec![
            tool_call("r", "Read", &json!({"file_path": "src/lib.rs"})),
            edit("e1", "{ 1 }", "{ \"one\" }"),
            reread(),
            edit("e2", "{ \"one\" }", "{ 2 }"),
            say("fixed"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("change f".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    assert!(r[1].0.contains("round 1/3") && r[1].0.contains("src/lib.rs"), "{}", r[1].0);
    assert!(r[1].0.contains("error"), "the checker's error reaches the model: {}", r[1].0);
    assert!(r[3].0.contains("no errors in src/lib.rs"), "{}", r[3].0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "pub fn f() -> u32 { 2 }\n"
    );
}

#[tokio::test]
async fn verification_stops_after_three_failed_rounds() {
    let dir = tempfile::tempdir().unwrap();
    crate_with(dir.path(), "1");
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::BypassPermissions,
        vec![
            tool_call("r", "Read", &json!({"file_path": "src/lib.rs"})),
            edit("e1", "{ 1 }", "{ \"a\" }"),
            reread(),
            edit("e2", "\"a\"", "\"b\""),
            reread(),
            edit("e3", "\"b\"", "\"c\""),
            reread(),
            edit("e4", "\"c\"", "\"d\""),
            say("still broken"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("change f".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    for (i, round) in [(1, "1/3"), (3, "2/3"), (5, "3/3")] {
        assert!(r[i].0.contains(&format!("round {round}")), "{}", r[i].0);
    }
    assert!(r[5].0.contains("last automatic check"), "{}", r[5].0);
    assert!(r[7].0.contains("no longer checked"), "{}", r[7].0);
    let mut noticed = false;
    while let Ok(ev) = rx.try_recv() {
        noticed |= matches!(ev, AgentEvent::Notice(n) if n.contains("verification stopped"));
    }
    assert!(noticed, "the user is told");

    // `agent.verify = "off"`: no checker at all.
    let (mut off, _) = agent(
        dir.path(),
        PermissionMode::BypassPermissions,
        vec![
            tool_call("r", "Read", &json!({"file_path": "src/lib.rs"})),
            edit("e5", "\"d\"", "\"e\""),
            say("ok"),
        ],
    );
    off.verify.enabled = false;
    off.send("again".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    assert!(!results(&off)[1].0.contains("[verify]"));
}

#[tokio::test]
async fn project_checkers_need_bash_permission() {
    let dir = tempfile::tempdir().unwrap();
    crate_with(dir.path(), "1");
    // acceptEdits allows the edit but not the command; the approver denies it.
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::AcceptEdits,
        vec![
            tool_call("r", "Read", &json!({"file_path": "src/lib.rs"})),
            edit("e1", "{ 1 }", "{ \"x\" }"),
            say("ok"),
        ],
    );
    let approver = Answers(Mutex::new([PermissionReply::Deny].into()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let asked = approver.1.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].tool == "Bash" && asked[0].summary.contains("cargo check"));
    assert!(results(&a)[1].0.contains("needs Bash permission"));
    assert!(!dir.path().join("target").exists(), "cargo never ran");
}

#[tokio::test]
async fn write_over_a_long_existing_file_is_refused_with_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big.txt"), "line\n".repeat(10)).unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::BypassPermissions,
        vec![
            tool_call("r", "Read", &json!({"file_path": "big.txt"})),
            tool_call("w", "Write", &json!({"file_path": "big.txt", "content": "all new\n"})),
            tool_call("n", "Write", &json!({"file_path": "new.txt", "content": "fresh\n"})),
            say("ok"),
        ],
    );
    a.verify.write_max_lines = 5;
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    assert!(r[1].1 && r[1].0.contains("has 10 lines") && r[1].0.contains("with Edit"), "{}", r[1].0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("big.txt")).unwrap(),
        "line\n".repeat(10)
    );
    assert!(!r[2].1, "new files are written: {}", r[2].0);
}

#[tokio::test]
async fn malformed_text_tool_call_is_repaired_locally() {
    use kor_agent::toolcall::ToolMode;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.txt"), "status = broken\n").unwrap();
    let (mut a, provider) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            // Single quotes, a foreign name, a string for a number and no closing braces.
            say("<tool_call>\n{'name': 'read_file', 'arguments': {'target_file': 'lib.txt', 'offset': '1'\n</tool_call>"),
            say("It says broken."),
        ],
    );
    a.tool_mode = ToolMode::Text;
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("read".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let result = provider.seen.lock().unwrap()[1].messages.last().unwrap().plain_text();
    assert!(result.contains("status = broken"), "{result}");
    assert_eq!(
        a.stats,
        kor_agent::repair::ToolStats { tool_calls_ok: 0, tool_calls_repaired: 1, tool_calls_failed: 0 }
    );
}

#[tokio::test]
async fn unparseable_call_gets_one_model_repair_then_fails_after_three_attempts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let raw = |id: &str, text: &str| {
        vec![
            StreamEvent::ToolCallStart { index: 0, id: id.into(), name: "Read".into() },
            StreamEvent::ToolCallDelta { index: 0, json: text.into() },
            StreamEvent::Finish { reason: FinishReason::ToolUse },
        ]
    };
    let (mut a, provider) = agent(
        dir.path(),
        PermissionMode::Default,
        vec![
            raw("c1", "file_path=a.txt"),
            // The repair round-trip answers with a valid call.
            say("{\"name\": \"Read\", \"arguments\": {\"file_path\": \"a.txt\"}}"),
            raw("c2", "<<garbage>>"),
            say("I cannot"),
            say("done"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("read".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    assert!(!r[0].1 && r[0].0.contains("hello"), "{}", r[0].0);
    assert!(r[1].1 && r[1].0.contains("not valid JSON"), "{}", r[1].0);
    {
        let seen = provider.seen.lock().unwrap();
        assert!(seen[1].tools.is_empty() && seen[1].system.as_deref().unwrap().contains("repair"));
        assert_eq!(seen[1].messages.len(), 1, "only the schema and the broken output");
    }
    let mut failed_notice = false;
    while let Ok(ev) = rx.try_recv() {
        failed_notice |= matches!(ev, AgentEvent::Notice(n) if n.contains("could not be repaired"));
    }
    assert!(failed_notice);
    assert_eq!((a.stats.tool_calls_repaired, a.stats.tool_calls_failed), (1, 1));
}

#[tokio::test]
async fn schema_invalid_input_is_rejected_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(
        dir.path(),
        PermissionMode::BypassPermissions,
        vec![
            tool_call("w", "Write", &json!({"file_path": "x.txt", "content": {"not": "text"}})),
            tool_call(
                "m",
                "MultiEdit",
                &json!({"file_path": "x.txt", "edits": [{"new_string": "y"}]}),
            ),
            tool_call("ok", "Write", &json!({"file_path": "y.txt", "content": 42})),
            say("ok"),
        ],
    );
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, &approver, CancellationToken::new())
        .await
        .unwrap();
    let r = results(&a);
    assert!(r[0].1 && r[0].0.contains("invalid input for `Write`: `content` must be string"), "{}", r[0].0);
    assert!(r[1].1 && r[1].0.contains("`edits[0].old_string`"), "{}", r[1].0);
    assert!(!dir.path().join("x.txt").exists(), "nothing reached the file system");
    // A number where a string belongs is coerced, not rejected.
    assert_eq!(std::fs::read_to_string(dir.path().join("y.txt")).unwrap(), "42");
    assert_eq!(a.stats.tool_calls_failed, 2);
    assert_eq!(a.stats.tool_calls_repaired, 1);
}

/// A tool that never finishes and never looks at the cancel token (a hung command).
struct Hang;

#[async_trait]
impl kor_tools::Tool for Hang {
    fn name(&self) -> &'static str {
        "Hang"
    }
    fn rule_name(&self) -> &'static str {
        "Hang"
    }
    fn kind(&self) -> kor_tools::ToolKind {
        kor_tools::ToolKind::Read
    }
    fn spec(&self) -> kor_core::ToolSpec {
        kor_core::ToolSpec { name: "Hang".into(), description: "hangs".into(), input_schema: json!({"type": "object"}) }
    }
    fn target(&self, _input: &serde_json::Value) -> Option<String> {
        None
    }
    async fn call(&self, _input: serde_json::Value, _ctx: &ToolCtx) -> kor_tools::ToolOutput {
        std::future::pending().await
    }
}

#[tokio::test]
async fn stop_interrupts_a_tool_that_never_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _) = agent(dir.path(), PermissionMode::BypassPermissions, vec![tool_call("h", "Hang", &json!({}))]);
    a.tools.push(Arc::new(Hang));
    let approver = Answers(Mutex::new(VecDeque::new()), Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        stop.cancel();
    });
    let r = tokio::time::timeout(std::time::Duration::from_secs(10), a.send("go".into(), &tx, &approver, cancel)).await;
    assert_eq!(r.expect("Stop must not wait for the tool").unwrap(), StopReason::Cancelled);
    assert!(results(&a)[0].0.contains("cancelled"));
    // The tool card is closed for the front end.
    drop(tx);
    let mut ended = false;
    while let Some(e) = rx.recv().await {
        ended |= matches!(e, AgentEvent::ToolEnd { ref id, is_error: true, .. } if id == "h");
    }
    assert!(ended);
}

