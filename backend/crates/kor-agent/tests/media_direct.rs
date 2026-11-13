//! `direct_generate` (§20) end to end against mocks: a local `ComfyUI` job is queued, polled and
//! saved into the gallery; a paid Stability image is charged to the usage ledger at list price.

use kor_agent::media::{DirectMedia, MediaEnv, direct_tools};
use kor_providers::ProviderManifest;
use kor_providers::direct::{Direct, DirectKind};
use kor_secrets::SecretString;
use kor_tools::ToolCtx;
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn manifest(kind: DirectKind, server: &MockServer) -> ProviderManifest {
    let def = match kind {
        DirectKind::ComfyUi => "http://127.0.0.1:8188",
        _ => "https://api.stability.ai",
    };
    let text = kind.manifest_toml().unwrap().replace(def, &server.uri());
    ProviderManifest::parse(&text, kind.id()).unwrap()
}

#[tokio::test]
async fn comfyui_and_stability_through_the_tool() {
    let s = MockServer::start().await;
    Mock::given(path("/prompt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"prompt_id": "p1", "node_errors": {}})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/history/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"p1": {"status": {"status_str": "success"},
            "outputs": {"9": {"images": [{"filename": "kor_0001.png", "subfolder": "", "type": "output"}]}}}})))
        .mount(&s)
        .await;
    Mock::given(path("/view"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"\x89PNG".to_vec()),
        )
        .mount(&s)
        .await;
    let png = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"\x89PNG");
    Mock::given(path("/v2beta/stable-image/generate/core"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"image": png, "finish_reason": "SUCCESS"})),
        )
        .mount(&s)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("usage.jsonl");
    let clients = vec![
        Direct::new(
            DirectKind::ComfyUi,
            &manifest(DirectKind::ComfyUi, &s),
            None,
        )
        .unwrap(),
        Direct::new(
            DirectKind::Stability,
            &manifest(DirectKind::Stability, &s),
            Some(SecretString::from("sk-test".to_owned())),
        )
        .unwrap(),
    ];
    let env = MediaEnv {
        ledger: Some(ledger.clone()),
        ..MediaEnv::default()
    };
    let d = DirectMedia::new(clients, "sd_xl_base_1.0.safetensors".into(), env).unwrap();
    let tool = direct_tools(&d).remove(0);
    assert_eq!(tool.name(), "direct_generate");
    let ctx = ToolCtx::new(dir.path()).unwrap();

    let out = tool
        .call(
            json!({"provider": "comfyui", "prompt": "a lighthouse", "seed": 3}),
            &ctx,
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("saved: "), "{}", out.content);

    let out = tool
        .call(
            json!({"provider": "stability", "prompt": "a lighthouse"}),
            &ctx,
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    let charged = std::fs::read_to_string(&ledger).unwrap();
    assert!(charged.contains("stability/core"), "{charged}");

    let out = tool
        .call(
            json!({"provider": "stability", "kind": "video", "prompt": "x"}),
            &ctx,
        )
        .await;
    assert!(out.is_error, "Stability makes images only");
    let out = tool
        .call(json!({"provider": "veo", "prompt": "x"}), &ctx)
        .await;
    assert!(out.is_error, "not configured");
}
