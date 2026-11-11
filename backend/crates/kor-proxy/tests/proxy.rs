//! End-to-end: real clients' request shapes through the gateway to wiremock upstreams.
//! Each test starts its own server on 127.0.0.1:0.

use std::sync::Arc;

use kor_providers::{ProviderManifest, Registry};
use kor_proxy::{Access, Gateway, Running, Sources, config};
use kor_secrets::SecretString;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "kor-test-access-key";
const ANTHROPIC_SSE: &str = include_str!("../../kor-providers/tests/fixtures/anthropic.sse");
const OPENAI_SSE: &str = include_str!("../../kor-providers/tests/fixtures/openai_compat.sse");
const GEMINI_SSE: &str = include_str!("../../kor-providers/tests/fixtures/gemini.sse");

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body.to_owned())
}

fn sources(registry: Registry, privacy: bool) -> Sources {
    Sources {
        registry,
        keys: Arc::new(|_| Vec::new()),
        is_local: Arc::new(|m: &ProviderManifest| m.base_url.contains("127.0.0.1")),
        privacy,
        ledger: false,
        list_registry: false,
    }
}

async fn serve_with(yaml: &str, src: Sources, access: Access) -> Running {
    let st = config::parse(yaml).unwrap();
    let gw = Gateway::new(
        st,
        None,
        src,
        access,
        &kor_i18n::Localizer::new(kor_i18n::Lang::En),
    );
    assert!(gw.warnings.is_empty(), "{:?}", gw.warnings);
    kor_proxy::start(Arc::new(gw), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap()
}

async fn serve(yaml: &str) -> Running {
    let access = Access {
        key: Some(SecretString::from(KEY)),
        no_auth: false,
    };
    serve_with(yaml, sources(Registry::builtin(), false), access).await
}

fn url(r: &Running, p: &str) -> String {
    format!("http://{}{p}", r.addr)
}

async fn post(r: &Running, p: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url(r, p))
        .bearer_auth(KEY)
        .body(body.to_string())
        .send()
        .await
        .unwrap()
}

/// `data:` payloads of an SSE body, `[DONE]` excluded.
fn data_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|l| *l != "[DONE]")
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn openai_sdk_stream_through_a_claude_upstream() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-1"))
        .respond_with(sse(ANTHROPIC_SSE))
        .mount(&up)
        .await;
    let r = serve(&format!(
        "api-keys: {{ claude: [ {{ name: claude, base-url: \"{}\", keys: [ {{ api-key: sk-ant-1 }} ] }} ] }}",
        up.uri()
    ))
    .await;
    let resp = post(&r, "/v1/chat/completions", &json!({
        "model": "claude-sonnet-4-5", "stream": true, "stream_options": {"include_usage": true},
        "messages": [{"role": "system", "content": "sys"}, {"role": "user", "content": "selam"}],
        "tools": [{"type": "function", "function": {"name": "read_file", "parameters": {"type": "object"}}}]
    }))
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    let text = resp.text().await.unwrap();
    assert!(text.trim_end().ends_with("data: [DONE]"));
    let chunks = data_lines(&text);
    let content: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(content, "Merhaba");
    let args: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str())
        .collect();
    assert_eq!(
        serde_json::from_str::<Value>(&args).unwrap(),
        json!({"path": "a.rs"})
    );
    let last = chunks.last().unwrap();
    assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(last["usage"]["completion_tokens"], 7);
    // The upstream got a real Messages request: system on top, tool translated.
    let sent: Value =
        serde_json::from_slice(&up.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent["model"], "claude-sonnet-4-5");
    assert_eq!(sent["tools"][0]["name"], "read_file");
    r.stop().await;
}

#[tokio::test]
async fn anthropic_sdk_stream_through_an_openai_compatible_upstream() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer or-1"))
        .respond_with(sse(OPENAI_SSE))
        .mount(&up)
        .await;
    let r = serve(&format!(
        "api-keys:\n  openai-compatibility:\n    - name: router\n      base-url: \"{}/v1\"\n      \
         keys: [ {{ api-key: or-1 }} ]\n      models: [ {{ name: \"moonshotai/kimi-k2\", alias: kimi }} ]",
        up.uri()
    ))
    .await;
    let resp = reqwest::Client::new()
        .post(url(&r, "/v1/messages"))
        .header("x-api-key", KEY)
        .header("anthropic-version", "2023-06-01")
        .body(
            json!({
                "model": "kimi", "max_tokens": 256, "stream": true,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "selam"}]}]
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    let events: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(events.first(), Some(&"message_start"));
    assert_eq!(events.last(), Some(&"message_stop"));
    let data = data_lines(&text);
    let joined: String = data
        .iter()
        .filter_map(|d| d["delta"]["text"].as_str())
        .collect();
    assert_eq!(joined, "Merhaba");
    let tool = data
        .iter()
        .find(|d| d["content_block"]["type"] == "tool_use")
        .unwrap();
    assert_eq!(tool["content_block"]["name"], "read_file");
    let delta = data.iter().find(|d| d["type"] == "message_delta").unwrap();
    assert_eq!(delta["delta"]["stop_reason"], "tool_use");
    assert_eq!(delta["usage"]["output_tokens"], 7);
    let sent: Value =
        serde_json::from_slice(&up.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(
        sent["model"], "moonshotai/kimi-k2",
        "alias mapped to the upstream name"
    );
    r.stop().await;
}

#[tokio::test]
async fn gemini_sdk_stream_and_whole_response_through_gemini_and_openai_upstreams() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-2.5-flash:streamGenerateContent",
        ))
        .and(header("x-goog-api-key", "AIza-1"))
        .respond_with(sse(GEMINI_SSE))
        .mount(&up)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(OPENAI_SSE))
        .mount(&up)
        .await;
    let r = serve(&format!(
        "gemini-api-key:\n  - api-key: AIza-1\n    base-url: \"{u}\"\nopenai-compatibility:\n  - name: oc\n    \
         base-url: \"{u}/v1\"\n    api-key-entries: [ {{ api-key: k }} ]\n    models: [ {{ name: gpt-4o-mini }} ]",
        u = up.uri()
    ))
    .await;
    let body = json!({
        "contents": [{"role": "user", "parts": [{"text": "selam"}]}],
        "generationConfig": {"maxOutputTokens": 64}
    });
    // SSE stream (google-genai uses alt=sse), Gemini client → Gemini upstream.
    let resp = reqwest::Client::new()
        .post(url(
            &r,
            "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
        ))
        .header("x-goog-api-key", KEY)
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let chunks = data_lines(&resp.text().await.unwrap());
    let txt: String = chunks
        .iter()
        .filter_map(|c| c["candidates"][0]["content"]["parts"][0]["text"].as_str())
        .collect();
    assert_eq!(txt, "Merhaba");
    let call = chunks
        .iter()
        .find_map(|c| {
            c["candidates"][0]["content"]["parts"][0]
                .get("functionCall")
                .cloned()
        })
        .unwrap();
    assert_eq!(call["args"]["path"], "a.rs");
    assert_eq!(
        chunks.last().unwrap()["usageMetadata"]["candidatesTokenCount"],
        7
    );

    // Whole response, Gemini client → OpenAI-compatible upstream, key in `?key=`.
    let resp = reqwest::Client::new()
        .post(url(
            &r,
            &format!("/v1beta/models/gpt-4o-mini:generateContent?key={KEY}"),
        ))
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.text().await.unwrap().parse::<Value>().unwrap();
    let parts = &v["candidates"][0]["content"]["parts"];
    assert_eq!(parts[0]["text"], "Merhaba");
    assert_eq!(parts[1]["functionCall"]["name"], "read_file");
    assert_eq!(v["usageMetadata"]["promptTokenCount"], 12);

    // JSON-array streaming (no alt=sse) is one valid JSON document.
    let resp = reqwest::Client::new()
        .post(url(&r, "/v1beta/models/gpt-4o-mini:streamGenerateContent"))
        .header("x-goog-api-key", KEY)
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let arr: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert!(arr.as_array().unwrap().len() >= 3);
    r.stop().await;
}

#[tokio::test]
async fn responses_client_whole_response_through_claude_upstream() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(ANTHROPIC_SSE))
        .mount(&up)
        .await;
    let r = serve(&format!(
        "claude-api-key:\n  - api-key: a\n    base-url: \"{}\"",
        up.uri()
    ))
    .await;
    let resp = post(
        &r,
        "/v1/responses",
        &json!({
            "model": "claude-x", "instructions": "be terse",
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "selam"}]}]
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let v: Value = resp.text().await.unwrap().parse::<Value>().unwrap();
    assert_eq!(v["object"], "response");
    assert_eq!(v["output_text"], "Merhaba");
    let kinds: Vec<&str> = v["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["message", "function_call"]);
    assert_eq!(v["output"][1]["call_id"], "toolu_1");
    r.stop().await;
}

#[tokio::test]
async fn auth_is_required_and_foreign_hosts_are_refused() {
    let r = serve("").await;
    let c = reqwest::Client::new();
    let body = json!({"model": "x", "messages": []});
    let none = c
        .post(url(&r, "/v1/chat/completions"))
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
    let wrong = c
        .post(url(&r, "/v1/chat/completions"))
        .bearer_auth("nope")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    let claude = c
        .post(url(&r, "/v1/messages"))
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(claude.status(), 401);
    assert_eq!(
        claude.text().await.unwrap().parse::<Value>().unwrap()["type"],
        "error"
    );
    let mgmt = c.get(url(&r, "/v0/management/usage")).send().await.unwrap();
    assert_eq!(mgmt.status(), 401);
    // DNS rebinding / cross-site browser requests.
    let rebind = c
        .get(url(&r, "/v1/models"))
        .bearer_auth(KEY)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebind.status(), 403);
    let xsite = c
        .get(url(&r, "/v1/models"))
        .bearer_auth(KEY)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(xsite.status(), 403);
    // Known key, unknown model.
    let unknown = post(
        &r,
        "/v1/chat/completions",
        &json!({"model": "nope", "messages": []}),
    )
    .await;
    assert_eq!(unknown.status(), 404);
    // Health and the page need no key; the page carries no data.
    assert_eq!(
        c.get(url(&r, "/healthz")).send().await.unwrap().status(),
        200
    );
    let page = c.get(url(&r, "/management.html")).send().await.unwrap();
    assert!(
        page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
    r.stop().await;
}

#[tokio::test]
async fn oversized_bodies_and_non_loopback_without_key_are_refused() {
    let r = serve("server: { max-request-body-bytes: 200 }").await;
    let big = json!({"model": "x", "messages": [{"role": "user", "content": "a".repeat(500)}]});
    assert_eq!(post(&r, "/v1/chat/completions", &big).await.status(), 413);
    r.stop().await;

    let st = config::parse("").unwrap();
    let open = Access {
        key: None,
        no_auth: true,
    };
    let gw = Gateway::new(
        st,
        None,
        sources(Registry::default(), false),
        open,
        &kor_i18n::Localizer::new(kor_i18n::Lang::En),
    );
    let err = kor_proxy::start(Arc::new(gw), "0.0.0.0:0".parse().unwrap())
        .await
        .err()
        .unwrap();
    assert!(err.contains("loopback"), "{err}");
}

#[tokio::test]
async fn rate_limited_key_rotates_to_the_next_and_rests() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-api-key", "k1"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("{\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow\"}}"),
        )
        .mount(&up)
        .await;
    Mock::given(method("POST"))
        .and(header("x-api-key", "k2"))
        .respond_with(sse(ANTHROPIC_SSE))
        .mount(&up)
        .await;
    let r = serve(&format!(
        "routing: {{ strategy: fill-first }}\napi-keys: {{ claude: [ {{ name: c, base-url: \"{}\", \
         keys: [ {{ api-key: k1 }}, {{ api-key: k2 }} ] }} ] }}",
        up.uri()
    ))
    .await;
    let body = json!({"model": "claude-x", "messages": [{"role": "user", "content": "hi"}]});
    for _ in 0..2 {
        let resp = post(&r, "/v1/chat/completions", &body).await;
        assert_eq!(resp.status(), 200);
        let v: Value = resp.text().await.unwrap().parse::<Value>().unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "Merhaba");
    }
    let keys: Vec<String> = up
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|q| q.headers["x-api-key"].to_str().unwrap().to_owned())
        .collect();
    // Fill-first would pick k1 again, but it rests for its retry-after.
    assert_eq!(keys, ["k1", "k2", "k2"]);

    let usage: Value = reqwest::Client::new()
        .get(url(&r, "/v0/management/usage"))
        .bearer_auth(KEY)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse()
        .unwrap();
    let apis = &usage["usage"]["apis"];
    assert_eq!(apis["c#1"]["models"]["claude-x"]["rate_limited"], 1);
    assert_eq!(apis["c#2"]["models"]["claude-x"]["success_count"], 2);
    assert_eq!(apis["c#2"]["models"]["claude-x"]["output_tokens"], 14);
    let creds = usage["credentials"].as_array().unwrap();
    assert!(creds[0]["cooling_for_secs"].as_u64().unwrap() > 20);
    assert!(
        !usage.to_string().contains("\"k1\""),
        "raw upstream keys never leave the process"
    );
    r.stop().await;
}

#[tokio::test]
async fn kor_providers_serve_provider_slash_model_and_privacy_keeps_them_local() {
    let up = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(OPENAI_SSE))
        .mount(&up)
        .await;
    let mk = |id: &str, base: &str| {
        ProviderManifest::parse(
            &format!(
                "id = \"{id}\"\nname = \"M\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\n\
                 base_url = \"{base}\"\nauth = {{ kind = \"none\" }}\n[privacy]\ntier = \"standard\"\nretention = \"t\"\n"
            ),
            "test",
        )
        .unwrap()
    };
    let mut reg = Registry::default();
    reg.add_if_absent(mk("mylocal", &format!("{}/v1", up.uri())));
    reg.add_if_absent(mk("remote", "https://remote.example/v1"));
    let access = Access {
        key: None,
        no_auth: true,
    };
    let r = serve_with(
        "kor: { aliases: { fast: \"mylocal/llama3\" } }",
        sources(reg, true),
        access,
    )
    .await;
    let c = reqwest::Client::new();
    let body = |m: &str| json!({"model": m, "messages": [{"role": "user", "content": "hi"}]});
    let ok = c
        .post(url(&r, "/v1/chat/completions"))
        .body(body("mylocal/llama3").to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "--no-auth on loopback");
    let alias = c
        .post(url(&r, "/v1/chat/completions"))
        .body(body("fast").to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(alias.status(), 200);
    let blocked = c
        .post(url(&r, "/v1/chat/completions"))
        .body(body("remote/x").to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 403, "privacy mode");
    let sent: Value =
        serde_json::from_slice(&up.received_requests().await.unwrap()[1].body).unwrap();
    assert_eq!(sent["model"], "llama3");
    let models: Value = c
        .get(url(&r, "/v1/models"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
    assert_eq!(models["data"][0]["id"], "fast");
    r.stop().await;
}

#[tokio::test]
async fn management_config_round_trip_reloads_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("proxy.yaml");
    std::fs::write(
        &file,
        "api-keys: { claude: [ { name: a, keys: [ { api-key: sk-ant-aaaaaaaaaaaa } ] } ] }",
    )
    .unwrap();
    let st = config::load(&file).unwrap();
    let access = Access {
        key: Some(SecretString::from(KEY)),
        no_auth: false,
    };
    let gw = Gateway::new(
        st,
        Some(file.clone()),
        sources(Registry::builtin(), false),
        access,
        &kor_i18n::Localizer::new(kor_i18n::Lang::En),
    );
    let r = kor_proxy::start(Arc::new(gw), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let c = reqwest::Client::new();
    let cfg: Value = c
        .get(url(&r, "/v0/management/config"))
        .header("x-management-key", KEY)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
    assert_eq!(cfg["credentials"][0]["id"], "a");
    assert!(!cfg.to_string().contains("sk-ant-aaaaaaaaaaaa"));
    let bad = c
        .put(url(&r, "/v0/management/config.yaml"))
        .bearer_auth(KEY)
        .body("port: 999999")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 422);
    let good = c
        .put(url(&r, "/v0/management/config.yaml"))
        .bearer_auth(KEY)
        .body("gemini-api-key:\n  - api-key: AIza-zzzzzzzzzzzz\n")
        .send()
        .await
        .unwrap();
    assert_eq!(good.status(), 200);
    let files: Value = c
        .get(url(&r, "/v0/management/auth-files"))
        .bearer_auth(KEY)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
    assert_eq!(files["files"][0]["provider"], "gemini");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("AIza-zzzzzzzzzzzz")
    );
    r.stop().await;
}
