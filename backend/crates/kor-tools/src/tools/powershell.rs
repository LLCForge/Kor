//! `PowerShell` tool. Off Windows it returns `PowerShell is not available on this system.`
//! Validation rejects `Invoke-Expression` and its `iex` alias from
//! `dangerousCmdlets.ts` / `parser.ts`.

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput};

pub fn powershell_denied(command: &str) -> bool {
    command
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .any(|token| {
            let t = token.to_ascii_lowercase();
            t == "invoke-expression" || t == "iex"
        })
}

pub const UNAVAILABLE: &str = "PowerShell is not available on this system.";

#[derive(Debug)]
pub(crate) struct PowerShellTool;

#[async_trait]
impl Tool for PowerShellTool {
    fn name(&self) -> &'static str {
        "PowerShell"
    }
    fn rule_name(&self) -> &'static str {
        "PowerShell"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "PowerShell".into(),
            description:
                "Run a Windows PowerShell command. Disabled when PowerShell is not available."
                    .into(),
            input_schema: json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"integer","description":"Timeout in milliseconds (default 120000, max 600000)"}},"required":["command"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["command"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let command = input["command"].as_str().unwrap_or("");
        if command.is_empty() {
            return ToolOutput::err("command is empty");
        }
        if powershell_denied(command) {
            return ToolOutput::err("PowerShell command failed validation");
        }
        if !cfg!(windows) {
            return ToolOutput::err(UNAVAILABLE);
        }
        let mut cmd = tokio::process::Command::new("powershell");
        cmd.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            command,
        ])
        .current_dir(&ctx.root);
        for v in &ctx.scrub_env {
            cmd.env_remove(v);
        }
        let policy = ctx.sandbox_policy();
        // Same confinement, timeout and stop as `Bash`; dropping the future kills the command.
        let ms = input["timeout"]
            .as_u64()
            .unwrap_or(super::shell::DEFAULT_TIMEOUT_MS)
            .clamp(1, super::shell::MAX_TIMEOUT_MS);
        let out = match kor_sandbox::spawn(ctx.sandbox, &policy, &cmd) {
            Ok(child) => tokio::select! {
                r = child.wait_with_output_streaming(&crate::live_line) => r,
                () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {
                    return ToolOutput::err(format!("command timed out after {ms}ms and was killed"));
                }
                () = ctx.cancel.cancelled() => return ToolOutput::err("command cancelled by the user"),
            },
            Err(e) => Err(e),
        };
        match out {
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                let all = format!("{text}{}", String::from_utf8_lossy(&o.stderr));
                ctx.note_sandbox_blocks(&policy, &all, &mut text);
                if o.status.success() {
                    ToolOutput::ok(text)
                } else {
                    ToolOutput::err(text)
                }
            }
            Err(e) => ToolOutput::err(format!("{UNAVAILABLE} {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn timeout_and_stop_kill_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let t = PowerShellTool
            .call(
                json!({"command": "Start-Sleep -Seconds 20", "timeout": 1000}),
                &ctx,
            )
            .await;
        assert!(
            t.is_error && t.content.contains("timed out"),
            "{}",
            t.content
        );
        ctx.cancel.cancel();
        let c = PowerShellTool
            .call(json!({"command": "Start-Sleep -Seconds 20"}), &ctx)
            .await;
        assert!(
            c.is_error && c.content.contains("cancelled"),
            "{}",
            c.content
        );
    }

    #[test]
    fn validator_rejects_invoke_expression() {
        assert!(powershell_denied("Invoke-Expression whoami"));
        assert!(powershell_denied("iex 'Get-Date'"));
        assert!(!powershell_denied("Write-Output hi"));
    }

    #[tokio::test]
    async fn runs_or_reports_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let out = PowerShellTool
            .call(json!({"command": "Write-Output hi"}), &ctx)
            .await;
        if cfg!(windows) {
            assert!(out.content.contains("hi"), "{}", out.content);
        } else {
            assert_eq!(out.content, UNAVAILABLE);
        }
    }
}
