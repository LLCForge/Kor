//! §20 end to end with mock providers (ADR 0060): the agent asks a media tool's parameter card
//! before generating, the output lands in the gallery with its sidecar and a media cost line, a
//! derived generation (upscale with the first output as reference) records the chain, a request
//! left by an earlier process is collected, and a Venice video above `media.confirm_above` needs a yes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use kor_agent::media::{CatalogRun, MediaEnv, Studio};
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest, Permissions, gallery};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::fal::Fal;
use kor_providers::media_catalog::{self as catalog, Capability};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_secrets::{Redactor, SecretString};
use kor_tools::{ParamAnswer, ParamCard, Tool, ToolCtx};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Scripted {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
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
        _req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn call(name: &str, args: &Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: "c1".into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: args.to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Usage {
            usage: Usage {
                input: 1,
                output: 1,
                ..Usage::default()
            },
        },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

fn say(t: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta { text: t.into() },
        StreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ]
}

/// Allows the paid call and answers the card with `answer` (recording what it saw).
struct CardAnswer {
    answer: serde_json::Map<String, Value>,
    seen: Mutex<Vec<ParamCard>>,
}

#[async_trait]
impl Approver for CardAnswer {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Allow
    }
    async fn params(&self, card: &ParamCard) -> Option<ParamAnswer> {
        self.seen.lock().unwrap().push(card.clone());
        Some(ParamAnswer {
            values: self.answer.clone(),
            go: true,
        })
    }
}

const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn data_png() -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(PNG)
    )
}

/// A fal mock: two models (text-to-image with a size choice, an upscaler) with schemas and prices,
/// each finishing at once with one PNG (deterministic output).
async fn fal_mock() -> (MockServer, Fal) {
    let server = MockServer::start().await;
    let base = server.uri();
    let openapi = |id: &str, schema: Value| json!({"models": [{"endpoint_id": id, "openapi": {"paths": {format!("/{id}"): {"post": {"requestBody": {"content": {"application/json": {"schema": schema}}}}}}}}]});
    let image_schema = json!({"required": ["prompt"], "properties": {
        "prompt": {"type": "string"},
        "image_size": {"type": "string", "enum": ["square_hd", "landscape_16_9", "portrait_16_9"], "default": "square_hd"},
        "num_images": {"type": "integer", "minimum": 1, "maximum": 4, "default": 1},
        "seed": {"type": "integer"}
    }});
    let upscale_schema = json!({"required": ["image_url"], "properties": {"image_url": {"type": "string"}, "scale": {"type": "integer", "minimum": 2, "maximum": 4, "default": 2}}});
    for (id, schema, price) in [
        ("fal-ai/img", image_schema, 0.003),
        ("fal-ai/esrgan", upscale_schema, 0.01),
    ] {
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("endpoint_id", id))
            .respond_with(ResponseTemplate::new(200).set_body_json(openapi(id, schema)))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/models/pricing")).and(query_param("endpoint_id", id))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prices": [{"endpoint_id": id, "unit_price": price, "unit": "image", "currency": "USD"}]})))
            .mount(&server).await;
        let rid = id.replace('/', "-");
        Mock::given(method("POST"))
            .and(path(format!("/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"request_id": rid, "response_url": format!("{base}/{id}/requests/{rid}")}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{id}/requests/{rid}/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "COMPLETED"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{id}/requests/{rid}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"images": [{"url": data_png(), "content_type": "image/png"}], "seed": 42}),
            ))
            .mount(&server)
            .await;
    }
    let manifest =
        ProviderManifest::parse(include_str!("../../../providers/fal.toml"), "fal.toml").unwrap();
    let fal = Fal::with_bases(
        &manifest,
        SecretString::from("k".to_owned()),
        &base,
        Some(&base),
        "",
    )
    .unwrap();
    (server, fal)
}

fn agent(
    root: &std::path::Path,
    tools: Vec<Arc<dyn Tool>>,
    script: Vec<Vec<StreamEvent>>,
) -> Agent {
    Agent {
        provider: Arc::new(Scripted {
            manifest: Registry::builtin().get("local").unwrap().clone(),
            turns: Mutex::new(script.into()),
        }),
        model: "m".into(),
        tools,
        permissions: Permissions::new(PermissionMode::Default, &[], &[], &[]).unwrap(),
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
        cost: None,
        shadow: None,
    }
}

#[tokio::test]
async fn card_gallery_chain_ledger_and_collect() {
    let (server, fal) = fal_mock().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let ledger = root.join("usage.jsonl");
    let env = MediaEnv {
        ledger: Some(ledger.clone()),
        ..MediaEnv::default()
    };
    let studio = Studio::with_env(Some(fal.clone()), None, env).unwrap();
    let mut tools = kor_agent::media::studio_tools(&studio);
    tools.push(kor_agent::media::gallery_tool());

    // 1. The agent asks for the size in one card before generating (§20.2); "16:9" in the prompt is
    // not an option name here, so the model's own choices are offered.
    let mut answer = serde_json::Map::new();
    answer.insert("image_size".into(), json!("landscape_16_9"));
    let approver = CardAnswer {
        answer,
        seen: Mutex::new(Vec::new()),
    };
    let mut a = agent(
        &root,
        tools.clone(),
        vec![
            call(
                "image_generate",
                &json!({"prompt": "a cat logo", "model": "fal-ai/img", "save_as": "public/logo.png"}),
            ),
            say("done"),
        ],
    );
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send(
        "make a logo".into(),
        &tx,
        &approver,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let cards = approver.seen.lock().unwrap().clone();
    assert_eq!(cards.len(), 1, "asked once");
    let names: Vec<&str> = cards[0].fields.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"image_size"), "{names:?}");
    assert_eq!(
        cards[0].cost.as_deref(),
        Some("$0.00"),
        "list price 0.003 per image"
    );
    assert!(
        root.join("public/logo.png").is_file(),
        "save_as copies into the project"
    );

    let items = gallery::list(&root);
    assert_eq!(items.len(), 1);
    let first = items[0].clone();
    assert_eq!(
        first.params["image_size"], "landscape_16_9",
        "the card's answer was used"
    );
    assert_eq!(first.seed, Some(42), "the provider's seed is kept");
    assert_eq!(
        (first.provider.as_str(), first.capability.as_deref()),
        ("fal", Some("image"))
    );
    let ledger_text = std::fs::read_to_string(&ledger).unwrap();
    assert!(ledger_text.contains("\"media\":true"), "{ledger_text}");

    // 2. "Use as reference" + upscale from the catalog path (CLI / desktop): chain of two.
    let up = catalog::fal_model(
        "fal-ai/esrgan",
        Capability::Upscale,
        Some(&fal.input_schema("fal-ai/esrgan").await.unwrap()),
        None,
    );
    let run = CatalogRun {
        capability: Some(Capability::Upscale),
        reference: Some(first.id.clone()),
        action: Some("upscale".into()),
        ..CatalogRun::default()
    };
    let ctx = ToolCtx::new(&root).unwrap();
    let out = kor_agent::media::run_catalog(&tools, &up, &run, &ctx).await;
    assert!(!out.is_error, "{}", out.content);
    let newest = gallery::list(&root)
        .into_iter()
        .find(|i| i.model == "fal-ai/esrgan")
        .unwrap();
    let chain = gallery::chain(&root, &newest.id);
    assert_eq!(
        chain.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        [newest.id.as_str(), first.id.as_str()]
    );
    assert_eq!(chain[0].action.as_deref(), Some("upscale"));
    let g = tools.iter().find(|t| t.name() == "media_gallery").unwrap();
    let listed = g.call(json!({"id": newest.id}), &ctx).await;
    assert!(
        listed.content.contains(&format!("← {}", first.id)),
        "{}",
        listed.content
    );

    // 3. A request queued by a process that ended: collected into the gallery.
    let job = gallery::Job {
        provider: "fal".into(),
        model: "fal-ai/img".into(),
        request_id: "fal-ai-img".into(),
        path: "/fal-ai/img/requests/fal-ai-img".into(),
        prompt: "late cat".into(),
        stem: "late-cat".into(),
        input: json!({"prompt": "late cat"}),
        capability: Some("image".into()),
        ..gallery::Job::default()
    };
    let id = gallery::job_start(&root, job);
    let pending = gallery::jobs(&root);
    assert_eq!(pending.len(), 1);
    let b = kor_agent::media::Backends {
        fal: Some(&fal),
        venice: None,
    };
    let saved = kor_agent::media::collect(
        &pending[0],
        b,
        &root,
        Some(&ledger),
        &CancellationToken::new(),
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(saved.paths.len(), 1);
    assert!(gallery::jobs(&root).is_empty(), "{id} collected");
    assert!(gallery::list(&root).iter().any(|i| i.prompt == "late cat"));
    drop(server);
}

#[tokio::test]
async fn venice_video_above_the_threshold_needs_a_yes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/models")).and(query_param("type", "video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "wan", "model_spec": {"constraints": {
            "model_type": "text-to-video", "durations": ["5s", "8s"], "resolutions": ["1080p", "720p"], "aspect_ratios": ["16:9", "9:16"]}}}]})))
        .mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/video/quote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"quote": 0.9})))
        .mount(&server)
        .await;
    let mut manifest = Registry::builtin().get("venice").unwrap().clone();
    manifest.base_url = server.uri();
    manifest.base_url_env = None;
    let venice =
        kor_providers::media::VeniceMedia::new(&manifest, SecretString::from("k".to_owned()))
            .unwrap();
    let env = MediaEnv {
        confirm_above: 0.5,
        ..MediaEnv::default()
    };
    let tools = kor_agent::media::tools(&kor_agent::media::Media::with_env(venice, env));
    let video = tools.iter().find(|t| t.name() == "generate_video").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolCtx::new(dir.path()).unwrap();
    // "4k, 10 seconds": this model has neither; the card says so and offers the nearest.
    let card = video
        .param_card(
            &json!({"prompt": "a cat video in 4k, 10 seconds", "model": "wan"}),
            &ctx,
        )
        .await
        .expect("a card");
    assert!(card.confirm, "$0.90 is above 0.50");
    assert_eq!(
        card.cost.as_deref(),
        Some("$0.90"),
        "Venice's quote is exact (no ≈)"
    );
    assert!(
        card.notes
            .iter()
            .any(|n| n.contains("resolution") && n.contains("1080p")),
        "{:?}",
        card.notes
    );
    let dur = card.fields.iter().find(|f| f.name == "duration").unwrap();
    assert_eq!(dur.default, Some(json!("8s")), "nearest to 10 s");
    // An approver that cannot answer: the paid call does not run.
    struct Nobody;
    #[async_trait]
    impl Approver for Nobody {
        async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
            PermissionReply::Allow
        }
    }
    let mut a = agent(
        dir.path(),
        tools.clone(),
        vec![
            call(
                "generate_video",
                &json!({"prompt": "a cat", "model": "wan"}),
            ),
            say("ok"),
        ],
    );
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("video".into(), &tx, &Nobody, CancellationToken::new())
        .await
        .unwrap();
    let refused = a.messages.iter().flat_map(|m| &m.parts).any(|p| matches!(p,
        kor_core::ContentPart::ToolResult { content, is_error: true, .. } if content.contains("media.confirm_above")));
    assert!(refused, "no one could confirm the cost");
}
