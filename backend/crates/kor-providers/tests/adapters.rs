//! Replay recorded provider streams against a local mock server and check both
//! directions: the request Kor sends and the events it parses.

use futures::StreamExt;
use kor_core::{
    ContentPart, FinishReason, Message, ProviderErrorKind, Role, StreamEvent, ToolSpec, Usage,
};
use kor_providers::{CancellationToken, ChatRequest, EventStream, ProviderManifest, build};
use kor_secrets::SecretString;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-test-key-0123456789abcdef";

fn manifest(adapter: &str, base: &str, auth: &str, extra: &str) -> ProviderManifest {
    let text = format!(
        r#"
id = "mock"
name = "Mock"
category = "standard"
adapter = "{adapter}"
base_url = "{base}"
auth = {auth}
{extra}
[privacy]
tier = "standard"
retention = "test"
"#
    );
    ProviderManifest::parse(&text, "test").unwrap()
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        system: Some("be brief".into()),
        messages: vec![
            Message::text(Role::User, "read a.rs"),
            Message {
                role: Role::Assistant,
                parts: vec![ContentPart::ToolUse {
                    id: "prev_1".into(),
                    name: "read_file".into(),
                    input: json!({"path": "b.rs"}),
                }],
            },
            Message {
                role: Role::User,
                parts: vec![ContentPart::ToolResult {
                    tool_use_id: "prev_1".into(),
                    content: "fn main() {}".into(),
                    is_error: false,
                }],
            },
        ],
        tools: vec![ToolSpec {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        }],
        max_tokens: Some(100),
        temperature: None,
        reasoning: None,
    }
}

#[derive(Debug, Default)]
struct Outcome {
    text: String,
    tools: Vec<(String, String, Value)>,
    usage: Usage,
    finish: Option<FinishReason>,
}

async fn drain(mut s: EventStream) -> Outcome {
    let mut o = Outcome::default();
    let mut args: std::collections::BTreeMap<u32, (String, String, String)> =
        std::collections::BTreeMap::new();
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            StreamEvent::TextDelta { text } => o.text.push_str(&text),
            StreamEvent::ToolCallStart { index, id, name } => {
                args.insert(index, (id, name, String::new()));
            }
            StreamEvent::ToolCallDelta { index, json } => {
                args.get_mut(&index).unwrap().2.push_str(&json);
            }
            StreamEvent::ToolCallEnd { index } => {
                let (id, name, a) = args.remove(&index).unwrap();
                o.tools.push((id, name, serde_json::from_str(&a).unwrap()));
            }
            StreamEvent::Usage { usage } => o.usage = usage,
            StreamEvent::Finish { reason } => o.finish = Some(reason),
            _ => {}
        }
    }
    assert!(args.is_empty(), "unclosed tool calls: {args:?}");
    o
}

fn sse(body: &str, content_type: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", content_type)
        .set_body_string(body)
}

async fn body_of(server: &MockServer) -> Value {
    let reqs = server.received_requests().await.unwrap();
    serde_json::from_slice(&reqs.last().unwrap().body).unwrap()
}

fn assert_standard(o: &Outcome, tool_id: &str) {
    assert_eq!(o.text, "Merhaba");
    assert_eq!(
        o.tools,
        vec![(
            tool_id.to_owned(),
            "read_file".to_owned(),
            json!({"path": "a.rs"})
        )]
    );
    assert_eq!((o.usage.input, o.usage.output), (12, 7));
    assert_eq!(o.finish, Some(FinishReason::ToolUse));
}

#[tokio::test]
async fn openai_compat_round_trip_with_extra_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(sse(
            include_str!("fixtures/openai_compat.sse"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let m = manifest(
        "openai_compat",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "bearer", env = "MOCK_KEY" }"#,
        "[quirks]\nextra_body = { venice_parameters = { include_venice_system_prompt = false } }",
    );
    let p = build(&m, Some(SecretString::from(KEY))).unwrap();
    let o = drain(
        p.complete(request(), CancellationToken::new())
            .await
            .unwrap(),
    )
    .await;
    assert_standard(&o, "call_1");

    let b = body_of(&server).await;
    let roles: Vec<_> = b["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].clone())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool"]);
    assert_eq!(
        b["messages"][2]["tool_calls"][0]["function"]["arguments"],
        "{\"path\":\"b.rs\"}"
    );
    assert_eq!(b["messages"][3]["tool_call_id"], "prev_1");
    assert_eq!(b["stream_options"]["include_usage"], true);
    assert_eq!(
        b["venice_parameters"]["include_venice_system_prompt"],
        false
    );
    assert_eq!(b["tools"][0]["function"]["name"], "read_file");
}

#[tokio::test]
async fn openai_responses_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(sse(
            include_str!("fixtures/openai_responses.sse"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let m = manifest(
        "openai",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "bearer" }"#,
        "",
    );
    let p = build(&m, Some(SecretString::from(KEY))).unwrap();
    let o = drain(
        p.complete(request(), CancellationToken::new())
            .await
            .unwrap(),
    )
    .await;
    assert_standard(&o, "call_1");
    assert_eq!(o.usage.cache_read, 2);

    let b = body_of(&server).await;
    assert_eq!(b["store"], false);
    assert_eq!(b["instructions"], "be brief");
    assert_eq!(b["max_output_tokens"], 100);
    let types: Vec<_> = b["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["type"].clone())
        .collect();
    assert_eq!(
        types,
        [
            Value::Null,
            json!("function_call"),
            json!("function_call_output")
        ]
    );
    assert_eq!(b["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(b["tools"][0]["name"], "read_file");
}

#[tokio::test]
async fn anthropic_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(sse(
            include_str!("fixtures/anthropic.sse"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let m = manifest(
        "anthropic",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "header", header = "x-api-key" }"#,
        "",
    );
    let p = build(&m, Some(SecretString::from(KEY))).unwrap();
    let o = drain(
        p.complete(request(), CancellationToken::new())
            .await
            .unwrap(),
    )
    .await;
    assert_standard(&o, "toolu_1");
    assert_eq!(o.usage.cache_read, 3);

    let b = body_of(&server).await;
    assert_eq!(b["system"][0]["text"], "be brief");
    // Prompt caching: system, last tool and newest message carry a breakpoint.
    assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(b["tools"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(
        b["messages"][2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(b["max_tokens"], 100);
    let roles: Vec<_> = b["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].clone())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
    assert_eq!(b["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(b["tools"][0]["input_schema"]["required"][0], "path");
}

#[tokio::test]
async fn gemini_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/m:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", KEY))
        .respond_with(sse(
            include_str!("fixtures/gemini.sse"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let m = manifest(
        "gemini",
        &format!("{}/v1beta", server.uri()),
        r#"{ kind = "header", header = "x-goog-api-key" }"#,
        "",
    );
    let p = build(&m, Some(SecretString::from(KEY))).unwrap();
    let o = drain(
        p.complete(request(), CancellationToken::new())
            .await
            .unwrap(),
    )
    .await;
    assert_standard(&o, "call_0");

    let b = body_of(&server).await;
    assert_eq!(b["systemInstruction"]["parts"][0]["text"], "be brief");
    let roles: Vec<_> = b["contents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].clone())
        .collect();
    assert_eq!(roles, ["user", "model", "user"]);
    assert_eq!(
        b["contents"][2]["parts"][0]["functionResponse"]["name"],
        "read_file"
    );
    assert_eq!(b["generationConfig"]["maxOutputTokens"], 100);
}

#[tokio::test]
async fn ollama_round_trip_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(sse(
            include_str!("fixtures/ollama.ndjson"),
            "application/x-ndjson",
        ))
        .mount(&server)
        .await;
    let m = manifest("ollama", &server.uri(), r#"{ kind = "none" }"#, "");
    let p = build(&m, None).unwrap();
    let o = drain(
        p.complete(request(), CancellationToken::new())
            .await
            .unwrap(),
    )
    .await;
    assert_standard(&o, "call_0");

    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[0].headers.get("authorization").is_none());
    let b = body_of(&server).await;
    assert_eq!(b["messages"][3]["role"], "tool");
    assert_eq!(b["messages"][3]["tool_name"], "read_file");
    assert_eq!(b["options"]["num_predict"], 100);
}

#[tokio::test]
async fn http_errors_are_classified_and_redacted() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "7")
                .set_body_json(json!({"error": {"message": format!("slow down, key {KEY}")}})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;
    let m = manifest(
        "openai_compat",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "bearer" }"#,
        "",
    );
    let p = build(&m, Some(SecretString::from(KEY))).unwrap();

    let e = p
        .complete(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ProviderErrorKind::RateLimited);
    assert_eq!(e.retry_after, Some(std::time::Duration::from_secs(7)));
    assert_eq!(e.http_status, Some(429));
    assert!(!e.message.contains(KEY), "{}", e.message);

    let e = p
        .complete(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ProviderErrorKind::AuthFailed);
}

#[tokio::test]
async fn truncated_stream_and_error_events_fail() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/chat/completions"))
        .respond_with(sse(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    Mock::given(path("/v1/messages"))
        .respond_with(sse(
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;

    let compat = manifest(
        "openai_compat",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "none" }"#,
        "",
    );
    let mut s = build(&compat, None)
        .unwrap()
        .complete(request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        s.next().await,
        Some(Ok(StreamEvent::TextDelta { .. }))
    ));
    assert_eq!(
        s.next().await.unwrap().unwrap_err().kind,
        ProviderErrorKind::Protocol
    );

    let anth = manifest(
        "anthropic",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "none" }"#,
        "",
    );
    let mut s = build(&anth, None)
        .unwrap()
        .complete(request(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        s.next().await.unwrap().unwrap_err().kind,
        ProviderErrorKind::Unavailable
    );
}

#[tokio::test]
async fn cancellation_ends_the_stream() {
    let server = MockServer::start().await;
    Mock::given(path("/api/chat"))
        .respond_with(sse(
            include_str!("fixtures/ollama.ndjson"),
            "application/x-ndjson",
        ))
        .mount(&server)
        .await;
    let m = manifest("ollama", &server.uri(), r#"{ kind = "none" }"#, "");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut s = build(&m, None)
        .unwrap()
        .complete(request(), cancel)
        .await
        .unwrap();
    assert_eq!(
        s.next().await.unwrap().unwrap(),
        StreamEvent::Finish {
            reason: FinishReason::Cancelled
        }
    );
    assert!(s.next().await.is_none());
}

#[tokio::test]
async fn list_models_per_adapter() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [{"id": "a", "context_length": 8192}, {"id": "b"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "models/g1", "displayName": "G1", "inputTokenLimit": 1000, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/embed", "supportedGenerationMethods": ["embedContent"]}
        ]})))
        .mount(&server)
        .await;
    Mock::given(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"models": [{"name": "llama3:8b"}]})),
        )
        .mount(&server)
        .await;

    let compat = manifest(
        "openai_compat",
        &format!("{}/v1", server.uri()),
        r#"{ kind = "none" }"#,
        "",
    );
    let ms = build(&compat, None).unwrap().list_models().await.unwrap();
    assert_eq!(ms.len(), 2);
    assert_eq!(ms[0].context_window, Some(8192));

    let g = manifest(
        "gemini",
        &format!("{}/v1beta", server.uri()),
        r#"{ kind = "none" }"#,
        "",
    );
    let ms = build(&g, None).unwrap().list_models().await.unwrap();
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].id, "g1");

    let o = manifest("ollama", &server.uri(), r#"{ kind = "none" }"#, "");
    assert_eq!(
        build(&o, None).unwrap().list_models().await.unwrap()[0].id,
        "llama3:8b"
    );
}

#[test]
fn missing_key_and_unsupported_adapter_fail_closed() {
    let m = manifest(
        "openai_compat",
        "https://example.com/v1",
        r#"{ kind = "bearer" }"#,
        "",
    );
    assert_eq!(
        build(&m, None).err().unwrap().kind,
        ProviderErrorKind::AuthFailed
    );
    let m = manifest("bedrock", "https://example.com", r#"{ kind = "none" }"#, "");
    assert_eq!(
        build(&m, None).err().unwrap().kind,
        ProviderErrorKind::InvalidRequest
    );
}

#[tokio::test]
async fn gemini_rejects_path_injection_in_model_id() {
    let m = manifest(
        "gemini",
        "https://example.com/v1beta",
        r#"{ kind = "none" }"#,
        "",
    );
    let mut req = request();
    req.model = "../../evil?x=1".into();
    let e = build(&m, None)
        .unwrap()
        .complete(req, CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ProviderErrorKind::InvalidRequest);
}
