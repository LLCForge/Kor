//! Optional session modes, off by default: coordinator (`KOR_COORDINATOR_MODE`) and
//! REPL (`KOR_REPL_MODE`). The Claude Code variables are read as fallbacks.
//!
//! Claude Code's "undercover" mode (instructions to hide AI involvement in public
//! repositories) is intentionally not ported.

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};
use std::sync::Arc;

fn truthy(kor: &str, claude: &str) -> bool {
    matches!(
        crate::env_compat(kor, claude).as_deref(),
        Some("1" | "true")
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModeFlags {
    pub coordinator: bool,
    pub repl: bool,
}

impl ModeFlags {
    pub fn from_env() -> Self {
        Self {
            coordinator: truthy("KOR_COORDINATOR_MODE", "CLAUDE_CODE_COORDINATOR_MODE"),
            repl: truthy("KOR_REPL_MODE", "CLAUDE_REPL_MODE"),
        }
    }
}

/// Empty unless coordinator mode is on. Callers must not apply the handler otherwise.
pub fn coordinator_context(flags: ModeFlags) -> &'static str {
    if flags.coordinator {
        coordinator_prompt()
    } else {
        ""
    }
}

pub fn coordinator_prompt() -> &'static str {
    "You are a **coordinator**. Your job is to direct workers and synthesize results. \
     Use Agent to spawn a worker and SendMessage to continue one."
}

pub struct ReplTool {
    pub tools: Vec<Arc<dyn Tool>>,
}

impl std::fmt::Debug for ReplTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplTool")
            .field("tools", &self.tools.len())
            .finish()
    }
}

#[async_trait]
impl Tool for ReplTool {
    fn name(&self) -> &'static str {
        "REPL"
    }
    fn rule_name(&self) -> &'static str {
        "REPL"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "REPL".into(),
            description: "Run one primitive tool (Read, Grep, Glob, Bash, Edit, Write) from inside REPL mode.".into(),
            input_schema: json!({"type":"object","properties":{
                "tool":{"type":"string"},
                "input":{"type":"object"}
            },"required":["tool","input"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["tool"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(name) = input["tool"].as_str() else {
            return ToolOutput::err("tool is required");
        };
        let Some(tool) = self.tools.iter().find(|t| t.name() == name) else {
            return ToolOutput::err(format!("REPL cannot run `{name}`"));
        };
        tool.call(input["input"].clone(), ctx).await
    }
}

/// Tools and prompt extras for the flags that are actually on.
pub fn extras(flags: ModeFlags, base: &[Arc<dyn Tool>]) -> (Vec<Arc<dyn Tool>>, String) {
    let mut tools = Vec::new();
    let mut prompt = String::new();
    if flags.coordinator {
        prompt.push_str(coordinator_prompt());
        prompt.push('\n');
    }
    if flags.repl {
        tools.push(Arc::new(ReplTool {
            tools: base.to_vec(),
        }) as Arc<dyn Tool>);
    }
    (tools, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_off_adds_nothing_and_flag_on_adds_repl() {
        let base = kor_tools::default_tools();
        let (tools, prompt) = extras(ModeFlags::default(), &base);
        assert!(tools.is_empty());
        assert!(prompt.is_empty());
        assert!(coordinator_context(ModeFlags::default()).is_empty());
        let (tools, prompt) = extras(
            ModeFlags {
                coordinator: true,
                repl: true,
            },
            &base,
        );
        assert!(tools.iter().any(|t| t.name() == "REPL"));
        assert!(prompt.contains("coordinator"));
    }

    #[tokio::test]
    async fn repl_runs_a_primitive_only_when_built() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let base = kor_tools::default_tools();
        let repl = ReplTool { tools: base };
        let out = repl
            .call(json!({"tool":"Read","input":{"file_path":"a.txt"}}), &ctx)
            .await;
        assert!(out.content.contains("hello"), "{}", out.content);
    }
}
