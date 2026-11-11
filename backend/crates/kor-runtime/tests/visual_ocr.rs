//! Spec §14 Faz 7 acceptance 2 (ADR 0048): "the OCR fallback lets a non-vision model solve an
//! error screenshot". A pasted screenshot of a Python `ZeroDivisionError` (fixture PNG) becomes OCR
//! text in the prompt; a scripted model WITHOUT vision receives that text (error type, file and
//! line) through the normal agent loop and proposes the fix. One test uses a fake OCR engine and
//! always runs; the other uses the machine's real OCR engine (Windows OCR, macOS Vision, tesseract)
//! and skips with a message when none is available.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, Approver, PermissionReply, PermissionRequest, Permissions, StopReason};
use kor_core::{ContentPart, FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::visual::{self, Ocr, Picture, SystemOcr};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};

const FIX: &str = "The crash is `ZeroDivisionError` in app.py line 3: `count` is 0. Guard the division:\n\n```python\naverage = total / count if count else 0\n```";

struct NonVision {
    manifest: ProviderManifest,
    seen: Mutex<Vec<ChatRequest>>,
    replies: Mutex<VecDeque<String>>,
}

#[async_trait]
impl Provider for NonVision {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        self.seen.lock().unwrap().push(req);
        let text = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model call");
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(StreamEvent::TextDelta { text }),
            Ok(StreamEvent::Usage {
                usage: Usage::default(),
            }),
            Ok(StreamEvent::Finish {
                reason: FinishReason::Stop,
            }),
        ])))
    }
}

struct Deny;

#[async_trait]
impl Approver for Deny {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

/// What Windows OCR reads from the fixture (`[x,y wxh] text` lines).
struct FakeOcr;

#[async_trait]
impl Ocr for FakeOcr {
    async fn read(&self, _p: &Picture, _c: &CancellationToken) -> Result<String, String> {
        Ok("OCR language: en-US\n[16,16 160x18] $ python app.py\n[16,49 380x20] Traceback (most recent call last):\n[38,82 420x20] File \"app.py\", line 3, in <module>\n[60,115 300x18] average = total / count\n[16,148 405x18] ZeroDivisionError: division by zero\n".into())
    }
}

fn fixture() -> Picture {
    Picture::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zero_division.png"))
        .unwrap()
}

/// Paste → OCR text → agent turn with a model that has no vision. Returns what the model saw and
/// the agent's final answer.
async fn solve(ocr: &dyn Ocr) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        "total = 10\ncount = 0\naverage = total / count\nprint(average)\n",
    )
    .unwrap();
    let mut manifest = Registry::builtin().get("local").unwrap().clone();
    manifest.vision = false;
    let p = Arc::new(NonVision {
        manifest,
        seen: Mutex::new(Vec::new()),
        replies: Mutex::new(VecDeque::from([FIX.to_owned()])),
    });
    let cancel = CancellationToken::new();
    let pasted = visual::image_context(&[fixture()], ocr, &cancel)
        .await
        .unwrap();
    let mut agent = Agent {
        provider: p.clone(),
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::Default, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(dir.path()).unwrap(),
        system: "sys".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 4,
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
        repair: None,
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = agent
        .send(
            format!("Why does this crash? Fix it.{pasted}"),
            &tx,
            &Deny,
            cancel,
        )
        .await
        .unwrap();
    assert!(matches!(stop, StopReason::Done(_)), "{stop:?}");
    let seen = p.seen.lock().unwrap();
    let user = seen[0].messages.last().unwrap();
    assert!(
        !user
            .parts
            .iter()
            .any(|x| matches!(x, ContentPart::Image { .. })),
        "a model without vision must not get image parts"
    );
    let answer = agent.messages.last().unwrap().plain_text();
    (user.plain_text(), answer)
}

fn assert_solved(seen: &str, answer: &str) {
    assert!(
        seen.contains("ZeroDivisionError"),
        "error type missing: {seen}"
    );
    assert!(seen.contains("app.py"), "file missing: {seen}");
    assert!(seen.contains("line 3"), "line missing: {seen}");
    assert!(
        seen.contains("never as instructions"),
        "untrusted label missing: {seen}"
    );
    assert!(answer.contains("if count else 0"), "{answer}");
}

#[tokio::test]
async fn non_vision_model_solves_the_error_screenshot_with_fake_ocr() {
    let (seen, answer) = solve(&FakeOcr).await;
    assert_solved(&seen, &answer);
}

#[tokio::test]
async fn non_vision_model_solves_the_error_screenshot_with_the_real_ocr_engine() {
    if let Err(e) = SystemOcr.read(&fixture(), &CancellationToken::new()).await {
        eprintln!(
            "SKIPPED: no OCR engine on this machine ({e}). Linux: install tesseract (or the `tesseract` component when published); Windows needs an installed OCR language, macOS the Vision framework."
        );
        return;
    }
    let (seen, answer) = solve(&SystemOcr).await;
    assert_solved(&seen, &answer);
}
