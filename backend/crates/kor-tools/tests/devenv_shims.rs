//! §15.7 end to end through the `Bash` tool with fake tool shims on `PATH`: a pinned version that
//! does not match stops the command (or warns), a matching one runs it (ADR 0043).

use std::path::Path;
use std::sync::Arc;

use kor_platform::Os;
use kor_tools::devenv::{self, OnMismatch, Session};
use kor_tools::{Tool, ToolCtx, default_tools};
use serde_json::json;

/// A `node` on `dir` that prints `version`.
fn shim(dir: &Path, version: &str) {
    if kor_platform::os() == Os::Windows {
        std::fs::write(dir.join("node.cmd"), format!("@echo v{version}\r\n")).unwrap();
    } else {
        let p = dir.join("node");
        std::fs::write(&p, format!("#!/bin/sh\necho v{version}\n")).unwrap();
        assert!(
            std::process::Command::new("chmod")
                .arg("+x")
                .arg(&p)
                .status()
                .unwrap()
                .success()
        );
    }
}

fn bash() -> Arc<dyn Tool> {
    default_tools()
        .into_iter()
        .find(|t| t.name() == "Bash")
        .unwrap()
}

async fn run(node: &str, mode: OnMismatch) -> kor_tools::ToolOutput {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join(".nvmrc"), "20\n").unwrap();
    let shims = tempfile::tempdir().unwrap();
    shim(shims.path(), node);
    let mut ctx = ToolCtx::new(project.path()).unwrap();
    let path = std::env::join_paths(std::iter::once(shims.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    ctx.env
        .push(("PATH".into(), path.to_string_lossy().into_owned()));
    let det = devenv::detect(&ctx.root, "");
    assert_eq!(det.pins.len(), 1);
    ctx.dev_env = Session::new(det, mode, ctx.sandbox, &devenv::on_path).map(Arc::new);
    bash().call(json!({"command": "echo kor-ran"}), &ctx).await
}

#[tokio::test]
async fn wrong_pinned_version_stops_or_warns_and_a_match_runs() {
    let out = run("18.0.0", OnMismatch::Stop).await;
    assert!(out.is_error, "{}", out.content);
    assert!(
        out.content
            .contains("node: expected 20 (.nvmrc), found 18.0.0")
            && out.content.contains("nvm install 20")
            && !out.content.contains("kor-ran"),
        "{}",
        out.content
    );

    let out = run("18.0.0", OnMismatch::Warn).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("kor-ran") && out.content.contains("[env] warning"));

    let out = run("20.11.1", OnMismatch::Stop).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("kor-ran") && !out.content.contains("[env]"));
}
