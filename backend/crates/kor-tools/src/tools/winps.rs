//! Run a fixed helper script with the interpreter each OS ships: Windows `PowerShell` 5.1 (the copy
//! built into Windows, which has the `WinRT` and UI Automation assemblies), `osascript -l
//! JavaScript` on macOS, `python3 -I` on Linux. Inputs travel only as environment variables, never
//! spliced into the script, so screen text or model input cannot become code.

use kor_platform::NoWindow as _;
use std::ffi::OsString;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

/// Program and arguments for a Windows `PowerShell` script.
fn powershell(script: &str) -> (OsString, Vec<OsString>) {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let exe = std::env::var_os("SystemRoot").map_or_else(
        || "powershell.exe".into(),
        |r| {
            std::path::PathBuf::from(r)
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
                .into_os_string()
        },
    );
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ];
    (exe, args.iter().map(OsString::from).collect())
}

/// Program and arguments for a JavaScript for Automation script.
pub(crate) fn osascript(script: &str) -> (OsString, Vec<OsString>) {
    (
        "osascript".into(),
        ["-l", "JavaScript", "-e", script]
            .iter()
            .map(OsString::from)
            .collect(),
    )
}

/// Program and arguments for a Python script (`-I`: no user site, no `PYTHON*` variables).
pub(crate) fn python(script: &str) -> (OsString, Vec<OsString>) {
    (
        "python3".into(),
        ["-I", "-c", script].iter().map(OsString::from).collect(),
    )
}

/// A Windows `PowerShell` script: `stdout` on success; the first meaningful `stderr` line otherwise.
pub(crate) async fn run(
    script: &str,
    env: &[(&str, &str)],
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<String, String> {
    if kor_platform::os() != kor_platform::Os::Windows {
        return Err("this tool uses Windows components and is only available on Windows".into());
    }
    let (exe, args) = powershell(script);
    exec(&exe, &args, env, None, timeout, cancel).await
}

/// Run `program` with `args`, the given environment and optional `stdin`.
pub(crate) async fn exec(
    program: &OsString,
    args: &[OsString],
    env: &[(&str, &str)],
    stdin: Option<&[u8]>,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let name = program.to_string_lossy();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {name}: {e}"))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(data)
            .await
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let out = tokio::select! {
        o = tokio::time::timeout(timeout, child.wait_with_output()) => o,
        () = cancel.cancelled() => return Err("cancelled".into()),
    };
    match out {
        Err(_) => Err("timed out".into()),
        Ok(Err(e)) => Err(format!("{name}: {e}")),
        Ok(Ok(o)) if !o.status.success() => Err(first_error(&String::from_utf8_lossy(&o.stderr))),
        Ok(Ok(o)) => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
    }
}

/// The readable part of a failure: `PowerShell` prints the message first (then position info and
/// `CLIXML` noise); `osascript` wraps it as `execution error: Error: msg (-2700)`.
fn first_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("#<") && !l.starts_with('<'))
        .unwrap_or("the script failed");
    if line.contains("-1719") || line.contains("-25211") || line.contains("assistive access") {
        return "macOS has not given Kor Accessibility access (System Settings > Privacy & Security > Accessibility)".into();
    }
    let line = line
        .split_once("execution error: ")
        .map_or(line, |(_, m)| m);
    let line = line.strip_prefix("Error: ").unwrap_or(line);
    match line.rfind(" (-") {
        Some(i) if line.ends_with(')') => &line[..i],
        _ => line,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_skip_clixml_and_osascript_wrapping() {
        assert_eq!(
            first_error("#< CLIXML\n<Objs Version=\"1\">\nno window 5\n"),
            "no window 5"
        );
        assert_eq!(first_error(""), "the script failed");
        assert_eq!(
            first_error("-e: execution error: Error: No app is in front. (-2700)\n"),
            "No app is in front."
        );
        assert!(
            first_error("execution error: System Events got an error: osascript is not allowed assistive access. (-1719)")
                .contains("Accessibility access")
        );
    }

    #[test]
    fn interpreters_get_the_script_as_one_argument() {
        let (p, a) = osascript("run()");
        assert_eq!(p, "osascript");
        assert_eq!(a, ["-l", "JavaScript", "-e", "run()"]);
        let (p, a) = python("print(1)");
        assert_eq!(p, "python3");
        assert_eq!(a, ["-I", "-c", "print(1)"]);
        let (_, a) = powershell("'x'");
        assert_eq!(a[a.len() - 2], "-EncodedCommand");
    }

    #[tokio::test]
    async fn exec_passes_env_and_stdin_and_reports_missing_programs() {
        let cancel = CancellationToken::new();
        let t = Duration::from_secs(20);
        let missing = exec(&"kor-no-such-program".into(), &[], &[], None, t, &cancel).await;
        assert!(
            missing
                .unwrap_err()
                .starts_with("could not start kor-no-such-program")
        );
        // cargo is on PATH wherever tests run.
        let out = exec(
            &kor_platform::exe("cargo").into(),
            &["--version".into()],
            &[("KOR_TEST", "1")],
            Some(b""),
            t,
            &cancel,
        )
        .await
        .unwrap();
        assert!(out.starts_with("cargo "), "{out}");
    }
}
