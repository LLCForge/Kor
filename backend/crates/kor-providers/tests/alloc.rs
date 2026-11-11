//! §32.2 (ADR 0062): allocation profile (dhat) of the token streaming hot path, in CI. A provider
//! stream of 2000 SSE deltas from a mock server may allocate at most `MAX_PER_TOKEN` blocks per
//! token end to end (HTTP chunk, SSE frame, JSON parse, event); a regression that, say, clones the
//! whole buffer per event fails here.

use futures::StreamExt;
use kor_core::{Message, Role, StreamEvent};
use kor_providers::{CancellationToken, ChatRequest, ProviderManifest, build};
use kor_secrets::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const TOKENS: usize = 2000;
/// Measured ≈ see ADR 0062; the bound leaves headroom for allocator and parser noise.
const MAX_PER_TOKEN: u64 = 40;

#[tokio::test]
async fn streaming_allocations_per_token_stay_bounded() {
    let mut body = String::new();
    for i in 0..TOKENS {
        body.push_str(&format!(
            "data: {{\"id\":\"1\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"t{i} \"}},\"finish_reason\":null}}]}}\n\n"
        ));
    }
    body.push_str("data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    let m = ProviderManifest::parse(
        &format!(
            "id = \"mock\"\nname = \"Mock\"\ncategory = \"standard\"\nadapter = \"openai_compat\"\nbase_url = \"{}/v1\"\n\
             auth = {{ kind = \"bearer\", env = \"MOCK_KEY\" }}\n[privacy]\ntier = \"standard\"\nretention = \"test\"\n",
            server.uri()
        ),
        "test",
    )
    .unwrap();
    let p = build(&m, Some(SecretString::from("sk-test"))).unwrap();
    let req = ChatRequest {
        model: "m".into(),
        system: None,
        messages: vec![Message::text(Role::User, "go")],
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        reasoning: None,
    };
    let mut stream = p.complete(req, CancellationToken::new()).await.unwrap();
    // Measure only the stream itself: connection set-up and the request are outside.
    let profiler = dhat::Profiler::builder().testing().build();
    let mut tokens = 0;
    while let Some(ev) = stream.next().await {
        if let Ok(StreamEvent::TextDelta { .. }) = ev {
            tokens += 1;
        }
    }
    let stats = dhat::HeapStats::get();
    drop(profiler);
    assert_eq!(tokens, TOKENS);
    let per_token = stats.total_blocks / TOKENS as u64;
    println!(
        "allocations per streamed token: {per_token} ({} blocks, {} bytes)",
        stats.total_blocks, stats.total_bytes
    );
    assert!(
        per_token <= MAX_PER_TOKEN,
        "{per_token} allocations per token > {MAX_PER_TOKEN}"
    );
}
