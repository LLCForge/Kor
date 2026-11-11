//! macOS and Linux counterparts of the `desktop.ps1` actions that need no accessibility API:
//! `launch_app`, the clipboard, `list_processes` and `system_info`. Programs are started with an
//! argument list (no shell), so targets and arguments cannot inject commands.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use kor_platform::Os;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::super::winps::exec;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Split a command line into arguments: whitespace separates, single or double quotes group,
/// a backslash escapes the next character outside single quotes.
pub(super) fn split_args(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut any = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => cur.push(c),
            (_, '\\') => cur.push(chars.next().ok_or("args end with a backslash")?),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("args have an unclosed quote".into());
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// The program and arguments that start `target`: on macOS `open` (an app by name with `-a`, a
/// file or folder with its default app); on Linux the program itself, or `xdg-open` for a file,
/// folder or URL.
pub(super) fn launch_command(
    os: Os,
    target: &str,
    args: &[String],
    cwd: &Path,
) -> (OsString, Vec<OsString>) {
    let path = if Path::new(target).is_absolute() {
        Path::new(target).to_owned()
    } else {
        cwd.join(target)
    };
    let is_url = target.contains("://");
    let is_path = !is_url && (target.contains('/') || path.exists());
    let mut v: Vec<OsString> = Vec::new();
    if os == Os::Mac {
        if !is_path && !is_url {
            v.push("-a".into());
        }
        v.push(if is_path { path.into() } else { target.into() });
        if !args.is_empty() {
            v.push("--args".into());
            v.extend(args.iter().map(OsString::from));
        }
        return ("open".into(), v);
    }
    if !is_url && (!is_path || is_executable(&path)) {
        v.extend(args.iter().map(OsString::from));
        (if is_path { path.into() } else { target.into() }, v)
    } else {
        v.push(if is_path { path.into() } else { target.into() });
        ("xdg-open".into(), v)
    }
}

fn is_executable(p: &Path) -> bool {
    use std::io::Read as _;
    // A file Linux can run starts with ELF or a `#!` line; documents open with their app instead.
    let mut head = [0u8; 4];
    p.is_file()
        && std::fs::File::open(p)
            .and_then(|mut f| f.read_exact(&mut head))
            .is_ok()
        && (head == *b"\x7fELF" || head.starts_with(b"#!"))
}

pub(super) fn launch(target: &str, args: &str, cwd: &Path) -> Result<String, String> {
    let list = split_args(args)?;
    let (program, params) = launch_command(kor_platform::os(), target, &list, cwd);
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&params)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.to_string_lossy()))?;
    Ok(child
        .id()
        .map_or_else(|| "started".into(), |pid| format!("started pid {pid}")))
}

/// Clipboard programs to try, in order: `(program, args)` for reading and writing.
pub(super) fn clipboard_commands(
    os: Os,
    write: bool,
    wayland: bool,
) -> Vec<(&'static str, Vec<&'static str>)> {
    match (os, write) {
        (Os::Mac, false) => vec![("pbpaste", vec![])],
        (Os::Mac, true) => vec![("pbcopy", vec![])],
        (_, false) => {
            let mut v = vec![
                ("xclip", vec!["-selection", "clipboard", "-o"]),
                ("xsel", vec!["--clipboard", "--output"]),
            ];
            if wayland {
                v.insert(0, ("wl-paste", vec!["--no-newline"]));
            }
            v
        }
        (_, true) => {
            let mut v = vec![
                ("xclip", vec!["-selection", "clipboard", "-i"]),
                ("xsel", vec!["--clipboard", "--input"]),
            ];
            if wayland {
                v.insert(0, ("wl-copy", vec![]));
            }
            v
        }
    }
}

pub(super) async fn clipboard(
    write: Option<&str>,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut last = String::new();
    for (program, args) in clipboard_commands(kor_platform::os(), write.is_some(), wayland) {
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        match exec(
            &program.into(),
            &args,
            &[],
            write.map(str::as_bytes),
            TIMEOUT,
            cancel,
        )
        .await
        {
            Ok(text) => return Ok(text),
            Err(e) if e.starts_with("could not start") => last = e,
            // wl-paste/xclip fail when the clipboard holds no text.
            Err(_) if write.is_none() => return Ok(String::new()),
            Err(e) => return Err(e),
        }
    }
    Err(format!(
        "no clipboard program on this desktop (wl-clipboard, xclip or xsel); {last}"
    ))
}

/// `ps -A -o pid=,rss=,time=,comm=` → the same JSON rows `desktop.ps1` prints.
pub(super) fn parse_ps(out: &str) -> Vec<Value> {
    let mut rows: Vec<(u64, Value)> = out
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid: u64 = it.next()?.parse().ok()?;
            let rss_kb: u64 = it.next()?.parse().ok()?;
            let cpu = cpu_seconds(it.next()?);
            let comm = it.collect::<Vec<_>>().join(" ");
            // macOS prints the executable's path, Linux the bare name (which may contain `/`).
            let name = if comm.starts_with('/') {
                comm.rsplit('/').next().unwrap_or(&comm).to_owned()
            } else {
                comm
            };
            #[allow(clippy::cast_precision_loss)]
            let mem_mb = (rss_kb as f64 / 1024.0 * 10.0).round() / 10.0;
            Some((
                rss_kb,
                json!({"pid": pid, "name": name, "cpu_s": cpu, "mem_mb": mem_mb, "title": ""}),
            ))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    rows.into_iter().map(|(_, v)| v).collect()
}

/// `[[dd-]hh:]mm:ss[.cc]` → seconds.
fn cpu_seconds(t: &str) -> f64 {
    let (days, rest) = t.split_once('-').unwrap_or(("0", t));
    let secs = rest
        .split(':')
        .fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0));
    let total = days.parse::<f64>().unwrap_or(0.0) * 86_400.0 + secs;
    (total * 10.0).round() / 10.0
}

pub(super) async fn processes(
    limit: usize,
    cancel: &CancellationToken,
) -> Result<Vec<Value>, String> {
    let args: Vec<OsString> = ["-A", "-o", "pid=,rss=,time=,comm="]
        .iter()
        .map(OsString::from)
        .collect();
    let out = exec(&"ps".into(), &args, &[], None, TIMEOUT, cancel).await?;
    Ok(parse_ps(&out).into_iter().take(limit).collect())
}

/// `PRETTY_NAME` from `/etc/os-release`.
pub(super) fn os_release_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

/// `(total, available)` in kB from `/proc/meminfo`.
pub(super) fn meminfo(text: &str) -> Option<(u64, u64)> {
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
    };
    Some((get("MemTotal:")?, get("MemAvailable:")?))
}

/// `sysctl -n kern.boottime` → `{ sec = 1700000000, usec = 0 } …` → boot time in Unix seconds.
pub(super) fn boot_secs(text: &str) -> Option<u64> {
    let after = text.split_once("sec = ")?.1;
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn uptime(secs: u64) -> String {
    format!(
        "{}d {}h {}m",
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60
    )
}

#[allow(clippy::cast_precision_loss)]
fn gb(kb: u64) -> f64 {
    (kb as f64 / 1_048_576.0 * 10.0).round() / 10.0
}

pub(super) async fn system_info(cancel: &CancellationToken) -> Result<String, String> {
    let os = kor_platform::os();
    let run = |p: &'static str, a: &'static [&'static str]| async move {
        let args: Vec<OsString> = a.iter().map(OsString::from).collect();
        exec(&p.into(), &args, &[], None, TIMEOUT, cancel)
            .await
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let arch = std::env::consts::ARCH;
    let threads = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let mut out = String::new();
    let (name, cpu, memory, up) = if os == Os::Mac {
        let name = format!(
            "{} {}",
            run("sw_vers", &["-productName"]).await,
            run("sw_vers", &["-productVersion"]).await
        );
        let cpu = run("sysctl", &["-n", "machdep.cpu.brand_string"]).await;
        let total = run("sysctl", &["-n", "hw.memsize"])
            .await
            .parse::<u64>()
            .unwrap_or(0)
            / 1024;
        let boot = boot_secs(&run("sysctl", &["-n", "kern.boottime"]).await);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        (
            name,
            cpu,
            format!("{} GB total", gb(total)),
            boot.map(|b| uptime(now.saturating_sub(b))),
        )
    } else {
        let name = format!(
            "{} (kernel {})",
            os_release_name(&read("/etc/os-release")).unwrap_or_else(|| "Linux".into()),
            run("uname", &["-r"]).await
        );
        let cpu = read("/proc/cpuinfo")
            .lines()
            .find_map(|l| l.strip_prefix("model name"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
            .unwrap_or_default();
        let memory = meminfo(&read("/proc/meminfo")).map_or_else(String::new, |(t, a)| {
            format!("{} / {} GB used", gb(t.saturating_sub(a)), gb(t))
        });
        let up = read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|s| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let s = s as u64;
                uptime(s)
            });
        (name, cpu, memory, up)
    };
    let host = run("hostname", &[]).await;
    let user = std::env::var("USER").unwrap_or_default();
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let tz = std::env::var("TZ").ok().or_else(|| {
        std::fs::read_link("/etc/localtime").ok().and_then(|p| {
            p.to_string_lossy()
                .split_once("zoneinfo/")
                .map(|(_, z)| z.to_owned())
        })
    });
    let _ = writeln!(out, "OS: {name} ({arch})");
    let _ = writeln!(out, "Computer: {host}  User: {user}");
    let _ = writeln!(out, "CPU: {cpu} ({threads} threads)");
    let _ = writeln!(out, "Memory: {memory}");
    let _ = writeln!(out, "Uptime: {}", up.unwrap_or_default());
    let _ = write!(
        out,
        "Locale: {locale}  Time zone: {}",
        tz.unwrap_or_default()
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_split_like_a_shell_without_running_one() {
        assert_eq!(
            split_args(r#"--new-window "a b" 'c "d"' e\ f"#).unwrap(),
            ["--new-window", "a b", "c \"d\"", "e f"]
        );
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
        assert_eq!(split_args("''").unwrap(), [""]);
        assert!(split_args("\"open").is_err());
        assert!(
            split_args("x; rm -rf /")
                .unwrap()
                .contains(&"rm".to_owned())
        );
    }

    #[test]
    fn launch_builds_open_and_direct_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::write(dir.path().join("run.sh"), "#!/bin/sh\n").unwrap();
        let s = |v: (OsString, Vec<OsString>)| {
            let mut all = vec![v.0.to_string_lossy().into_owned()];
            all.extend(v.1.iter().map(|a| a.to_string_lossy().into_owned()));
            all
        };
        let args = vec!["--private".to_owned()];
        assert_eq!(
            s(launch_command(Os::Mac, "Safari", &args, dir.path())),
            ["open", "-a", "Safari", "--args", "--private"]
        );
        let doc = s(launch_command(Os::Mac, "notes.txt", &[], dir.path()));
        assert_eq!(doc[0], "open");
        assert!(doc[1].ends_with("notes.txt"), "{doc:?}");
        assert_eq!(
            s(launch_command(Os::Linux, "firefox", &args, dir.path())),
            ["firefox", "--private"]
        );
        let doc = s(launch_command(Os::Linux, "notes.txt", &[], dir.path()));
        assert_eq!(doc[0], "xdg-open");
        let script = s(launch_command(Os::Linux, "run.sh", &[], dir.path()));
        assert!(script[0].ends_with("run.sh"), "{script:?}");
        assert_eq!(
            s(launch_command(
                Os::Linux,
                "https://kor.dev",
                &[],
                dir.path()
            )),
            ["xdg-open", "https://kor.dev"]
        );
    }

    #[test]
    fn clipboard_prefers_wayland_tools_on_wayland() {
        assert_eq!(clipboard_commands(Os::Mac, false, false)[0].0, "pbpaste");
        assert_eq!(clipboard_commands(Os::Mac, true, true)[0].0, "pbcopy");
        assert_eq!(clipboard_commands(Os::Linux, false, true)[0].0, "wl-paste");
        assert_eq!(clipboard_commands(Os::Linux, true, false)[0].0, "xclip");
        assert_eq!(clipboard_commands(Os::Linux, true, true).len(), 3);
    }

    #[test]
    fn ps_output_from_linux_and_macos_parses() {
        let linux = include_str!("../fixtures/ps_linux.txt");
        let rows = parse_ps(linux);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["name"], "firefox");
        assert_eq!(rows[0]["mem_mb"], 512.0);
        assert_eq!(rows[0]["cpu_s"], 3725.0);
        assert_eq!(rows[2]["cpu_s"], 90061.0, "{}", rows[2]);
        let mac = include_str!("../fixtures/ps_macos.txt");
        let rows = parse_ps(mac);
        assert_eq!(rows[0]["name"], "Google Chrome Helper (Renderer)");
        assert_eq!(rows[0]["cpu_s"], 83.5);
        assert_eq!(rows[1]["name"], "Finder");
    }

    #[test]
    fn system_files_parse() {
        let rel = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(os_release_name(rel).unwrap(), "Ubuntu 24.04.1 LTS");
        let mem = "MemTotal:       16303428 kB\nMemFree:  1 kB\nMemAvailable:    8151714 kB\n";
        assert_eq!(meminfo(mem), Some((16_303_428, 8_151_714)));
        assert_eq!(
            boot_secs("{ sec = 1731200000, usec = 123 } Sun Nov 10 01:33:20 2024"),
            Some(1_731_200_000)
        );
        assert_eq!(uptime(90_061), "1d 1h 1m");
    }
}
