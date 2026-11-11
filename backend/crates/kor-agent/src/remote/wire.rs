//! The event and command schema every Kor front end speaks: the desktop bridge's `kor:event`
//! bodies (`frontend/src/desktop/ipc.ts` `AgentEvent`) and the frames of `kor remote-serve`
//! (spec §15.3, ADR 0044). One mapping, used by both, so a remote session renders like a local one.
//!
//! Frames on the remote channel are newline-delimited JSON objects ([`MAX_FRAME`] bytes at most).
//! Server → client: the event bodies below plus `hello`, `tasks`, `attached`, `confirmed`, each
//! carrying `task` and `seq` when they belong to a task. Client → server: [`Command`].

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AgentEvent, FileDecision, PermissionReply, PermissionRequest, Review};

/// Protocol version of `kor remote-serve`; a client refuses another one.
pub const PROTOCOL: u32 = 1;
/// Longest accepted frame (a confirm with large file previews fits).
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// UI body of an agent event (`type`: `text`, `g3:reasoning`, `tool`, `toolResult`, `notice`,
/// `status`);
/// `None` for events front ends read elsewhere (usage, context, progress).
pub fn event_json(ev: AgentEvent) -> Option<Value> {
    Some(match ev {
        AgentEvent::Text(delta) => json!({ "type": "text", "delta": delta }),
        AgentEvent::Thinking(delta) => json!({ "type": "g3:reasoning", "delta": delta }),
        AgentEvent::ToolStart { id, name, summary } => {
            json!({ "type": "tool", "id": id, "name": name, "summary": summary })
        }
        AgentEvent::ToolEnd { id, output, .. } => {
            json!({ "type": "toolResult", "id": id, "result": output })
        }
        AgentEvent::Notice(text) => json!({ "type": "notice", "text": text }),
        // `agent.status` (§16.2): the desktop `KorStatus` draws the core's verb and clock.
        AgentEvent::Status(s) => json!({
            "type": "status",
            "verb": s.verb,
            "verbIndex": s.verb_index,
            "elapsedMs": s.elapsed_ms,
            "tokens": s.tokens,
        }),
        // §27.16: `estimate` before the first model call (desktop composer, `--json` stream).
        AgentEvent::Estimate(p) => json!({
            "type": "estimate",
            "line": p.line,
            "method": p.method,
            "secs": p.secs,
            "usd": p.usd,
            "files": p.files,
        }),
        AgentEvent::ToolProgress { .. } | AgentEvent::Usage(_) | AgentEvent::Context(_) => {
            return None;
        }
    })
}

/// The permission card (`confirm`) for a request.
pub fn confirm_json(req: &PermissionRequest) -> Value {
    json!({
        "type": "confirm", "id": req.id, "name": req.tool, "summary": req.summary,
        "files": req.files, "destructive": req.destructive, "rules": req.rules,
    })
}

/// `approval`: `yes` | `no` | `always`; `files` (optional): per file `"accept"` | `"reject"` |
/// `{ "edit": content }` (`ipc-review.ts` `FileDecisions`).
///
/// # Errors
/// On an unknown answer or file decision.
pub fn parse_review(approval: &str, files: Option<&Value>) -> Result<Review, String> {
    let reply = match approval {
        "yes" => PermissionReply::Allow,
        "always" => PermissionReply::AllowAlways,
        "no" => PermissionReply::Deny,
        other => return Err(format!("unknown approval: {other}")),
    };
    let mut review = Review::from(reply);
    if let Some(map) = files.and_then(Value::as_object) {
        for (path, d) in map {
            let decision = match (d.as_str(), d.get("edit").and_then(Value::as_str)) {
                (Some("accept"), _) => FileDecision::Accept,
                (Some("reject"), _) => FileDecision::Reject,
                (_, Some(content)) => FileDecision::Edit(content.to_owned()),
                _ => return Err(format!("invalid file decision: {path}")),
            };
            review.files.insert(path.clone(), decision);
        }
    }
    Ok(review)
}

/// Client → server frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    /// Start a task, or a follow-up turn of `task`.
    Send {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
    },
    /// Answer a `confirm` of the attached task.
    Approve {
        id: String,
        approval: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<Value>,
    },
    /// Cancel the attached task's running turn.
    Abort,
    /// Follow a task's events from `from` (event sequence number; 0 = replay everything).
    Attach {
        task: String,
        #[serde(default)]
        from: u64,
    },
    /// List the tasks on the server.
    Tasks,
    /// Stop following; the task keeps running.
    Detach,
}

impl Command {
    /// One frame (JSON + `\n`).
    pub fn frame(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// Task ids are session ids too (`kor_runtime::valid_session_id` rules, shorter).
pub fn valid_task_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_with_the_documented_shape() {
        let c = Command::Send {
            text: "hi".into(),
            task: None,
        };
        assert_eq!(c.frame(), "{\"type\":\"send\",\"text\":\"hi\"}\n");
        let a: Command = serde_json::from_str(r#"{"type":"attach","task":"r-1"}"#).expect("attach");
        assert_eq!(
            a,
            Command::Attach {
                task: "r-1".into(),
                from: 0
            }
        );
        assert!(serde_json::from_str::<Command>(r#"{"type":"rm"}"#).is_err());
    }

    #[test]
    fn events_and_reviews_match_the_desktop_contract() {
        let v = event_json(AgentEvent::Text("a".into())).expect("text");
        assert_eq!(v, json!({"type": "text", "delta": "a"}));
        assert!(event_json(AgentEvent::Usage(kor_core::Usage::default())).is_none());
        let s = event_json(AgentEvent::Status(crate::status::status_at(
            0,
            std::time::Duration::from_secs(5),
            42,
        )))
        .expect("status");
        assert_eq!(s["type"], "status");
        assert_eq!(s["elapsedMs"], 5000);
        assert_eq!(s["tokens"], 42);
        assert_eq!(s["verbIndex"], 1);
        let r = parse_review(
            "yes",
            Some(&json!({"a.txt": "reject", "b.txt": {"edit": "x"}})),
        )
        .expect("review");
        assert_eq!(r.reply, PermissionReply::Allow);
        assert_eq!(r.files["a.txt"], FileDecision::Reject);
        assert!(parse_review("maybe", None).is_err());
        assert!(valid_task_id("r-123abc") && !valid_task_id("../x") && !valid_task_id(""));
    }
}
