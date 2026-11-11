#![allow(clippy::format_collect, clippy::many_single_char_names)]
//! Wire-level checks for the provider layer additions: Cohere v2, auth variants (fal `Key`,
//! Azure `api-key` + deployment mapping) and key-pool / fallback failover against mock servers.

use futures::StreamExt;
use kor_core::{FinishReason, Message, ProviderErrorKind, Role, StreamEvent, ToolSpec};
use kor_providers::{
    CancellationToken, ChatRequest, ProviderManifest, Registry, build, build_pool, health,
    with_fallbacks,
};
use kor_secrets::SecretString;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK_SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn manifest(id: &str, adapter: &str, base: &str, auth: &str, extra: &str) -> ProviderManifest {
    let text = format!(
        "id = \"{id}\"\nname = \"M\"\ncategory = \"standard\"\nadapter = \"{adapter}\"\n\
         base_url = \"{base}\"\nauth = {auth}\n{extra}\n[privacy]\ntier = \"standard\"\nretention = \"t\"\n"
    );
    ProviderManifest::parse(&text, "test").unwrap()
}

fn req(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages: vec![Message::text(Role::User, "hi")],
        ..ChatRequest::default()
    }
}

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body.to_owned())
}

async fn text_of(
    p: &dyn kor_providers::Provider,
    r: ChatRequest,
) -> Result<String, ProviderErrorKind> {
    let mut s = p
        .complete(r, CancellationToken::new())
        .await
        .map_err(|e| e.kind)?;
    let mut out = String::new();
    while let Some(ev) = s.next().await {
        if let StreamEvent::TextDelta { text } = ev.map_err(|e| e.kind)? {
            out.push_str(&text);
        }
    }
    Ok(out)
}

async fn last_body(s: &MockServer) -> Value {
    let reqs = s.received_requests().await.unwrap();
    serde_json::from_slice(&reqs.last().unwrap().body).unwrap()
}

#[tokio::test]
async fn cohere_v2_round_trip() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/chat"))
        .and(header("authorization", "Bearer co-key"))
        .respond_with(sse(include_str!("fixtures/cohere.sse")))
        .mount(&s)
        .await;
    let m = manifest(
        "co",
        "cohere",
        &s.uri(),
        r#"{ kind = "bearer", env = "CO_API_KEY" }"#,
        "",
    );
    let p = build(&m, Some(SecretString::from("co-key"))).unwrap();
    let mut r = req("command-a");
    r.messages.push(Message {
        role: Role::Assistant,
        parts: vec![kor_core::ContentPart::ToolUse {
            id: "t0".into(),
            name: "read_file".into(),
            input: json!({}),
        }],
    });
    r.tools = vec![ToolSpec {
        name: "read_file".into(),
        description: "Read".into(),
        input_schema: json!({"type": "object"}),
    }];
    let mut st = p.complete(r, CancellationToken::new()).await.unwrap();
    let (mut text, mut args, mut finish, mut usage) = (String::new(), String::new(), None, None);
    while let Some(ev) = st.next().await {
        match ev.unwrap() {
            StreamEvent::TextDelta { text: t } => text.push_str(&t),
            StreamEvent::ToolCallDelta { json, .. } => args.push_str(&json),
            StreamEvent::Finish { reason } => finish = Some(reason),
            StreamEvent::Usage { usage: u } => usage = Some(u),
            _ => {}
        }
    }
    assert_eq!(text, "Merhaba");
    assert_eq!(
        serde_json::from_str::<Value>(&args).unwrap(),
        json!({"path": "a.rs"})
    );
    assert_eq!(finish, Some(FinishReason::ToolUse));
    let u = usage.unwrap();
    assert_eq!((u.input, u.output, u.cache_read), (12, 7, 2));
    let b = last_body(&s).await;
    assert!(b.get("stream_options").is_none());
    assert_eq!(b["stream"], true);
    // Tool-only assistant turn: no empty content field.
    assert!(b["messages"][1].get("content").is_none());
    assert_eq!(b["tools"][0]["function"]["name"], "read_file");
}

#[tokio::test]
async fn fal_key_scheme_and_azure_api_key_with_deployment_map() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal/chat/completions"))
        .and(header("authorization", "Key fal-secret"))
        .respond_with(sse(OK_SSE))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .and(header("api-key", "az-secret"))
        .respond_with(sse(OK_SSE))
        .mount(&s)
        .await;

    let mut fal = Registry::builtin().get("fal").unwrap().clone();
    fal.base_url = format!("{}/fal", s.uri());
    let p = build(&fal, Some(SecretString::from("fal-secret"))).unwrap();
    assert_eq!(text_of(p.as_ref(), req("m")).await.unwrap(), "hi");

    let m = manifest(
        "az",
        "openai_compat",
        &format!("{}/openai/v1", s.uri()),
        r#"{ kind = "header", env = "AZ_KEY", header = "api-key" }"#,
        "[quirks]\nmodel_map = { \"gpt-4o\" = \"my-gpt4o-deploy\" }",
    );
    let p = build(&m, Some(SecretString::from("az-secret"))).unwrap();
    assert_eq!(text_of(p.as_ref(), req("gpt-4o")).await.unwrap(), "hi");
    assert_eq!(last_body(&s).await["model"], "my-gpt4o-deploy");
}

#[tokio::test]
async fn placeholder_base_urls_need_their_env_var() {
    let reg = Registry::builtin();
    for id in ["azure", "vertex", "bedrock"] {
        let m = reg.get(id).unwrap();
        let var = m.base_url_env.clone().unwrap();
        if std::env::var_os(&var).is_some() {
            continue;
        }
        let e = build(m, Some(SecretString::from("k"))).err().unwrap();
        assert!(e.message.contains(&var), "{id}: {}", e.message);
    }
    let e = build(reg.get("sagemaker").unwrap(), None).err().unwrap();
    assert!(e.message.contains("SigV4"));
    let e = build(reg.get("github-models").unwrap(), None)
        .err()
        .unwrap();
    assert!(e.message.contains("retired"));
}

/// Two keys of one provider, then a fallback provider: key 1 is rate-limited, key 2 gets 503,
/// the fallback answers. The caller sees one clean stream.
#[tokio::test]
async fn key_pool_then_fallback_provider_within_one_call() {
    let primary = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer key-one"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "5")
                .set_body_string("{\"error\":{\"message\":\"slow down\"}}"),
        )
        .mount(&primary)
        .await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer key-two"))
        .respond_with(ResponseTemplate::new(503).set_body_string("{\"error\":\"down\"}"))
        .mount(&primary)
        .await;
    let backup = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(OK_SSE))
        .mount(&backup)
        .await;

    let auth = r#"{ kind = "bearer", env = "P_KEY" }"#;
    let pm = manifest(
        "pool-primary",
        "openai_compat",
        &format!("{}/v1", primary.uri()),
        auth,
        "",
    );
    let bm = manifest(
        "pool-backup",
        "openai_compat",
        &format!("{}/v1", backup.uri()),
        auth,
        "",
    );
    let pool = build_pool(
        &pm,
        vec![SecretString::from("key-one"), SecretString::from("key-two")],
    )
    .unwrap();
    let fb = build(&bm, Some(SecretString::from("key-b"))).unwrap();
    let p = with_fallbacks(pool, vec![(fb, "backup-model".into())]);

    assert_eq!(
        text_of(p.as_ref(), req("primary-model")).await.unwrap(),
        "hi"
    );
    assert_eq!(last_body(&backup).await["model"], "backup-model");
    assert_eq!(primary.received_requests().await.unwrap().len(), 2);
    assert_eq!(p.manifest().id, "pool-primary");

    let h1 = health::get("pool-primary").unwrap();
    let h2 = health::get("pool-primary#2").unwrap();
    let hb = health::get("pool-backup").unwrap();
    assert!(h1.error_rate() > 0.99 && h2.error_rate() > 0.99);
    assert!(hb.error_rate() < 0.01);

    // Next call: key one is resting (retry-after 5 s), so key two is asked first.
    let _ = text_of(p.as_ref(), req("primary-model")).await;
    let reqs = primary.received_requests().await.unwrap();
    assert_eq!(reqs[2].headers["authorization"], "Bearer key-two");
}

#[tokio::test]
async fn auth_failure_is_not_hidden_by_the_pool() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"bad key\"}"))
        .mount(&s)
        .await;
    let m = manifest(
        "pool-auth",
        "openai_compat",
        &format!("{}/v1", s.uri()),
        r#"{ kind = "bearer", env = "P_KEY" }"#,
        "",
    );
    let p = build_pool(&m, vec![SecretString::from("a"), SecretString::from("b")]).unwrap();
    assert_eq!(
        text_of(p.as_ref(), req("m")).await.err(),
        Some(ProviderErrorKind::AuthFailed)
    );
    assert_eq!(s.received_requests().await.unwrap().len(), 1);
}
