//! `git worktree add` / `remove`. The session cwd follows the worktree.

use std::process::Command;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, lock};

fn git(ctx: &ToolCtx, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(&ctx.root)
        .output()
        .map_err(|e| e.to_string())?;
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

#[derive(Debug)]
pub(crate) struct EnterWorktree;

#[derive(Debug)]
pub(crate) struct ExitWorktree;

#[async_trait]
impl Tool for EnterWorktree {
    fn name(&self) -> &'static str {
        "EnterWorktree"
    }
    fn rule_name(&self) -> &'static str {
        "EnterWorktree"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "EnterWorktree".into(),
            description: "Create a git worktree and switch the session cwd to it.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["path"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let path = input["path"].as_str().unwrap_or("");
        if path.is_empty() || path.contains("..") {
            return ToolOutput::err("path is required");
        }
        if let Err(e) = git(ctx, &["worktree", "add", path, "HEAD"]) {
            return ToolOutput::err(e);
        }
        let full = ctx.root.join(path);
        *lock(&ctx.cwd) = crate::without_verbatim(&full);
        ToolOutput::ok(format!("worktree {}", full.display()))
    }
}

#[async_trait]
impl Tool for ExitWorktree {
    fn name(&self) -> &'static str {
        "ExitWorktree"
    }
    fn rule_name(&self) -> &'static str {
        "ExitWorktree"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ExitWorktree".into(),
            description: "Remove a git worktree and restore the session cwd.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["path"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let path = input["path"].as_str().unwrap_or("");
        lock(&ctx.cwd).clone_from(&ctx.root);
        match git(ctx, &["worktree", "remove", "--force", path]) {
            Ok(_) => ToolOutput::ok(format!("removed {path}")),
            Err(e) => ToolOutput::err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_and_remove_switches_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap();
            assert!(st.success(), "{args:?}");
        };
        run(&["init"]);
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        run(&["add", "a.txt"]);
        run(&["commit", "-m", "init"]);
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let entered = EnterWorktree.call(json!({"path": "wt"}), &ctx).await;
        assert!(!entered.is_error, "{}", entered.content);
        assert!(
            lock(&ctx.cwd).ends_with("wt"),
            "{}",
            lock(&ctx.cwd).display()
        );
        assert!(dir.path().join("wt").join("a.txt").is_file());
        let left = ExitWorktree.call(json!({"path": "wt"}), &ctx).await;
        assert!(!left.is_error, "{}", left.content);
        assert_eq!(*lock(&ctx.cwd), ctx.root);
    }
}
