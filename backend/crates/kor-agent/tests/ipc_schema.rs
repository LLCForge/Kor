//! §16.3 (ADR 0061): what the Rust core actually puts on the wire validates against
//! `kor-core/schema/ipc.json` — the same schema the desktop's TypeScript types are generated from.

use std::time::Duration;

use kor_agent::remote::wire::{confirm_json, event_json};
use kor_agent::{AgentEvent, PermissionRequest, Proposal};
use kor_core::ipc::{check_def, check_event};
use serde_json::{Value, json};

fn with_session(mut v: Value) -> Value {
    v["sessionId"] = json!("s1");
    v
}

fn assert_valid(name: &str, v: &Value) {
    let errors = check_event("kor:event", v);
    assert!(errors.is_empty(), "{name}: {v}\n{errors:#?}");
}

#[test]
fn every_wire_agent_event_matches_the_schema() {
    let events = [
        AgentEvent::Text("Merhaba".into()),
        AgentEvent::Thinking("düşünüyorum".into()),
        AgentEvent::ToolStart {
            id: "t1".into(),
            name: "Write".into(),
            summary: "notes.txt".into(),
        },
        AgentEvent::ToolEnd {
            id: "t1".into(),
            name: "Write".into(),
            output: "ok".into(),
            is_error: false,
        },
        AgentEvent::Notice("bütçe uyarısı".into()),
        AgentEvent::Status(kor_agent::status::status_at(
            2,
            Duration::from_millis(4200),
            77,
        )),
        AgentEvent::Estimate(kor_agent::cost::Preflight {
            line: "≈ 4m · ≈ $0.03 · ~12 files".into(),
            method: "prior".into(),
            secs: 240,
            usd: Some(0.03),
            files: 12,
            ask: None,
            price: None,
        }),
    ];
    for ev in events {
        let name = format!("{ev:?}");
        let v = event_json(ev).expect("wire item");
        assert_valid(&name, &with_session(v));
    }
    // Usage / Context / ToolProgress travel on other channels (cost, `context.last`), not `kor:event`.
    assert!(event_json(AgentEvent::Usage(kor_core::Usage::default())).is_none());
}

#[test]
fn confirm_with_per_file_review_matches_the_schema() {
    let req = PermissionRequest {
        id: "t2".into(),
        tool: "Write".into(),
        summary: "notes.txt".into(),
        files: vec![
            Proposal {
                path: "notes.txt".into(),
                before: Some("eski\n".into()),
                after: Some("yeni\n".into()),
            },
            Proposal {
                path: "gone.txt".into(),
                before: Some("x".into()),
                after: None,
            },
        ],
        destructive: Some("delete-file".into()),
        rules: vec![kor_tools::rules_check::Finding {
            verdict: kor_tools::rules_check::Verdict::Violated,
            rule: "Never use `unwrap()`".into(),
            source: "CLAUDE.md".into(),
            line: 3,
            file: Some("notes.txt".into()),
            file_line: Some(1),
            message: "`unwrap()` on an added line".into(),
            deterministic: true,
        }],
        ..PermissionRequest::default()
    };
    assert_valid("confirm", &with_session(confirm_json(&req)));
    // The review file shape itself.
    let file = serde_json::to_value(&req.files[1]).unwrap();
    assert!(check_def("ReviewFile", &file).is_empty(), "{file}");
}

#[test]
fn agent_status_event_matches_its_desktop_event() {
    let s = kor_agent::status::status_at(0, Duration::from_secs(3), 10);
    let body = with_session(event_json(AgentEvent::Status(s)).unwrap());
    let errors = check_event("kor:agent/status", &body);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn wrong_shapes_are_rejected() {
    let v = with_session(
        json!({ "type": "status", "verb": "x", "verbIndex": "0", "elapsedMs": 1, "tokens": 0 }),
    );
    assert!(!check_event("kor:event", &v).is_empty());
    let v = with_session(
        json!({ "type": "confirm", "id": "1", "name": "Write", "summary": "s", "files": [{ "path": 1 }] }),
    );
    assert!(!check_event("kor:event", &v).is_empty());
}
