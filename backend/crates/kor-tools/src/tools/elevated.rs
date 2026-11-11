//! `RunElevated`: run one command with administrator/root rights, using the operating
//! system's own consent prompt each time.
//!
//! - Windows: `Start-Process -Verb RunAs` (UAC dialog). Output is captured through a
//!   temporary file because an elevated process cannot share the caller's pipes.
//! - macOS: `osascript … with administrator privileges` (system password dialog).
//! - Linux: `pkexec` (polkit dialog) in a desktop session, otherwise `sudo -n`, which
//!   only works with cached credentials or a NOPASSWD rule.
//!
//! Kor itself never runs elevated by default: an agent that is always root turns any
//! prompt injection in a file or web page into full control of the machine.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse, truncate};

const DEFAULT_TIMEOUT: u64 = 300;
const MAX_OUTPUT: usize = 30_000;
/// Windows `ERROR_CANCELLED`: the user declined the UAC prompt.
const CANCELLED: i32 = 1223;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub program: String,
    pub args: Vec<String>,
    /// Windows: the elevated process writes its output here.
    pub output_file: Option<std::path::PathBuf>,
}

/// `PowerShell` `-EncodedCommand`: base64 of UTF-16LE, so no quoting can break out.
fn encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// How to run `command` elevated on `os`.
pub fn plan(
    os: &str,
    command: &str,
    cwd: &std::path::Path,
    out: &std::path::Path,
    linux: (bool, bool),
) -> Result<Plan, String> {
    match os {
        "windows" => {
            let q = |s: &str| s.replace('\'', "''");
            let inner = format!(
                "Set-Location -LiteralPath '{}'; & {{ {command} }} *> '{}'; exit $LASTEXITCODE",
                q(&crate::display_path(cwd)),
                q(&out.display().to_string())
            );
            let shell = if on_path("pwsh.exe") {
                "pwsh"
            } else {
                "powershell"
            };
            let outer = format!(
                "try {{ $p = Start-Process -FilePath '{shell}' -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{}' \
                 -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode }} \
                 catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit {CANCELLED} }}",
                encoded(&inner)
            );
            Ok(Plan {
                program: "powershell".into(),
                args: [
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &outer,
                ]
                .map(String::from)
                .to_vec(),
                output_file: Some(out.to_owned()),
            })
        }
        "macos" => {
            let script = format!(
                "cd {} && {command}",
                shell_quote(&cwd.display().to_string())
            );
            let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
            Ok(Plan {
                program: "osascript".into(),
                args: vec![
                    "-e".into(),
                    format!("do shell script \"{escaped}\" with administrator privileges"),
                ],
                output_file: None,
            })
        }
        "linux" => {
            let (pkexec, desktop) = linux;
            let script = format!(
                "cd {} && {command}",
                shell_quote(&cwd.display().to_string())
            );
            if pkexec && desktop {
                Ok(Plan {
                    program: "pkexec".into(),
                    args: vec!["sh".into(), "-c".into(), script],
                    output_file: None,
                })
            } else {
                Ok(Plan {
                    program: "sudo".into(),
                    args: vec!["-n".into(), "sh".into(), "-c".into(), script],
                    output_file: None,
                })
            }
        }
        other => Err(format!("elevation is not supported on {other}")),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Windows `PowerShell` 5 writes UTF-16LE with a BOM; `PowerShell` 7 writes UTF-8.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    command: String,
    timeout_secs: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct RunElevated;

#[async_trait]
impl Tool for RunElevated {
    fn name(&self) -> &'static str {
        "RunElevated"
    }
    fn rule_name(&self) -> &'static str {
        "RunElevated"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Run one shell command with administrator/root rights (installing system packages, \
                 services, drivers, protected files). The user approves it in Kor and in the OS dialog \
                 (UAC / password / polkit). Use Bash for everything that does not need elevation."
                .into(),
            input_schema: json!({"type": "object", "properties": {
                "command": {"type": "string"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 3600}
            }, "required": ["command"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["command"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.command.trim().is_empty() {
            return ToolOutput::err("command is empty");
        }
        if cfg!(test) {
            return ToolOutput::err("elevation is not run in tests");
        }
        let os = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let out_file =
            std::env::temp_dir().join(format!("kor-elevated-{}-{nonce}.log", std::process::id()));
        let desktop =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        let p = match plan(
            os,
            &a.command,
            &ctx.root,
            &out_file,
            (on_path("pkexec"), desktop),
        ) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let mut cmd = tokio::process::Command::new(&p.program);
        cmd.args(&p.args)
            .current_dir(&ctx.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for v in &ctx.scrub_env {
            cmd.env_remove(v);
        }
        let secs = a.timeout_secs.unwrap_or(DEFAULT_TIMEOUT).clamp(1, 3600);
        let result = tokio::select! {
            r = cmd.output() => r,
            () = tokio::time::sleep(Duration::from_secs(secs)) => {
                return ToolOutput::err(format!("elevated command timed out after {secs}s"));
            }
            () = ctx.cancel.cancelled() => return ToolOutput::err("cancelled by the user"),
        };
        let o = match result {
            Ok(o) => o,
            Err(e) => return ToolOutput::err(format!("could not start {}: {e}", p.program)),
        };
        let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
        if let Some(f) = &p.output_file {
            if let Ok(bytes) = std::fs::read(f) {
                text.push_str(&decode(&bytes));
            }
            let _ = std::fs::remove_file(f);
        }
        let stderr = String::from_utf8_lossy(&o.stderr);
        if !stderr.trim().is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(&stderr);
        }
        let code = o.status.code();
        let hint = match (os, code) {
            ("windows", Some(CANCELLED)) => "\nThe UAC prompt was declined.",
            ("linux", Some(1)) if p.program == "sudo" && stderr.contains("password") => {
                "\nsudo needs a password and cannot prompt here; run `sudo -v` in a terminal first, or use a desktop session with polkit."
            }
            ("macos", _) if stderr.contains("-128") => "\nThe password dialog was cancelled.",
            _ => "",
        };
        let body = format!(
            "{}\n[exit code: {}]{hint}",
            truncate(text.trim_end(), MAX_OUTPUT),
            code.map_or_else(|| "signal".into(), |c| c.to_string())
        );
        if o.status.success() {
            ToolOutput::ok(body)
        } else {
            ToolOutput::err(body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn windows_plan_uses_runas_and_an_encoded_command() {
        let p = plan(
            "windows",
            "choco install git -y",
            Path::new("C:\\proj"),
            Path::new("C:\\t\\o.log"),
            (false, false),
        )
        .unwrap();
        assert_eq!(p.program, "powershell");
        let outer = p.args.last().unwrap();
        assert!(outer.contains("-Verb RunAs") && outer.contains("-EncodedCommand"));
        assert!(
            !outer.contains("choco"),
            "the command travels encoded, never as raw text"
        );
        let b64 = outer
            .split("'-EncodedCommand','")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let inner = String::from_utf16(&units).unwrap();
        assert!(
            inner.contains("& { choco install git -y } *> 'C:\\t\\o.log'"),
            "{inner}"
        );
        assert_eq!(p.output_file.as_deref(), Some(Path::new("C:\\t\\o.log")));
    }

    #[test]
    fn macos_and_linux_plans() {
        let m = plan(
            "macos",
            "echo \"hi\" \\ x",
            Path::new("/p"),
            Path::new("/t"),
            (false, false),
        )
        .unwrap();
        assert_eq!(m.program, "osascript");
        assert!(m.args[1].ends_with("with administrator privileges"));
        assert!(
            m.args[1].contains("echo \\\"hi\\\" \\\\ x"),
            "{}",
            m.args[1]
        );
        let l = plan(
            "linux",
            "apt-get update",
            Path::new("/p"),
            Path::new("/t"),
            (true, true),
        )
        .unwrap();
        assert_eq!((l.program.as_str(), l.args[0].as_str()), ("pkexec", "sh"));
        let s = plan(
            "linux",
            "apt-get update",
            Path::new("/p q"),
            Path::new("/t"),
            (true, false),
        )
        .unwrap();
        assert_eq!(s.args[..3], ["-n", "sh", "-c"]);
        assert!(s.args[3].starts_with("cd '/p q' && "));
        assert!(
            plan(
                "plan9",
                "x",
                Path::new("/"),
                Path::new("/t"),
                (false, false)
            )
            .is_err()
        );
    }

    #[test]
    fn output_decoding() {
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'o', b'k']), "ok");
        assert_eq!(decode(b"plain"), "plain");
    }
}
