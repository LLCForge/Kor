#![allow(clippy::format_collect, clippy::many_single_char_names)]
//! Faz 1 acceptance (Master Build Prompt §14): one agent task gives the same result with a local
//! Ollama model and three cloud wire formats (`OpenAI`-compatible chat, Anthropic Messages,
//! Gemini). Each mock server speaks its real protocol; the agent loop, tools and adapters are
//! the production ones. The model asks to write a file, then finishes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest, Permissions, StopReason};
use kor_core::{FinishReason, PermissionMode, Usage};
use kor_providers::{CancellationToken, Registry, build};
use kor_secrets::{Redactor, SecretString};
use kor_tools::{ToolCtx, default_tools};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const FILE: &str = "hello.txt";
const CONTENT: &str = "Merhaba from Kor\nline two\n";

fn args() -> Value {
    json!({"file_path": FILE, "content": CONTENT})
}

fn sse(frames: &[Value]) -> String {
    frames.iter().map(|f| format!("data: {f}\n\n")).collect()
}

fn anthropic_sse(frames: &[Value]) -> String {
    frames
        .iter()
        .map(|f| format!("event: {}\ndata: {f}\n\n", f["type"].as_str().unwrap()))
        .collect()
}

/// (tool-call turn, final turn, content type) per wire format.
fn script(format: &str) -> (String, String, &'static str) {
    let a = args();
    match format {
        "ollama" => {
            let call = [
                json!({"message": {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "Write", "arguments": a}}]}, "done": false}),
                json!({"message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop", "prompt_eval_count": 5, "eval_count": 3}),
            ];
            let done = [
                json!({"message": {"role": "assistant", "content": "Done."}, "done": false}),
                json!({"message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop", "prompt_eval_count": 9, "eval_count": 2}),
            ];
            let nd = |v: &[Value]| v.iter().map(|x| format!("{x}\n")).collect::<String>();
            (nd(&call), nd(&done), "application/x-ndjson")
        }
        "openai_compat" => (
            sse(&[
                json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "c1", "type": "function", "function": {"name": "Write", "arguments": a.to_string()}}]}}]}),
                json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
                json!({"choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 3}}),
            ]) + "data: [DONE]\n\n",
            sse(&[
                json!({"choices": [{"index": 0, "delta": {"content": "Done."}}]}),
                json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
            ]) + "data: [DONE]\n\n",
            "text/event-stream",
        ),
        "anthropic" => (
            anthropic_sse(&[
                json!({"type": "message_start", "message": {"id": "m1", "role": "assistant", "content": [], "usage": {"input_tokens": 5, "output_tokens": 1}}}),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "Write", "input": {}}}),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": a.to_string()}}),
                json!({"type": "content_block_stop", "index": 0}),
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 3}}),
                json!({"type": "message_stop"}),
            ]),
            anthropic_sse(&[
                json!({"type": "message_start", "message": {"id": "m2", "role": "assistant", "content": [], "usage": {"input_tokens": 9, "output_tokens": 1}}}),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Done."}}),
                json!({"type": "content_block_stop", "index": 0}),
                json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 2}}),
                json!({"type": "message_stop"}),
            ]),
            "text/event-stream",
        ),
        "gemini" => (
            sse(&[
                json!({"candidates": [{"content": {"parts": [{"functionCall": {"name": "Write", "args": a}}], "role": "model"}, "finishReason": "STOP", "index": 0}], "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 3}}),
            ]),
            sse(&[
                json!({"candidates": [{"content": {"parts": [{"text": "Done."}], "role": "model"}, "finishReason": "STOP", "index": 0}], "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 2}}),
            ]),
            "text/event-stream",
        ),
        other => panic!("unknown format {other}"),
    }
}

/// First call: the tool call; every later call: the final answer. Records request bodies.
struct Turns {
    n: AtomicUsize,
    call: String,
    done: String,
    content_type: &'static str,
}

impl Respond for Turns {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let body = if self.n.fetch_add(1, Ordering::SeqCst) == 0 {
            &self.call
        } else {
            &self.done
        };
        ResponseTemplate::new(200)
            .insert_header("content-type", self.content_type)
            .set_body_string(body.clone())
    }
}

struct AllowAll;

#[async_trait]
impl Approver for AllowAll {
    async fn approve(&self, _: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
}

/// Run the task against `provider_id` (a built-in manifest re-pointed at a mock server).
async fn run(
    provider_id: &str,
    format: &str,
    base_path: &str,
    model: &str,
) -> (String, Vec<Value>) {
    let server = MockServer::start().await;
    let (call, done, content_type) = script(format);
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Turns {
            n: AtomicUsize::new(0),
            call,
            done,
            content_type,
        })
        .mount(&server)
        .await;
    let mut m = Registry::builtin().get(provider_id).unwrap().clone();
    m.base_url = format!("{}{base_path}", server.uri());
    let key = m
        .needs_key()
        .then(|| SecretString::from("test-key-0123456789"));
    let provider = build(&m, key).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent {
        provider: Arc::clone(&provider),
        model: model.into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(dir.path()).unwrap(),
        system: "You are a coding agent.".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 5,
        max_tokens: Some(256),
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
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = agent
        .send(
            format!("Create {FILE} with a greeting."),
            &tx,
            &AllowAll,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(stop, StopReason::Done(FinishReason::Stop), "{provider_id}");
    let written = std::fs::read_to_string(dir.path().join(FILE)).unwrap_or_default();
    let bodies = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    (written, bodies)
}

#[tokio::test]
async fn same_task_same_files_on_local_and_three_cloud_formats() {
    let runs = [
        ("local", "ollama", "", "qwen2.5-coder"),
        ("groq", "openai_compat", "/openai/v1", "llama-3.3-70b"),
        ("anthropic", "anthropic", "/v1", "claude-sonnet-4-5"),
        ("google", "gemini", "/v1beta", "gemini-2.5-flash"),
    ];
    for (id, format, base, model) in runs {
        let (written, bodies) = run(id, format, base, model).await;
        assert_eq!(written, CONTENT, "{id}: file differs");
        assert_eq!(
            bodies.len(),
            2,
            "{id}: expected a tool turn and a final turn"
        );
        // The second request carries the tool result back in the provider's own shape.
        let second = bodies[1].to_string();
        assert!(second.contains("Write"), "{id}: tool call not replayed");
        assert!(second.contains(FILE), "{id}: tool result missing");
    }
}
