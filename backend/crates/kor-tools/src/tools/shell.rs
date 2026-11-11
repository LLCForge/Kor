//! `bash`: run a shell command in the project root (named `bash` on every OS; on
//! Windows it runs `PowerShell` and the tool description says so).

use std::time::Duration;

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_sandbox::SandboxKind;
use serde::Deserialize;
use serde_json::{Value, json};

use tokio_util::sync::CancellationToken;

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, lock, parse, truncate};

pub(super) const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub(super) const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_OUTPUT: usize = 30_000;

/// The shell commands run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub name: &'static str,
    pub program: String,
    pub prefix: Vec<String>,
}

fn on_path(name: &str) -> bool {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd"]
    } else {
        &[""]
    };
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()))
    })
}

impl Shell {
    /// The shell inside a Docker sandbox image.
    pub fn container() -> Self {
        Self {
            name: "sh (Docker sandbox)",
            program: "sh".into(),
            prefix: vec!["-c".into()],
        }
    }

    pub fn detect() -> Self {
        if cfg!(windows) {
            // Git Bash first: models write bash (grep, sed, head), and it starts about 4× faster
            // than PowerShell (≈120 ms against 360–470 ms). PowerShell has its own tool.
            if let Some(bash) = kor_platform::os_integration::git_bash() {
                return Self {
                    name: "bash (Git for Windows)",
                    program: bash.to_string_lossy().into_owned(),
                    prefix: vec!["-c".into()],
                };
            }
            Self::powershell()
        } else if on_path("bash") {
            Self {
                name: "bash",
                program: "bash".into(),
                prefix: vec!["-c".into()],
            }
        } else {
            Self {
                name: "sh",
                program: "sh".into(),
                prefix: vec!["-c".into()],
            }
        }
    }

    /// `PowerShell` 7 when installed, else Windows `PowerShell`.
    pub fn powershell() -> Self {
        let ps = ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
            .map(String::from)
            .to_vec();
        {
            if on_path("pwsh") {
                Self {
                    name: "PowerShell 7 (pwsh)",
                    program: "pwsh".into(),
                    prefix: ps,
                }
            } else {
                Self {
                    name: "Windows PowerShell",
                    program: "powershell".into(),
                    prefix: ps,
                }
            }
        }
    }

    /// Git Bash (MSYS) cannot start inside an `AppContainer` (DLL init fails).
    fn is_git_bash(&self) -> bool {
        self.name.starts_with("bash (Git")
    }
}

#[derive(Debug)]
pub(crate) struct Bash {
    shell: Shell,
}

impl Bash {
    pub(crate) fn new(shell: Shell) -> Self {
        Self { shell }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BashIn {
    command: String,
    /// Milliseconds. The tool's `timeout`.
    timeout: Option<u64>,
    /// Older Kor field, in seconds.
    timeout_secs: Option<u64>,
    description: Option<String>,
    #[serde(default)]
    run_in_background: bool,
}

fn env_ms(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
}

fn timeout_ms(input: &BashIn) -> Result<u64, String> {
    let default = env_ms("BASH_DEFAULT_TIMEOUT_MS").unwrap_or(DEFAULT_TIMEOUT_MS);
    let max = env_ms("BASH_MAX_TIMEOUT_MS")
        .unwrap_or(MAX_TIMEOUT_MS)
        .max(default);
    let ms = if let Some(ms) = input.timeout {
        ms
    } else if let Some(secs) = input.timeout_secs {
        secs.saturating_mul(1000)
    } else {
        default
    };
    if ms == 0 || ms > max {
        return Err(format!("timeout must be between 1 and {max} milliseconds"));
    }
    Ok(ms)
}

#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &'static str {
        "Bash"
    }
    fn rule_name(&self) -> &'static str {
        "Bash"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: format!(
                "Run a command with {} in the project root and return stdout, stderr and the exit code. \
                 Use it for builds, tests and git. To read, find or search files use Read, Glob and Grep \
                 instead: they answer at once, need no approval and use no shell. `timeout` is milliseconds (default {DEFAULT_TIMEOUT_MS}, max {MAX_TIMEOUT_MS}, overridable with BASH_DEFAULT_TIMEOUT_MS and BASH_MAX_TIMEOUT_MS). \
                 `description` is a short active-voice summary. No interactive input is available.",
                self.shell.name
            ),
            input_schema: json!({"type": "object", "properties": {
                "command": {"type": "string"},
                "timeout": {"type": "integer", "description": "Timeout in milliseconds"},
                "description": {"type": "string"},
                "run_in_background": {"type": "boolean", "description": "Run without waiting. Refused when background tasks are disabled."}
            }, "required": ["command"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["command"].as_str().map(str::to_owned)
    }
    fn summary(&self, input: &Value) -> String {
        input["description"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| input["command"].as_str())
            .unwrap_or_default()
            .to_owned()
    }
    #[allow(clippy::too_many_lines)] // one linear spawn path: devenv, WSL, editor terminal, sandbox
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: BashIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.command.trim().is_empty() {
            return ToolOutput::err("command is empty");
        }
        if crate::tools::extra::deny_sed_escape(&a.command) {
            return ToolOutput::err("sed -i cannot edit a path outside the project");
        }
        if a.run_in_background && !ctx.background_tasks {
            return ToolOutput::err(
                "background tasks are disabled (local flag background_tasks is off)",
            );
        }
        let _ = a.description.as_deref();
        let ms = match timeout_ms(&a) {
            Ok(ms) => ms,
            Err(e) => return ToolOutput::err(e),
        };
        let scoped: Vec<(String, String)> = crate::scoped_env_for(&a.command, &ctx.scoped_env)
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        // A Docker sandbox runs a Linux image: its `sh`, whatever the host shell is.
        let shell = if ctx.sandbox == SandboxKind::Docker {
            &Shell::container()
        } else if ctx.sandbox == SandboxKind::AppContainer && self.shell.is_git_bash() {
            &Shell::powershell()
        } else {
            &self.shell
        };
        // §15.7: inside the project's environment (devcontainer, nix, mise, ...), after its
        // pinned versions were checked (ADR 0043).
        let mut env_warning = None;
        let (program, args, kind) = if let Some(env) = &ctx.dev_env {
            env.check(ctx, shell).await;
            match env.gate() {
                Ok(w) => env_warning = w,
                Err(e) => return ToolOutput::err(e),
            }
            let (p, a) = env.command(shell, &ctx.root, &a.command);
            (p, a, env.sandbox_for(ctx.sandbox))
        } else if let Some((p, args)) = kor_platform::wsl::command(&lock(&ctx.cwd), &a.command) {
            // §27.10: a project under `\\wsl$\<distro>\…` runs its commands inside that distro.
            (p, args, ctx.sandbox)
        } else {
            let mut args = shell.prefix.clone();
            args.push(a.command);
            (shell.program.clone(), args, ctx.sandbox)
        };
        // §28.8: an ACP editor with a terminal shows the command live — only when Kor's sandbox
        // does not confine it (the sandbox is the boundary; the editor's terminal is not).
        if !a.run_in_background
            && kind == SandboxKind::None
            && let Some(editor) = ctx.editor.as_ref().filter(|e| e.can_terminal())
        {
            let mut env = ctx.env.clone();
            env.extend(scoped);
            let cwd = lock(&ctx.cwd).clone();
            let run = editor.run(
                &program,
                &args,
                &cwd,
                &env,
                Duration::from_millis(ms),
                &ctx.cancel,
            );
            return editor_output(run.await, env_warning);
        }
        let policy = ctx.sandbox_policy();
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(args).current_dir(lock(&ctx.cwd).clone());
        for v in &ctx.scrub_env {
            cmd.env_remove(v);
        }
        cmd.envs(ctx.env.iter().map(|(k, v)| (k, v)));
        cmd.envs(scoped);
        // Confined by `ctx.sandbox`; dropping the future (timeout, cancel) kills the command.
        let child = match kor_sandbox::spawn(kind, &policy, &cmd) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::err(format!("failed to start {program}: {e}"));
            }
        };
        if a.run_in_background {
            return start_background(child, ctx);
        }
        let out = tokio::select! {
            r = child.wait_with_output_streaming(&crate::live_line) => r,
            () = tokio::time::sleep(Duration::from_millis(ms)) => {
                return ToolOutput::err(format!("command timed out after {ms}ms and was killed"));
            }
            () = ctx.cancel.cancelled() => return ToolOutput::err("command cancelled by the user"),
        };
        let out = match out {
            Ok(o) => o,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.trim().is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str("[stderr]\n");
            text.push_str(&stderr);
        }
        let code = out
            .status
            .code()
            .map_or_else(|| "killed by signal".into(), |c| c.to_string());
        let mut body = format!(
            "{}\n[exit code: {code}]",
            truncate(text.trim_end(), MAX_OUTPUT)
        );
        ctx.note_sandbox_blocks(&policy, &text, &mut body);
        if let Some(w) = env_warning {
            body.push_str("\n\n");
            body.push_str(&w);
        }
        if out.status.success() {
            ToolOutput::ok(body)
        } else {
            ToolOutput::err(body)
        }
    }
}

/// The result of a command the ACP editor ran in its terminal, shaped like a local run.
fn editor_output(
    r: Result<(String, Option<i32>), String>,
    env_warning: Option<String>,
) -> ToolOutput {
    let (text, code) = match r {
        Ok(x) => x,
        Err(e) => return ToolOutput::err(e),
    };
    let code_s = code.map_or_else(|| "killed by signal".into(), |c| c.to_string());
    let mut body = format!(
        "{}
[exit code: {code_s}]",
        truncate(text.trim_end(), MAX_OUTPUT)
    );
    if let Some(w) = env_warning {
        body.push_str(
            "

",
        );
        body.push_str(&w);
    }
    if code == Some(0) {
        ToolOutput::ok(body)
    } else {
        ToolOutput::err(body)
    }
}

fn start_background(child: kor_sandbox::Child, ctx: &ToolCtx) -> ToolOutput {
    let (id, cancel) = {
        let mut jobs = lock(&ctx.jobs);
        let id = format!("bg-{}", jobs.len() + 1);
        let cancel = CancellationToken::new();
        jobs.insert(
            id.clone(),
            crate::Job {
                output: String::new(),
                cancel: cancel.clone(),
                done: false,
            },
        );
        (id, cancel)
    };
    let jobs = std::sync::Arc::clone(&ctx.jobs);
    let id2 = id.clone();
    tokio::spawn(async move {
        let result = tokio::select! {
            r = child.wait_with_output() => match r {
                Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
                Err(e) => e.to_string(),
            },
            () = cancel.cancelled() => "cancelled".into(),
        };
        if let Some(job) = lock(&jobs).get_mut(&id2) {
            job.output = result;
            job.done = true;
        }
    });
    ToolOutput::ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows without Git Bash runs `PowerShell`; tests write commands for the shell in use.
    fn powershell() -> bool {
        Shell::detect().prefix.len() > 1
    }

    fn bash() -> Bash {
        Bash::new(Shell::detect())
    }

    #[tokio::test]
    async fn runs_in_root_reports_exit_code_and_scrubs_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        let mut ctx = ToolCtx::new(dir.path()).unwrap();

        let ok = bash().call(json!({"command": "echo hello"}), &ctx).await;
        assert!(!ok.is_error, "{}", ok.content);
        assert!(ok.content.contains("hello") && ok.content.contains("[exit code: 0]"));

        let ls = if powershell() {
            "Get-ChildItem -Name"
        } else {
            "ls"
        };
        assert!(
            bash()
                .call(json!({"command": ls}), &ctx)
                .await
                .content
                .contains("marker.txt")
        );

        let fail = bash().call(json!({"command": "exit 3"}), &ctx).await;
        assert!(
            fail.is_error && fail.content.contains("[exit code: 3]"),
            "{}",
            fail.content
        );

        // A variable every session has; scrubbing it must hide it from the child.
        let var = if kor_platform::os() == kor_platform::Os::Windows {
            "USERNAME"
        } else {
            "HOME"
        };
        assert!(std::env::var_os(var).is_some());
        ctx.scrub_env = vec![var.into()];
        let probe = if powershell() {
            format!("echo \"v=[$env:{var}]\"")
        } else {
            format!("echo \"v=[${var}]\"")
        };
        let out = bash().call(json!({"command": probe}), &ctx).await;
        assert!(out.content.contains("v=[]"), "{}", out.content);
    }

    #[tokio::test]
    async fn timeout_and_cancel_kill_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let sleep = if powershell() {
            "Start-Sleep -Seconds 20"
        } else {
            "sleep 20"
        };

        let t = bash()
            .call(json!({"command": sleep, "timeout_secs": 1}), &ctx)
            .await;
        assert!(
            t.is_error && t.content.contains("timed out"),
            "{}",
            t.content
        );

        ctx.cancel.cancel();
        let c = bash().call(json!({"command": sleep}), &ctx).await;
        assert!(c.is_error && c.content.contains("cancelled"));
    }

    #[tokio::test]
    async fn sandboxed_command_writes_inside_the_root_only() {
        if kor_sandbox::detect() != kor_sandbox::SandboxKind::AppContainer {
            return; // the Linux/macOS backends are covered in kor-sandbox
        }
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("x.txt");
        let mut ctx = ToolCtx::new(dir.path()).unwrap();
        ctx.sandbox = kor_sandbox::SandboxKind::AppContainer;
        let write = json!({"command": "Set-Content -Path in.txt -Value ok"});
        let ok = bash().call(write, &ctx).await;
        assert!(!ok.is_error, "{}", ok.content);
        assert!(dir.path().join("in.txt").is_file());
        let cmd = format!("Set-Content -LiteralPath '{}' -Value x", target.display());
        let denied = bash().call(json!({"command": cmd}), &ctx).await;
        assert!(denied.is_error && !target.exists(), "{}", denied.content);
        assert!(
            denied.content.contains("[sandbox] blocked by appcontainer"),
            "{}",
            denied.content
        );
        assert_eq!(lock(&ctx.sandbox_denials).len(), 1);
    }

    #[tokio::test]
    async fn background_returns_an_id_and_task_output_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ToolCtx::new(dir.path()).unwrap();
        ctx.background_tasks = true;
        let cmd = if powershell() {
            "Write-Output hi"
        } else {
            "echo hi"
        };
        let started = bash()
            .call(json!({"command": cmd, "run_in_background": true}), &ctx)
            .await;
        assert!(!started.is_error, "{}", started.content);
        let mut saw = String::new();
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let out = crate::tools::extra::TaskOutput
                .call(json!({"task_id": &started.content}), &ctx)
                .await;
            if out.content.contains("hi") {
                saw = out.content;
                break;
            }
        }
        assert!(saw.contains("hi"), "{saw}");
    }
}
