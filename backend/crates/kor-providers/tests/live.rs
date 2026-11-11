//! Live checks against real providers. Ignored by default; they cost tokens.
//!
//! `KOR_LIVE_VENICE_MODEL=<model> cargo test -p kor-providers --test live -- --ignored`

use futures::StreamExt;
use kor_core::{FinishReason, Message, Role, StreamEvent, ToolSpec};
use kor_providers::{CancellationToken, ChatRequest, Registry, build};
use kor_secrets::SecretString;
use serde_json::json;

fn venice() -> Option<(std::sync::Arc<dyn kor_providers::Provider>, String)> {
    let key = std::env::var("VENICE_API_KEY").ok()?;
    let model = std::env::var("KOR_LIVE_VENICE_MODEL").ok()?;
    let reg = Registry::builtin();
    let p = build(reg.get("venice")?, Some(SecretString::from(key))).ok()?;
    Some((p, model))
}

#[tokio::test]
#[ignore = "live API, costs tokens"]
async fn venice_streams_text_and_calls_a_tool() {
    let Some((p, model)) = venice() else {
        eprintln!("skipped: set VENICE_API_KEY and KOR_LIVE_VENICE_MODEL");
        return;
    };

    let req = ChatRequest {
        model: model.clone(),
        messages: vec![Message::text(Role::User, "Reply with exactly: pong")],
        max_tokens: Some(200),
        ..ChatRequest::default()
    };
    let mut s = p.complete(req, CancellationToken::new()).await.unwrap();
    let (mut text, mut finish) = (String::new(), None);
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            StreamEvent::TextDelta { text: t } => text.push_str(&t),
            StreamEvent::Finish { reason } => finish = Some(reason),
            _ => {}
        }
    }
    eprintln!("text: {text:?}");
    assert!(text.to_lowercase().contains("pong"));
    assert!(matches!(
        finish,
        Some(FinishReason::Stop | FinishReason::MaxTokens)
    ));

    let req = ChatRequest {
        model,
        messages: vec![Message::text(
            Role::User,
            "What is the weather in Ankara? Use the tool.",
        )],
        tools: vec![ToolSpec {
            name: "get_weather".into(),
            description: "Current weather for a city".into(),
            input_schema: json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
        }],
        max_tokens: Some(300),
        ..ChatRequest::default()
    };
    let mut s = p.complete(req, CancellationToken::new()).await.unwrap();
    let (mut name, mut args, mut finish) = (String::new(), String::new(), None);
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            StreamEvent::ToolCallStart { name: n, .. } => name = n,
            StreamEvent::ToolCallDelta { json, .. } => args.push_str(&json),
            StreamEvent::Finish { reason } => finish = Some(reason),
            _ => {}
        }
    }
    eprintln!("tool: {name} {args}");
    assert_eq!(name, "get_weather");
    let v: serde_json::Value = serde_json::from_str(&args).unwrap();
    assert!(
        v["city"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("ankara")
    );
    assert_eq!(finish, Some(FinishReason::ToolUse));
}
