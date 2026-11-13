//! `KaaS` API (§11.2) end to end on a real daemon listener with a scripted model: API-key auth,
//! task create → WebSocket events → status → signed callback, ownership, cancel, body limit,
//! and the `OpenAI`-compatible chat endpoint (plain and streaming).

mod common;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use kor_runtime::triggers::daemon::{self, kaas};
use kor_runtime::triggers::webhook;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn call(
    c: &reqwest::Client,
    m: reqwest::Method,
    url: &str,
    key: Option<&str>,
    body: Option<String>,
) -> (u16, Value) {
    let mut r = c.request(m, url);
    if let Some(k) = key {
        r = r.header("authorization", format!("Bearer {k}"));
    }
    if let Some(b) = body {
        r = r.header("content-type", "application/json").body(b);
    }
    let res = r.send().await.unwrap();
    let status = res.status().as_u16();
    let text = res.text().await.unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)] // one end-to-end story
async fn tasks_events_callback_and_chat_completions() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("daemon");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("README.md"), "demo\n").unwrap();
    let (_, key) = kaas::create_key(&state, "ci", Some(&work)).unwrap();
    let (_, other) = kaas::create_key(&state, "other", Some(&work)).unwrap();

    let running = daemon::start(daemon::Options {
        l: kor_i18n::Localizer::new(kor_i18n::Lang::En),
        dir: state.clone(),
        user_config: None,
        listen: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        approval_timeout: Duration::from_secs(5),
        factory: Arc::new(common::Factory::default()),
        notifiers: Vec::new(),
        secrets: Arc::new(|_| None),
        tick: Duration::from_millis(100),
        routines: false,
    })
    .await
    .unwrap();
    let base = format!("http://{}", running.addr);
    let c = reqwest::Client::new();
    let post = reqwest::Method::POST;
    let get = reqwest::Method::GET;

    // Auth: no key, a wrong key.
    let (s, _) = call(&c, get.clone(), &format!("{base}/v1/models"), None, None).await;
    assert_eq!(s, 401);
    let (s, _) = call(
        &c,
        get.clone(),
        &format!("{base}/v1/models"),
        Some("kor_nope"),
        None,
    )
    .await;
    assert_eq!(s, 401);
    let (s, v) = call(
        &c,
        get.clone(),
        &format!("{base}/v1/models"),
        Some(&key),
        None,
    )
    .await;
    assert_eq!((s, v["data"][0]["id"].as_str()), (200, Some("kor")));

    // Validation: bad callback, unknown field, a project the key may not use, body limit.
    let tasks = format!("{base}/api/v1/tasks");
    for body in [
        json!({ "task": "x", "callback": "ftp://example.com/x" }),
        json!({ "task": "x", "nope": 1 }),
        json!({ "task": "" }),
        json!({ "task": "x", "budget_usd": -1 }),
    ] {
        let (s, _) = call(&c, post.clone(), &tasks, Some(&key), Some(body.to_string())).await;
        assert_eq!(s, 400, "{body}");
    }
    let (s, _) = call(
        &c,
        post.clone(),
        &tasks,
        Some(&key),
        Some(json!({ "task": "x", "repo": tmp.path() }).to_string()),
    )
    .await;
    assert_eq!(s, 404, "only the key's project or a watched one");
    let big = format!(r#"{{"task":"{}"}}"#, "a".repeat(2 * 1024 * 1024));
    let (s, _) = call(&c, post.clone(), &tasks, Some(&key), Some(big)).await;
    assert_eq!(s, 413);

    // A task with a callback.
    let hooks = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/kor"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&hooks)
        .await;
    let (s, created) = call(
        &c,
        post.clone(),
        &tasks,
        Some(&key),
        Some(
            json!({ "task": "Review the project", "budget_usd": 0.5, "callback": format!("{}/kor", hooks.uri()) })
                .to_string(),
        ),
    )
    .await;
    assert_eq!(s, 202, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let secret = created["callback_secret"].as_str().unwrap().to_owned();
    assert_eq!(created["events"], format!("/api/v1/tasks/{id}/events"));

    // WebSocket: the task snapshot first, `done` last.
    let mut req = format!("ws://{}/api/v1/tasks/{id}/events", running.addr)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {key}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let mut seen = Vec::new();
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(30), ws.next()).await {
        if let Ok(t) = m.to_text()
            && let Ok(v) = serde_json::from_str::<Value>(t)
        {
            seen.push(v);
        }
    }
    assert_eq!(seen.first().unwrap()["type"], "task", "{seen:?}");
    let last = seen.last().unwrap();
    assert_eq!(last["type"], "done", "{seen:?}");
    assert_eq!(last["task"]["status"], "done", "{seen:?}");
    assert_eq!(last["task"]["summary"], common::REPLY);

    let (s, v) = call(&c, get.clone(), &format!("{tasks}/{id}"), Some(&key), None).await;
    assert_eq!((s, v["status"].as_str()), (200, Some("done")));
    assert!(v.get("callback_secret").is_none() && v.get("transcript").is_none());
    // Another key sees nothing of it.
    let (s, _) = call(
        &c,
        get.clone(),
        &format!("{tasks}/{id}"),
        Some(&other),
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = call(
        &c,
        post.clone(),
        &format!("{tasks}/{id}/cancel"),
        Some(&key),
        None,
    )
    .await;
    assert_eq!(s, 409, "finished tasks cannot be cancelled");

    // The callback arrives signed with the task's secret.
    let mut got = None;
    for _ in 0..100 {
        if let Some(r) = hooks
            .received_requests()
            .await
            .and_then(|r| r.into_iter().next())
        {
            got = Some(r);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let got = got.expect("callback");
    let sig = got
        .headers
        .get("x-kor-signature")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(sig, webhook::sign(secret.as_bytes(), &got.body));
    let body: Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(
        (body["event"].as_str(), body["task"]["id"].as_str()),
        (Some("task.finished"), Some(id.as_str()))
    );

    // OpenAI-compatible chat, plain.
    let chat = format!("{base}/v1/chat/completions");
    let msgs = json!([{ "role": "system", "content": "be brief" }, { "role": "user", "content": "What is this project?" }]);
    let (s, v) = call(
        &c,
        post.clone(),
        &chat,
        Some(&key),
        Some(json!({ "model": "kor", "messages": msgs, "temperature": 0.2 }).to_string()),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["object"], "chat.completion");
    assert_eq!(v["choices"][0]["message"]["content"], common::REPLY);
    assert_eq!(v["usage"]["completion_tokens"], 5);
    let (s, _) = call(
        &c,
        post.clone(),
        &chat,
        Some(&key),
        Some(json!({ "model": "gpt-4o", "messages": msgs }).to_string()),
    )
    .await;
    assert_eq!(s, 404);

    // Streaming.
    let res = c
        .post(&chat)
        .header("authorization", format!("Bearer {key}"))
        .body(json!({ "model": "kor", "messages": msgs, "stream": true }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.headers()["content-type"].to_str().unwrap(),
        "text/event-stream"
    );
    let text = res.text().await.unwrap();
    let chunks: Vec<Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|d| *d != "[DONE]")
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();
    let content: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(content, common::REPLY, "{text}");
    assert_eq!(
        chunks.last().unwrap()["choices"][0]["finish_reason"],
        "stop"
    );
    assert!(text.trim_end().ends_with("data: [DONE]"), "{text}");

    // A revoked key stops working at once.
    let keys = kaas::list_keys(&state);
    assert!(kaas::revoke_key(&state, &keys[0].id).unwrap());
    let (s, _) = call(&c, get, &format!("{base}/v1/models"), Some(&key), None).await;
    assert_eq!(s, 401);

    running.stop().await;
}
