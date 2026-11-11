//! Local swarm mailbox. `TeamCreate`, `TeamDelete`, and `SendMessage` use files
//! under `<project>/.kor/teams`. There is no remote bus.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Team {
    name: String,
    #[serde(default)]
    description: String,
    lead: String,
    members: Vec<String>,
}

fn safe_name(name: &str) -> Result<(), ToolOutput> {
    if name.is_empty() || name.contains(['/', '\\', '.', '\0']) || name.len() > 64 {
        return Err(ToolOutput::err("invalid team or member name"));
    }
    Ok(())
}

fn team_dir(ctx: &ToolCtx, name: &str) -> PathBuf {
    ctx.root.join(".kor").join("teams").join(name)
}

#[derive(Debug)]
pub(crate) struct TeamCreate;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateIn {
    team_name: String,
    #[serde(default)]
    description: String,
}

#[async_trait]
impl Tool for TeamCreate {
    fn name(&self) -> &'static str {
        "TeamCreate"
    }
    fn rule_name(&self) -> &'static str {
        "TeamCreate"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description:
                "Create a local multi-agent team mailbox under .kor/teams. The lead is team-lead."
                    .into(),
            input_schema: json!({"type":"object","properties":{
                "team_name":{"type":"string"},
                "description":{"type":"string"}
            },"required":["team_name"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["team_name"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: CreateIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if let Err(e) = safe_name(&a.team_name) {
            return e;
        }
        let dir = team_dir(ctx, &a.team_name);
        if dir.join("team.json").is_file() {
            return ToolOutput::err("team already exists");
        }
        if let Err(e) = fs::create_dir_all(dir.join("inbox")) {
            return ToolOutput::err(e.to_string());
        }
        let team = Team {
            name: a.team_name.clone(),
            description: a.description,
            lead: "team-lead".into(),
            members: vec!["team-lead".into()],
        };
        if let Err(e) = fs::write(
            dir.join("team.json"),
            serde_json::to_string_pretty(&team).unwrap_or_default(),
        ) {
            return ToolOutput::err(e.to_string());
        }
        ToolOutput::ok(format!("created team {}", a.team_name))
    }
}

#[derive(Debug)]
pub(crate) struct TeamDelete;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameIn {
    team_name: String,
}

#[async_trait]
impl Tool for TeamDelete {
    fn name(&self) -> &'static str {
        "TeamDelete"
    }
    fn rule_name(&self) -> &'static str {
        "TeamDelete"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Delete a local team mailbox.".into(),
            input_schema: json!({"type":"object","properties":{"team_name":{"type":"string"}},"required":["team_name"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["team_name"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: NameIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if let Err(e) = safe_name(&a.team_name) {
            return e;
        }
        let dir = team_dir(ctx, &a.team_name);
        if !dir.is_dir() {
            return ToolOutput::err("no such team");
        }
        fs::remove_dir_all(&dir).map_or_else(
            |e| ToolOutput::err(e.to_string()),
            |()| ToolOutput::ok(format!("deleted team {}", a.team_name)),
        )
    }
}

#[derive(Debug)]
pub(crate) struct SendMessage;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MsgIn {
    team_name: String,
    recipient: String,
    message: String,
    #[serde(default)]
    summary: Option<String>,
}

#[async_trait]
impl Tool for SendMessage {
    fn name(&self) -> &'static str {
        "SendMessage"
    }
    fn rule_name(&self) -> &'static str {
        "SendMessage"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Append a message to a teammate's local inbox, or to every member when recipient is *.".into(),
            input_schema: json!({"type":"object","properties":{
                "team_name":{"type":"string"},
                "recipient":{"type":"string"},
                "message":{"type":"string"},
                "summary":{"type":"string"}
            },"required":["team_name","recipient","message"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["recipient"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: MsgIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if let Err(e) = safe_name(&a.team_name) {
            return e;
        }
        let dir = team_dir(ctx, &a.team_name);
        let file = dir.join("team.json");
        let Ok(text) = fs::read_to_string(&file) else {
            return ToolOutput::err("no such team");
        };
        let mut team: Team = match serde_json::from_str(&text) {
            Ok(t) => t,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let recipients: Vec<String> = if a.recipient == "*" {
            team.members
                .iter()
                .filter(|m| m.as_str() != "team-lead")
                .cloned()
                .collect()
        } else {
            if let Err(e) = safe_name(&a.recipient) {
                return e;
            }
            if !team.members.iter().any(|m| m == &a.recipient) {
                team.members.push(a.recipient.clone());
                if let Err(e) = fs::write(
                    &file,
                    serde_json::to_string_pretty(&team).unwrap_or_default(),
                ) {
                    return ToolOutput::err(e.to_string());
                }
            }
            vec![a.recipient.clone()]
        };
        if recipients.is_empty() {
            return ToolOutput::err("no teammates to message");
        }
        let line = json!({
            "from": "team-lead",
            "summary": a.summary,
            "message": a.message,
        });
        for name in &recipients {
            let inbox = dir.join("inbox").join(format!("{name}.jsonl"));
            if let Some(parent) = inbox.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                return ToolOutput::err(e.to_string());
            }
            let mut f = match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&inbox)
            {
                Ok(f) => f,
                Err(e) => return ToolOutput::err(e.to_string()),
            };
            if writeln!(f, "{line}").is_err() {
                return ToolOutput::err("could not write inbox");
            }
        }
        ToolOutput::ok(format!("sent to {}", recipients.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn two_members_exchange_through_files() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let made = TeamCreate
            .call(json!({"team_name": "alpha", "description": "local"}), &ctx)
            .await;
        assert!(!made.is_error, "{}", made.content);
        let sent = SendMessage
            .call(
                json!({"team_name":"alpha","recipient":"worker","message":"hello worker","summary":"ping"}),
                &ctx,
            )
            .await;
        assert!(!sent.is_error, "{}", sent.content);
        let inbox = dir.path().join(".kor/teams/alpha/inbox/worker.jsonl");
        let text = fs::read_to_string(&inbox).unwrap();
        assert!(text.contains("hello worker"), "{text}");
        let gone = TeamDelete.call(json!({"team_name":"alpha"}), &ctx).await;
        assert!(!gone.is_error, "{}", gone.content);
        assert!(!dir.path().join(".kor/teams/alpha").exists());
    }
}
