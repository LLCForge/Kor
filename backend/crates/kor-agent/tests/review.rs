//! Per-file review before a write (§12.3, ADR 0030): accept a subset, edited content,
//! reject all, and the plain yes/no answer, against a scripted model and a real directory.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{
    Agent, Approver, FileDecision, PermissionReply, PermissionRequest, Permissions, Review,
};
use kor_core::{ContentPart, FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::json;

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
            .expect("extra model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn call(name: &str, args: &serde_json::Value) -> Vec<StreamEvent> {
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
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

fn done() -> Vec<StreamEvent> {
    vec![StreamEvent::Finish {
        reason: FinishReason::Stop,
    }]
}

/// Answers with a fixed review (or only `approve`, like every existing approver) and keeps the requests.
struct Reviewer(
    Option<Review>,
    PermissionReply,
    Mutex<Vec<PermissionRequest>>,
);

#[async_trait]
impl Approver for Reviewer {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.2.lock().unwrap().push(req.clone());
        self.1
    }
    async fn review(&self, req: &PermissionRequest) -> Review {
        match &self.0 {
            Some(r) => {
                self.2.lock().unwrap().push(req.clone());
                r.clone()
            }
            None => self.approve(req).await.into(),
        }
    }
}

const PATCH: &str = "*** Begin Patch\n*** Update File: a.txt\n-a\n+A\n*** Update File: b.txt\n-b\n+B\n*** Add File: c.txt\n+c\n*** End Patch";

async fn run(
    reviewer: &Reviewer,
    tool: &str,
    args: serde_json::Value,
) -> (tempfile::TempDir, Vec<(String, bool)>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    let mut a = Agent {
        provider: Arc::new(Scripted {
            manifest: Registry::builtin().get("local").unwrap().clone(),
            turns: Mutex::new([call(tool, &args), done()].into()),
        }),
        model: "m".into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::Default, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(dir.path()).unwrap(),
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
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    a.send("go".into(), &tx, reviewer, CancellationToken::new())
        .await
        .unwrap();
    let results = a
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .collect();
    (dir, results)
}

fn read(dir: &tempfile::TempDir, p: &str) -> Option<String> {
    std::fs::read_to_string(dir.path().join(p)).ok()
}

fn review(reply: PermissionReply, d: &[(&str, FileDecision)]) -> Review {
    Review {
        reply,
        files: d
            .iter()
            .map(|(p, d)| ((*p).to_owned(), d.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[tokio::test]
async fn accepted_subset_and_edited_content_are_written_and_reported() {
    let r = Reviewer(
        Some(review(
            PermissionReply::Allow,
            &[
                ("b.txt", FileDecision::Reject),
                ("c.txt", FileDecision::Edit("mine\n".into())),
            ],
        )),
        PermissionReply::Deny,
        Mutex::new(Vec::new()),
    );
    let (dir, results) = run(&r, "apply_patch", json!({"patch": PATCH})).await;
    assert_eq!(read(&dir, "a.txt").as_deref(), Some("A\n"));
    assert_eq!(read(&dir, "b.txt").as_deref(), Some("b\n"));
    assert_eq!(read(&dir, "c.txt").as_deref(), Some("mine\n"));
    let (text, is_error) = &results[0];
    assert!(!is_error);
    assert!(text.contains("Written as you proposed: a.txt"), "{text}");
    assert!(
        text.contains("own edits") && text.contains("c.txt"),
        "{text}"
    );
    assert!(
        text.contains("not written") && text.contains("b.txt"),
        "{text}"
    );

    let req = &r.2.lock().unwrap()[0];
    let paths: Vec<&str> = req.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(req.files[0].before.as_deref(), Some("a\n"));
    // Updating files the model never read is destructive.
    assert_eq!(req.destructive.as_deref(), Some("overwrite-unread"));
}

#[tokio::test]
async fn reject_all_writes_nothing() {
    let all = [
        ("a.txt", FileDecision::Reject),
        ("b.txt", FileDecision::Reject),
        ("c.txt", FileDecision::Reject),
    ];
    let r = Reviewer(
        Some(review(PermissionReply::Allow, &all)),
        PermissionReply::Deny,
        Mutex::new(Vec::new()),
    );
    let (dir, results) = run(&r, "apply_patch", json!({"patch": PATCH})).await;
    assert_eq!(read(&dir, "a.txt").as_deref(), Some("a\n"));
    assert_eq!(read(&dir, "c.txt"), None);
    assert!(
        results[0].1 && results[0].0.contains("rejected every file"),
        "{:?}",
        results[0]
    );
}

#[tokio::test]
async fn plain_yes_and_no_still_work() {
    let yes = Reviewer(None, PermissionReply::Allow, Mutex::new(Vec::new()));
    let (dir, results) = run(&yes, "apply_patch", json!({"patch": PATCH})).await;
    assert!(!results[0].1, "{:?}", results[0]);
    assert_eq!(read(&dir, "b.txt").as_deref(), Some("B\n"));
    assert_eq!(read(&dir, "c.txt").as_deref(), Some("c\n"));

    let no = Reviewer(None, PermissionReply::Deny, Mutex::new(Vec::new()));
    let (dir, results) = run(&no, "Write", json!({"file_path": "n.txt", "content": "x"})).await;
    assert!(results[0].1);
    assert_eq!(read(&dir, "n.txt"), None);
    let req = &no.2.lock().unwrap()[0];
    assert_eq!(req.files[0].after.as_deref(), Some("x"));
    assert_eq!(req.destructive, None);
}
