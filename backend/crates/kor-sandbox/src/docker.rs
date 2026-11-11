//! Docker backend (ADR 0033): one throwaway container per command.
//!
//! The project is bind-mounted read-write at the same path (Windows `C:\a` → `/c/a`), the
//! container's own file system is read-only with a private `/tmp`, all capabilities are dropped,
//! no network unless allowed, memory and CPU flags from [`crate::Options`]. The image is never
//! pulled implicitly (`--pull=never`); [`docker_pull`] is the explicit step. Environment values
//! travel through the `docker` client's environment (`-e NAME`), never on its command line.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use kor_platform::{NoWindow as _, Os};

use crate::Policy;

/// Small image with a POSIX `sh` (busybox).
pub const DEFAULT_IMAGE: &str = "alpine:3.20";

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Image references Kor passes to `docker`: no option-looking or shell-special text.
pub fn valid_image(image: &str) -> bool {
    !image.is_empty()
        && image.len() <= 255
        && !image.starts_with('-')
        && image.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/' | b':' | b'@')
        })
}

/// Path of `host` inside the container: the same on Linux/macOS, `/c/...` for `C:\...`.
fn in_container(host: &Path) -> String {
    let s = host.to_string_lossy();
    if kor_platform::os() != Os::Windows {
        return s.into_owned();
    }
    let s = s.trim_start_matches(r"\\?\").replace('\\', "/");
    match s.as_bytes() {
        [d, b':', rest @ ..] if d.is_ascii_alphabetic() => format!(
            "/{}{}",
            char::from(*d).to_ascii_lowercase(),
            String::from_utf8_lossy(rest)
        ),
        _ => s,
    }
}

/// `docker run` arguments for `program args` (without the leading `docker`).
pub(crate) fn run_args(
    policy: &Policy,
    name: &str,
    cwd: &Path,
    env_names: &[OsString],
    program: &OsStr,
    args: &[OsString],
) -> Result<Vec<OsString>, String> {
    let o = &policy.options;
    if !valid_image(&o.image) {
        return Err(format!("invalid Docker image name: {}", o.image));
    }
    let root = in_container(&policy.root);
    let mut a: Vec<OsString> = [
        "run",
        "--rm",
        "--pull=never",
        "--init",
        "--name",
        name,
        "--read-only",
        "--tmpfs",
        "/tmp",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
    ]
    .map(OsString::from)
    .to_vec();
    // Docker's networks are all or nothing; an allowlist turns the default bridge on.
    if !policy.network {
        a.extend(["--network", "none"].map(OsString::from));
    }
    let mut mount = OsString::from("type=bind,source=");
    mount.push(policy.root.as_os_str());
    mount.push(format!(",target={root}"));
    a.extend([OsString::from("--mount"), mount]);
    a.extend(["-w".into(), in_container(cwd).into()]);
    if o.memory_mb > 0 {
        let m = format!("{}m", o.memory_mb);
        a.extend([
            "--memory".into(),
            m.clone().into(),
            "--memory-swap".into(),
            m.into(),
        ]);
    }
    if o.cpu_percent > 0 && o.cpu_percent < 100 {
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let share =
            f64::from(u32::try_from(cpus).unwrap_or(u32::MAX)) * f64::from(o.cpu_percent) / 100.0;
        a.extend(["--cpus".into(), format!("{:.2}", share.max(0.1)).into()]);
    }
    // Files written into the project keep the user's ownership on Linux.
    if kor_platform::os() == Os::Linux
        && let Some((uid, gid)) = kor_platform::owner_ids(&policy.root)
    {
        a.extend(["--user".into(), format!("{uid}:{gid}").into()]);
    }
    for n in env_names {
        a.push("-e".into());
        a.push(n.clone());
    }
    a.push(o.image.clone().into());
    a.push(program.to_owned());
    a.extend(args.iter().cloned());
    Ok(a)
}

/// Removes the container if the command is dropped before it ends (timeout, cancel).
#[derive(Debug)]
pub(crate) struct Guard {
    name: String,
    done: bool,
}

impl Guard {
    pub(crate) fn finished(&mut self) {
        self.done = true;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let name = std::mem::take(&mut self.name);
        // Killing the client leaves the container running; remove it off the async runtime.
        std::thread::spawn(move || {
            let _ = std::process::Command::new(kor_platform::exe("docker"))
                .args(["rm", "-f", &name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .no_window()
                .status();
        });
    }
}

pub(crate) fn spawn(
    policy: &Policy,
    cmd: &std::process::Command,
    args: &[OsString],
    cwd: &Path,
) -> io::Result<(tokio::process::Child, Guard)> {
    let name = format!("kor-sbx-{}-{}", std::process::id(), next_id());
    let env_names: Vec<OsString> = cmd
        .get_envs()
        .filter(|(_, v)| v.is_some())
        .map(|(k, _)| k.to_owned())
        .collect();
    let a = run_args(policy, &name, cwd, &env_names, cmd.get_program(), args)
        .map_err(io::Error::other)?;
    let mut c = tokio::process::Command::new(kor_platform::exe("docker"));
    c.args(a)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .no_window();
    for (k, v) in cmd.get_envs() {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
    let child = c.spawn()?;
    Ok((child, Guard { name, done: false }))
}

/// Docker as seen from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockerState {
    pub installed: bool,
    pub daemon: bool,
    pub image_present: bool,
}

fn docker_ok(args: &[&str]) -> Option<bool> {
    std::process::Command::new(kor_platform::exe("docker"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .no_window()
        .status()
        .ok()
        .map(|s| s.success())
}

/// Whether the CLI runs, the daemon answers and `image` is already local. Never pulls.
pub fn docker_state(image: &str) -> DockerState {
    let Some(daemon) = docker_ok(&["version", "--format", "{{.Server.Version}}"]) else {
        return DockerState {
            installed: false,
            daemon: false,
            image_present: false,
        };
    };
    let image_present =
        daemon && valid_image(image) && docker_ok(&["image", "inspect", image]) == Some(true);
    DockerState {
        installed: true,
        daemon,
        image_present,
    }
}

/// Pull `image` (the explicit step behind `kor sandbox pull`). Progress goes to the terminal
/// when `inherit` is set, else it is captured.
pub fn docker_pull(image: &str, inherit: bool) -> io::Result<std::process::Output> {
    if !valid_image(image) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid Docker image name: {image}"),
        ));
    }
    let mut c = std::process::Command::new(kor_platform::exe("docker"));
    c.args(["pull", image]).stdin(Stdio::null()).no_window();
    if inherit {
        c.status().map(|status| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    } else {
        c.output()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, SandboxKind};

    fn strs(v: &[OsString]) -> Vec<String> {
        v.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn image_names_are_checked() {
        for ok in [
            "alpine:3.20",
            "ghcr.io/org/img@sha256:abc",
            "debian:bookworm-slim",
        ] {
            assert!(valid_image(ok), "{ok}");
        }
        for bad in ["", "--privileged", "a b", "img;rm", "x$(y)"] {
            assert!(!valid_image(bad), "{bad}");
        }
    }

    #[test]
    fn run_args_confine_the_container() {
        let root = std::env::temp_dir();
        let policy = Policy {
            root: root.clone(),
            network: false,
            options: Options {
                memory_mb: 512,
                cpu_percent: 50,
                ..Options::default()
            },
        };
        let a = strs(
            &run_args(
                &policy,
                "kor-sbx-1-1",
                &root,
                &["SECRET".into()],
                "sh".as_ref(),
                &["-c".into(), "ls".into()],
            )
            .unwrap(),
        );
        let has = |w: &[&str]| a.windows(w.len()).any(|x| x == w);
        assert!(has(&["--pull=never"]) && has(&["--read-only"]) && has(&["--network", "none"]));
        assert!(has(&["--memory", "512m", "--memory-swap", "512m"]));
        assert!(has(&["--cap-drop", "ALL"]) && has(&["-e", "SECRET"]));
        assert!(a.iter().any(|s| s == "--cpus"));
        assert!(
            !a.iter().any(|s| s.contains("SECRET=")),
            "values never on the command line"
        );
        assert_eq!(&a[a.len() - 4..], ["alpine:3.20", "sh", "-c", "ls"]);
        let target = in_container(&root);
        assert!(a.iter().any(|s| s.ends_with(&format!(",target={target}"))));
        assert!(target.starts_with('/'), "{target}");

        let open = Policy {
            network: true,
            ..policy.clone()
        };
        let a = strs(&run_args(&open, "n", &root, &[], "sh".as_ref(), &[]).unwrap());
        assert!(!a.iter().any(|s| s == "none"));
        let bad = Policy {
            options: Options {
                image: "--privileged".into(),
                ..Options::default()
            },
            ..policy
        };
        assert!(run_args(&bad, "n", &root, &[], "sh".as_ref(), &[]).is_err());
    }

    #[test]
    fn windows_paths_map_to_drive_folders() {
        if kor_platform::os() == Os::Windows {
            assert_eq!(in_container(Path::new(r"C:\Work\Proj")), "/c/Work/Proj");
            assert_eq!(in_container(Path::new(r"\\?\D:\x")), "/d/x");
        } else {
            assert_eq!(in_container(Path::new("/home/u/p")), "/home/u/p");
        }
    }

    /// Needs Docker with `alpine:3.20` pulled: `cargo test -p kor-sandbox -- --ignored docker`.
    #[tokio::test]
    #[ignore = "needs a running Docker daemon and the alpine:3.20 image"]
    async fn docker_blocks_outside_writes_and_network() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let root = std::path::PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
        let policy = Policy::new(root.clone());
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args([
            "-c",
            "echo in > inside.txt; echo out > /etc/kor-probe; wget -q -T 3 http://example.com -O /dev/null",
        ])
        .current_dir(&root);
        let out = crate::spawn(SandboxKind::Docker, &policy, &cmd)
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(root.join("inside.txt").is_file(), "{text}");
        let d = crate::denials(SandboxKind::Docker, &policy, &text);
        assert!(
            d.iter()
                .any(|d| d.target.as_deref() == Some("/etc/kor-probe")),
            "{text} {d:?}"
        );
        assert!(
            d.iter().any(|d| d.kind == crate::DenialKind::Network),
            "{text}"
        );
    }
}
