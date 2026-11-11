//! `~/Kor/` folder (Master Build Prompt §27.5, ADR 0052): folder links and the `kor mount` drive.
//!
//! - Links: a symlink on macOS / Linux, a directory junction on Windows (no administrator rights
//!   or developer mode needed, unlike Windows symlinks). Removing a link never touches its target.
//! - Drive: `kor mount` serves the same tree over `WebDAV` on `127.0.0.1` (Kor's own server, in
//!   `kor_runtime::korfolder::dav`) and asks the operating system's built-in `WebDAV` client to
//!   mount it: `net use` (Windows `WebClient`), `mount_webdav` (macOS), `gio mount` (GNOME/`GVfs` on
//!   Linux). No kernel driver (`FUSE`, `WinFsp`, `macFUSE`) has to be installed; when the client is
//!   missing the folder stays a normal folder and the error says which package to install.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{NoWindow as _, Os, os};

/// Make `link` point at the directory `target`.
///
/// # Errors
/// If `link` already exists or the link cannot be created.
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    if link.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", link.display()),
        ));
    }
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let out = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .no_window()
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "mklink /J: {}",
                String::from_utf8_lossy(&out.stdout).trim()
            )))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = target;
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Where `link` points when it is a symlink or a junction.
pub fn link_target(link: &Path) -> Option<PathBuf> {
    std::fs::read_link(link).ok().map(|t| {
        // Junctions read back as `\\?\C:\…`.
        let s = t.to_string_lossy();
        s.strip_prefix(r"\\?\").map_or(t.clone(), PathBuf::from)
    })
}

/// Remove the link itself; the target and its contents stay.
///
/// # Errors
/// If `link` is not a link (nothing is removed then).
pub fn remove_link(link: &Path) -> io::Result<()> {
    if link_target(link).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a link", link.display()),
        ));
    }
    // A junction (and a symlink to a directory on Windows) is removed as a directory.
    std::fs::remove_file(link).or_else(|_| std::fs::remove_dir(link))
}

/// How the drive is mounted on this OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    pub program: String,
    pub args: Vec<String>,
    /// Where the drive appears (a drive letter on Windows, a folder elsewhere).
    pub at: String,
}

/// The command that mounts `url` (`http://127.0.0.1:<port>/<token>/`) at `at` on `target`.
/// `at` on Windows is a drive letter (`K:`) or `*` for the next free one.
pub fn mount_plan(target: Os, url: &str, at: &str) -> MountPlan {
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    match target {
        Os::Windows => MountPlan {
            program: "net".into(),
            args: v(&["use", at, url, "/persistent:no"]),
            at: at.into(),
        },
        Os::Mac => MountPlan {
            program: "/sbin/mount_webdav".into(),
            args: v(&["-S", "-v", "Kor", url, at]),
            at: at.into(),
        },
        Os::Linux | Os::Other => MountPlan {
            program: "gio".into(),
            args: v(&["mount", &url.replacen("http://", "dav://", 1)]),
            at: at.into(),
        },
    }
}

/// The command that unmounts what [`mount_plan`] mounted.
pub fn unmount_plan(target: Os, url: &str, at: &str) -> MountPlan {
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    match target {
        Os::Windows => MountPlan {
            program: "net".into(),
            args: v(&["use", at, "/delete", "/y"]),
            at: at.into(),
        },
        Os::Mac => MountPlan {
            program: "/sbin/umount".into(),
            args: v(&[at]),
            at: at.into(),
        },
        Os::Linux | Os::Other => MountPlan {
            program: "gio".into(),
            args: v(&["mount", "-u", &url.replacen("http://", "dav://", 1)]),
            at: at.into(),
        },
    }
}

/// What to install when the `WebDAV` client is missing.
pub fn client_hint(target: Os) -> &'static str {
    match target {
        Os::Windows => {
            "Windows WebClient service (Settings › Optional features › WebDAV Redirector)"
        }
        Os::Mac => "mount_webdav (part of macOS)",
        Os::Linux | Os::Other => "gvfs-backends (provides `gio mount` for dav://)",
    }
}

fn run(plan: &MountPlan) -> io::Result<String> {
    let out = Command::new(&plan.program)
        .args(&plan.args)
        .no_window()
        .output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(io::Error::other(text.trim().to_owned()))
    }
}

/// Mount the `WebDAV` `url` at `at` (created on macOS). Returns where the drive appeared: on
/// Windows the letter `net use` picked, on Linux the `GVfs` folder.
///
/// # Errors
/// When the client is missing (the message names it) or refuses.
pub fn mount(url: &str, at: &str) -> io::Result<String> {
    let target = os();
    if target == Os::Mac {
        std::fs::create_dir_all(at)?;
    }
    let plan = mount_plan(target, url, at);
    let out = run(&plan).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, client_hint(target))
        } else {
            e
        }
    })?;
    Ok(match target {
        Os::Windows => parse_net_use_drive(&out).unwrap_or_else(|| at.to_owned()),
        Os::Linux | Os::Other => {
            gvfs_dir(url).map_or_else(|| at.to_owned(), |p| p.display().to_string())
        }
        Os::Mac => at.to_owned(),
    })
}

/// Undo [`mount`].
///
/// # Errors
/// When the client refuses.
pub fn unmount(url: &str, at: &str) -> io::Result<()> {
    run(&unmount_plan(os(), url, at)).map(|_| ())
}

/// `Drive K: is now connected to …` → `K:`.
fn parse_net_use_drive(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|w| {
            w.len() == 2 && w.ends_with(':') && w.starts_with(|c: char| c.is_ascii_alphabetic())
        })
        .map(str::to_owned)
}

/// `GVfs` exposes `dav://host:port/path` under `$XDG_RUNTIME_DIR/gvfs/dav:host=…,port=…,ssl=false,prefix=…`.
fn gvfs_dir(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = hostport.split_once(':')?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let prefix = format!("/{}", path.trim_end_matches('/')).replace('/', "%2F");
    Some(PathBuf::from(runtime).join("gvfs").join(format!(
        "dav:host={host},port={port},ssl=false,prefix={prefix}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_point_at_targets_and_removal_keeps_them() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("store");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("a.txt"), "x").unwrap();
        let link = d.path().join("Kor").join("Kayıtlar");
        link_dir(&target, &link).unwrap();
        assert!(link.join("a.txt").is_file());
        assert_eq!(
            link_target(&link).map(|t| std::fs::canonicalize(t).unwrap()),
            Some(std::fs::canonicalize(&target).unwrap())
        );
        assert!(link_dir(&target, &link).is_err(), "never replaces");
        remove_link(&link).unwrap();
        assert!(!link.exists() && target.join("a.txt").is_file());
        assert!(
            remove_link(&target).is_err(),
            "a real folder is not removed"
        );
        assert!(target.exists());
    }

    #[test]
    fn mount_commands_per_os() {
        let url = "http://127.0.0.1:4567/tok/";
        let w = mount_plan(Os::Windows, url, "*");
        assert_eq!(w.program, "net");
        assert_eq!(w.args, ["use", "*", url, "/persistent:no"]);
        assert_eq!(
            unmount_plan(Os::Windows, url, "K:").args,
            ["use", "K:", "/delete", "/y"]
        );
        let m = mount_plan(Os::Mac, url, "/Users/a/KorDrive");
        assert_eq!(m.args.last().map(String::as_str), Some("/Users/a/KorDrive"));
        let l = mount_plan(Os::Linux, url, "");
        assert_eq!(l.args, ["mount", "dav://127.0.0.1:4567/tok/"]);
        assert_eq!(
            parse_net_use_drive("Drive K: is now connected to http://x."),
            Some("K:".into())
        );
        assert_eq!(
            parse_net_use_drive("Le lecteur Z: est maintenant connecté"),
            Some("Z:".into())
        );
    }
}
