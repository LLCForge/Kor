//! `Agent` tool: a nested Kor loop for the user-visible subagents.
//!
//! Built-ins: `general-purpose`, `Explore`, `Plan`. Markdown agents from
//! `.claude/agents` are passed in. `Agent` itself is never given to a child.

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use kor_compat::AgentDef;
use kor_core::{ContentPart, Role, StreamEvent, ToolSpec};
use kor_providers::Provider;
use kor_secrets::Redactor;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};

use crate::permissions::Permissions;
use crate::{Agent, Approver, PermissionReply, PermissionRequest};

const EXPLORE_DENY: &[&str] = &[
    "Agent",
    "Edit",
    "MultiEdit",
    "Delete",
    "Move",
    "remember",
    "forget",
    "browser_click",
    "browser_fill",
    "browser_press",
    "browser_select",
    "ssh_exec",
    "ssh_write_file",
    "ssh_upload",
    "ssh_download",
    "Write",
    "NotebookEdit",
    "TaskCreate",
    "TaskUpdate",
    "TeamCreate",
    "TeamDelete",
    "SendMessage",
];

struct DenyAll;

#[async_trait]
impl Approver for DenyAll {
    async fn approve(&self, _: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

/// Characters of each agent description in the tool spec; enough to tell agents apart.
const SPEC_DESCRIPTION_CHARS: usize = 160;

pub struct AgentTool {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub permissions: Permissions,
    pub agents: Vec<AgentDef>,
    pub explore_plan: bool,
}

impl std::fmt::Debug for AgentTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTool")
            .field("model", &self.model)
            .field("agents", &self.agents.len())
            .finish_non_exhaustive()
    }
}

impl AgentTool {
    fn child_tools(&self, kind: &str) -> Vec<Arc<dyn Tool>> {
        let custom = self.agents.iter().find(|a| a.name == kind);
        self.tools
            .iter()
            .filter(|t| t.name() != "Agent")
            .filter(|t| match kind {
                "Explore" | "Plan" => !EXPLORE_DENY.contains(&t.name()),
                _ => custom
                    .is_none_or(|a| a.tools.is_empty() || a.tools.iter().any(|n| n == t.name())),
            })
            .cloned()
            .collect()
    }

    fn system(&self, kind: &str) -> String {
        if let Some(a) = self.agents.iter().find(|a| a.name == kind) {
            return a.prompt.clone();
        }
        match kind {
            "Explore" => "You are a read-only file search agent. Use Read, Grep, and Glob. \
                          Do not create or modify files. Report findings as text."
                .into(),
            "Plan" => "You are a planning agent. Read the project and return a plan as text. \
                       Do not edit files."
                .into(),
            _ => {
                "You are a general-purpose coding subagent. Finish the task and report the result."
                    .into()
            }
        }
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &'static str {
        "Agent"
    }
    fn rule_name(&self) -> &'static str {
        "Agent"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        // Like Claude Code's Task tool: each type with what it is for, so the model can choose.
        let mut description = String::from(
            "Run a subagent (its own context and tools) and return its final text. Delegate focused              work to a matching specialist; launch independent subagents in the same response to run              them in parallel. subagent_type is one of:
             - general-purpose: any multi-step task.
",
        );
        if self.explore_plan {
            description.push_str(
                "- Explore: read-only search across the codebase; reports findings.
                 - Plan: reads the project and returns an implementation plan without editing.
",
            );
        }
        for a in &self.agents {
            let short: String = a.description.chars().take(SPEC_DESCRIPTION_CHARS).collect();
            let more = if a.description.chars().count() > SPEC_DESCRIPTION_CHARS {
                "…"
            } else {
                ""
            };
            let _ = writeln!(description, "- {}: {short}{more}", a.name);
        }
        ToolSpec {
            name: self.name().into(),
            description,
            input_schema: json!({"type":"object","properties":{
                "description":{"type":"string"},
                "prompt":{"type":"string"},
                "subagent_type":{"type":"string"}
            },"required":["prompt"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["subagent_type"].as_str().map(str::to_owned)
    }
    fn summary(&self, input: &Value) -> String {
        input["description"]
            .as_str()
            .or_else(|| input["prompt"].as_str())
            .unwrap_or("subagent")
            .chars()
            .take(80)
            .collect()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let prompt = match input["prompt"].as_str() {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return ToolOutput::err("prompt is required"),
        };
        let kind = input["subagent_type"].as_str().unwrap_or("general-purpose");
        if (kind == "Explore" || kind == "Plan") && !self.explore_plan {
            return ToolOutput::err(format!("{kind} is disabled by the local explore/plan flag"));
        }
        if kind != "general-purpose"
            && kind != "Explore"
            && kind != "Plan"
            && !self.agents.iter().any(|a| a.name == kind)
        {
            return ToolOutput::err(format!("unknown subagent `{kind}`"));
        }
        let mut child = Agent {
            provider: self.provider.clone(),
            model: self.model.clone(),
            tools: self.child_tools(kind),
            permissions: self.permissions.clone(),
            ctx: ctx.sibling(),
            system: self.system(kind),
            messages: Vec::new(),
            usage: kor_core::Usage::default(),
            max_turns: 8,
            max_tokens: Some(4_000),
            reasoning: None,
            redactor: Redactor::default(),
            hooks: None,
            rules: Vec::new(),
            auto_compact: false,
            context_window: crate::compact::DEFAULT_CONTEXT_WINDOW,
            memory_extract: false,
            memory_base: None,
            session_notes: None,
            output_budget: None,
            dream: false,
            // The child speaks the same protocol the provider supports; it still falls
            // back to text automatically if native tools are rejected.
            tool_mode: crate::toolcall::ToolMode::for_provider(
                self.provider.manifest().tool_calling,
            ),
            verify: crate::verify::Verify::default(),
            stats: crate::repair::ToolStats::default(),
            context: crate::context::Budget::default(),
            cost: None,
            shadow: None,
            repair: None,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let result = child.send(prompt, &tx, &DenyAll, ctx.cancel.clone()).await;
        drop(tx);
        let mut text = String::new();
        while let Some(ev) = rx.recv().await {
            if let crate::AgentEvent::Text(t) = ev {
                text.push_str(&t);
            }
        }
        if text.is_empty() {
            text = child
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| {
                    m.parts
                        .iter()
                        .filter_map(|p| match p {
                            ContentPart::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<String>()
                })
                .unwrap_or_default();
        }
        match result {
            Ok(_) if !text.is_empty() => ToolOutput::ok(text),
            Ok(_) => ToolOutput::err("subagent returned no text"),
            Err(e) => ToolOutput::err(e.message),
        }
    }
}

/// Collect text from a stream. Used by the page summarizer.
pub async fn collect_text(
    mut stream: kor_providers::EventStream,
) -> Result<String, kor_core::ProviderError> {
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        if let StreamEvent::TextDelta { text: t } = ev? {
            text.push_str(&t);
        }
    }
    Ok(text)
}
