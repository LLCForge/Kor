#![allow(clippy::format_collect, clippy::too_many_lines)] // one linear acceptance flow
//! Faz 5 acceptance (spec §14 item 6, ADR 0035): a full task flow with the network off.
//!
//! The parent test runs this binary again as a child with `KOR_OFFLINE=1` and every proxy
//! variable pointing at a loopback "trap" listener that counts connections; nothing on the
//! machine's real network changes. In the child the agent works against a mock
//! OpenAI-compatible server on 127.0.0.1: it reads a file, edits it, runs a command, asks the
//! project graph, saves a memory, tries a web fetch, and finishes; a second task recalls what the
//! first one learned. The model catalog, update check and team sync are pointed at internet
//! hosts on purpose. Pass = the task completes, every leak fails fast with the offline message,
//! the network guard let only loopback hosts through, and the trap saw zero connections.

use std::collections::VecDeque;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest, Permissions, StopReason};
use kor_core::net::{self, Policy};
use kor_core::{FinishReason, PermissionMode, Usage};
use kor_i18n::{Lang, Localizer};
use kor_providers::model_catalog::{self, Endpoints, Options};
use kor_providers::{CancellationToken, Registry};
use kor_runtime::cost::{Control, Files, Prices, Target};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CHILD: &str = "KOR_OFFLINE_E2E_CHILD";

#[test]
fn full_task_flow_with_the_network_off() {
    if std::env::var_os(CHILD).is_some() {
        return;
    }
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", trap.local_addr().unwrap());
    let home = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["child_flow", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("KOR_OFFLINE", "1")
        .env("KOR_HOME", home.path())
        .env("HTTP_PROXY", &proxy)
        .env("HTTPS_PROXY", &proxy)
        .env("ALL_PROXY", &proxy)
        .env_remove("NO_PROXY")
        .output()
        .unwrap();
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    assert!(log.contains("1 passed"), "child did not run: {log}");
    // Queued connections are accepted now; any leak through the proxy would be one.
    let mut leaks = 0;
    while trap.accept().is_ok() {
        leaks += 1;
    }
    assert_eq!(leaks, 0, "connections reached the proxy trap");
}

/// One SSE response per model call, in order.
struct Script(Mutex<VecDeque<String>>);

impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let body = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model call");
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    }
}

fn sse(frames: &[Value]) -> String {
    let mut s: String = frames.iter().map(|f| format!("data: {f}\n\n")).collect();
    s.push_str("data: [DONE]\n\n");
    s
}

fn usage() -> Value {
    json!({"choices": [], "usage": {"prompt_tokens": 120, "completion_tokens": 12}})
}

fn call(name: &str, args: &Value) -> String {
    sse(&[
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": format!("c-{name}"), "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        usage(),
    ])
}

fn say(text: &str) -> String {
    sse(&[
        json!({"choices": [{"index": 0, "delta": {"content": text}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        usage(),
    ])
}

struct AllowAll;

#[async_trait]
impl Approver for AllowAll {
    async fn approve(&self, _: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
}

async fn leaks_fail_fast(l: &Localizer) {
    let started = Instant::now();
    let offline = |e: &str| assert!(e.contains("Offline mode"), "{e}");
    let ep = Endpoints {
        catalog: "https://catalog.kor-leak.invalid/catalog.json".into(),
        openrouter: "https://openrouter.ai/api/v1/models".into(),
    };
    let opts = Options {
        offline: false,
        refresh: true,
    };
    offline(
        &model_catalog::load(None, &ep, opts)
            .await
            .unwrap_err()
            .to_string(),
    );
    offline(&kor_runtime::update::releases().await.unwrap_err());
    offline(
        &kor_agent::team_sync::push("https://team.kor-leak.invalid", "r", "t", &json!({}))
            .await
            .unwrap_err(),
    );
    // A cloud provider is refused before any connection.
    let cloud = Registry::builtin().get("openrouter").unwrap().clone();
    assert!(kor_runtime::check_provider_allowed(l, &cloud).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

fn agent(
    root: &Path,
    home: &Path,
    provider: Arc<dyn kor_providers::Provider>,
    cost: Arc<Control>,
) -> Agent {
    let mut tools = default_tools();
    tools.extend(kor_agent::knowledge::tools(
        Some(kor_agent::memory::layout(home, root)),
        None,
        None,
    ));
    let mut system = "You are a coding agent.".to_owned();
    kor_agent::graph::attach(
        &mut tools,
        &mut system,
        root,
        kor_agent::graph::Options {
            strict: false,
            watch: false,
            home: Some(home.to_owned()),
            skip_hotspots: false,
        },
    );
    let mut ctx = ToolCtx::new(root).unwrap();
    // As `prepare` does in offline mode (the sandbox follows the config; default: off/ask).
    ctx.network = false;
    ctx.env = kor_runtime::OFFLINE_COMMAND_ENV
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Agent {
        provider,
        model: "qwen-local".into(),
        tools,
        permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
        ctx,
        system,
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 12,
        max_tokens: Some(512),
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
        context: kor_agent::context::Budget {
            log: Some(home.join("context.jsonl")),
            recall: kor_agent::recall::Recall::open(Some(home), root, true, 2_000, true),
            ..kor_agent::context::Budget::default()
        },
        cost: Some(cost),
        shadow: None,
        repair: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_flow() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let l = Localizer::new(Lang::En);
    let loaded = kor_config::load(&kor_config::Paths::default(), std::env::vars(), &[]).unwrap();
    assert!(loaded.config.offline.enabled && loaded.config.privacy.mode);
    kor_runtime::apply_network_policy(&l, &loaded.config);
    assert!(kor_runtime::offline_mode() && kor_runtime::privacy_mode());
    assert_eq!(net::policy(), Policy::Offline);

    leaks_fail_fast(&l).await;

    let project = tempfile::tempdir().unwrap();
    let root = project.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hi\"\n}\n",
    )
    .unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("KOR_HOME").unwrap());

    let server = MockServer::start().await;
    let turns = vec![
        call("Read", &json!({"file_path": "src/lib.rs"})),
        call(
            "Edit",
            &json!({"file_path": "src/lib.rs", "old_string": "\"hi\"", "new_string": "\"merhaba\""}),
        ),
        call("Bash", &json!({"command": "echo offline-ok"})),
        call("graph_query", &json!({"query": "greet"})),
        call(
            "remember",
            &json!({"title": "Offline greeting", "body": "greet returns merhaba", "type": "project"}),
        ),
        call(
            "WebFetch",
            &json!({"url": "https://example.com/", "prompt": "summary"}),
        ),
        say("Done: greet now returns merhaba."),
        say("greet returns merhaba."),
    ];
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Script(Mutex::new(turns.into())))
        .mount(&server)
        .await;
    let mut m = Registry::builtin().get("local-vllm").unwrap().clone();
    m.base_url = format!("{}/v1", server.uri());
    kor_runtime::check_provider_allowed(&l, &m).unwrap();
    let provider = kor_providers::build(&m, None).unwrap();

    let files = Files {
        ledger: Some(home.join("usage.jsonl")),
        log: Some(home.join("routing.jsonl")),
    };
    let target = Target {
        provider: provider.clone(),
        model: "qwen-local".into(),
        tool_mode: kor_agent::toolcall::ToolMode::Native,
    };
    let cost = Arc::new(Control::new(
        Lang::En,
        loaded.config.routing.clone(),
        loaded.config.budget.clone(),
        target,
        Vec::new(),
        Prices::default(),
        files,
    ));
    let mut a = agent(&root, &home, provider, cost.clone());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = a
        .send(
            "Change greet to return merhaba and check it.".into(),
            &tx,
            &AllowAll,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(stop, StopReason::Done(FinishReason::Stop));

    // The work happened.
    let src = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(src.contains("\"merhaba\""), "{src}");
    let transcript = serde_json::to_string(&a.messages).unwrap();
    assert!(transcript.contains("offline-ok"), "command output missing");
    assert!(transcript.contains("lib.rs"), "graph answer missing");
    assert!(
        transcript.contains("Offline mode"),
        "web fetch was not refused"
    );
    let memory = kor_agent::memory::layout(&home, &root);
    assert!(
        std::fs::read_dir(&memory.auto_dir).unwrap().count() >= 2,
        "memory not saved"
    );

    // Cost ledger and context breakdown, written locally.
    let ledger = kor_runtime::usage::read(&home.join("usage.jsonl"));
    assert_eq!(ledger.len(), 7, "{ledger:?}");
    let breakdowns = kor_agent::context::read_log(&home.join("context.jsonl"), 100);
    assert_eq!(breakdowns.len(), 7);
    assert!(breakdowns.iter().all(|b| b.tools > 0 && b.system > 0));

    // Knowledge: the first task was recorded, the second one recalls it.
    let kb = a.context.recall.as_ref().unwrap().kb.clone();
    kb.flush().unwrap(); // records go through a writer thread
    assert!(kb.count().unwrap() >= 1);
    a.send(
        "What does greet return?".into(),
        &tx,
        &AllowAll,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(a.context.memory_tokens > 0, "nothing recalled");
    assert_eq!(server.received_requests().await.unwrap().len(), 8);

    // The guard: blocked internet hosts were seen, and only loopback got through.
    let attempts = net::attempts();
    for host in [
        "catalog.kor-leak.invalid",
        "api.github.com",
        "team.kor-leak.invalid",
        "example.com",
    ] {
        assert!(
            attempts.iter().any(|x| x.host == host && !x.allowed),
            "{host} not refused: {attempts:?}"
        );
    }
    assert!(
        attempts
            .iter()
            .filter(|x| x.allowed)
            .all(|x| net::is_loopback_host(&x.host)),
        "{attempts:?}"
    );
    assert!(attempts.iter().any(|x| x.allowed));
}
