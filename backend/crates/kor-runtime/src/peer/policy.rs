//! The permission matrix of a delegated task (ADR 0057): what a peer may do on this machine.
//! Enforced on the machine that runs the task, around every tool, whatever the permission mode:
//!
//! | Tool kind | Allowed when |
//! |---|---|
//! | read (inside the task root) | always |
//! | network (`WebFetch`, `mcp__…`, browser, ssh, db) | the grant lists the tool or `network` |
//! | edit | the root is the task's own snapshot workspace, or the grant has `write` |
//! | command | its command line starts with a granted prefix (`*` = any) |
//! | anything else asking to act (computer use, elevation) | the grant lists the tool |
//!
//! The default grant is empty: read-only. Every refusal is audited (`peer.denied`).

use std::sync::Arc;

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_tools::{Proposal, Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::Value;

use super::Grant;

/// The rules for one task.
#[derive(Debug, Clone)]
pub struct Policy {
    pub peer: String,
    pub task: String,
    pub grant: Grant,
    /// The task works in its own snapshot copy (writes there touch nothing of this machine).
    pub workspace: bool,
}

fn command_allowed(granted: &[String], command: &str) -> bool {
    let c = command.trim();
    granted.iter().any(|g| {
        let g = g.trim();
        g == "*"
            || c == g
            || c.strip_prefix(g)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

impl Policy {
    /// `Err(reason)` when the peer may not make this call.
    pub fn check(&self, tool: &dyn Tool, input: &Value) -> Result<(), String> {
        let name = tool.name();
        let listed = |n: &str| self.grant.tools.iter().any(|t| t == n);
        match kor_agent::audit::tool_kind(name) {
            "network" => {
                if listed(name) || listed("network") {
                    return Ok(());
                }
                return Err(format!(
                    "the peer policy of this machine does not allow {name} for {}",
                    self.peer
                ));
            }
            "read" => return Ok(()),
            _ => {}
        }
        match tool.kind() {
            ToolKind::Read => Ok(()),
            ToolKind::Edit if self.workspace || self.grant.write => Ok(()),
            ToolKind::Edit => Err(format!(
                "{} has read-only access to this directory (peer policy)",
                self.peer
            )),
            ToolKind::Execute => {
                if listed(name) {
                    return Ok(());
                }
                match tool.target(input) {
                    Some(cmd)
                        if matches!(name, "Bash" | "PowerShell")
                            && command_allowed(&self.grant.commands, &cmd) =>
                    {
                        Ok(())
                    }
                    Some(cmd) if matches!(name, "Bash" | "PowerShell") => Err(format!(
                        "the peer policy does not allow {} to run `{}` here (allowed: {})",
                        self.peer,
                        kor_agent::audit::clip(&cmd),
                        if self.grant.commands.is_empty() {
                            "none".to_owned()
                        } else {
                            self.grant.commands.join(", ")
                        }
                    )),
                    _ => Err(format!(
                        "the peer policy does not allow {name} for {}",
                        self.peer
                    )),
                }
            }
        }
    }
}

/// A tool behind the policy.
pub struct Guarded {
    inner: Arc<dyn Tool>,
    policy: Arc<Policy>,
}

impl std::fmt::Debug for Guarded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guarded")
            .field("tool", &self.inner.name())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Tool for Guarded {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn rule_name(&self) -> &'static str {
        self.inner.rule_name()
    }
    fn kind(&self) -> ToolKind {
        self.inner.kind()
    }
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn target(&self, input: &Value) -> Option<String> {
        self.inner.target(input)
    }
    fn summary(&self, input: &Value) -> String {
        self.inner.summary(input)
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        self.inner.touches(input)
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<Proposal>> {
        self.inner.propose(input, ctx).await
    }
    async fn param_card(&self, input: &Value, ctx: &ToolCtx) -> Option<kor_tools::ParamCard> {
        self.inner.param_card(input, ctx).await
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        if let Err(reason) = self.policy.check(&*self.inner, &input) {
            super::audit(
                "denied",
                &self.policy.peer,
                serde_json::json!({
                    "task": self.policy.task,
                    "tool": self.inner.name(),
                    "target": self.inner.target(&input).map(|t| kor_agent::audit::clip(&t)),
                }),
            );
            return ToolOutput::err(reason);
        }
        self.inner.call(input, ctx).await
    }
}

/// Every tool of a session behind `policy`.
pub fn guard(tools: Vec<Arc<dyn Tool>>, policy: &Arc<Policy>) -> Vec<Arc<dyn Tool>> {
    tools
        .into_iter()
        .map(|inner| {
            Arc::new(Guarded {
                inner,
                policy: policy.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> Arc<dyn Tool> {
        kor_tools::default_tools()
            .into_iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
    }

    fn policy(grant: Grant, workspace: bool) -> Policy {
        Policy {
            peer: "laptop".into(),
            task: "r-1".into(),
            grant,
            workspace,
        }
    }

    #[test]
    fn default_grant_is_read_only() {
        let p = policy(Grant::default(), false);
        let read = tool("Read");
        let write = tool("Write");
        let bash = tool("Bash");
        assert!(p.check(&*read, &json!({"file_path": "a"})).is_ok());
        assert!(p.check(&*write, &json!({"file_path": "a", "content": ""})).is_err());
        assert!(p.check(&*bash, &json!({"command": "ls"})).is_err());
        assert!(p.check(&*tool("WebFetch"), &json!({"url": "https://x"})).is_err());
        // The task's own snapshot copy is writable, commands still need a grant.
        let w = policy(Grant::default(), true);
        assert!(w.check(&*write, &json!({"file_path": "a", "content": ""})).is_ok());
        assert!(w.check(&*bash, &json!({"command": "ls"})).is_err());
    }

    #[test]
    fn granted_commands_match_whole_words() {
        let p = policy(
            Grant {
                commands: vec!["cargo test".into(), "npm run".into()],
                tools: vec!["network".into()],
                ..Grant::default()
            },
            false,
        );
        let bash = tool("Bash");
        assert!(p.check(&*bash, &json!({"command": "cargo test -p x"})).is_ok());
        assert!(p.check(&*bash, &json!({"command": "cargo test"})).is_ok());
        assert!(p.check(&*bash, &json!({"command": "cargo testfoo"})).is_err());
        assert!(p.check(&*bash, &json!({"command": "rm -rf /"})).is_err());
        assert!(p.check(&*tool("WebFetch"), &json!({"url": "https://x"})).is_ok());
        assert!(command_allowed(&["*".into()], "anything"));
    }

    #[tokio::test]
    async fn guarded_tools_refuse_before_running() {
        let d = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(d.path()).unwrap();
        let p = Arc::new(policy(Grant::default(), false));
        let tools = guard(vec![tool("Write")], &p);
        let out = tools[0]
            .call(json!({"file_path": "x.txt", "content": "hi"}), &ctx)
            .await;
        assert!(out.is_error && out.content.contains("read-only"), "{}", out.content);
        assert!(!d.path().join("x.txt").exists());
        assert_eq!(tools[0].name(), "Write");
    }
}
