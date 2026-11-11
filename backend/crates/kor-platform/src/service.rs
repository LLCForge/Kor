//! Per-user autostart for `kor daemon` (`kor daemon install|uninstall`, ADR 0039): a systemd user
//! unit, a launchd agent or a Task Scheduler logon task. Only on the user's explicit command;
//! nothing system-wide, no administrator rights.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{NoWindow as _, Os, os};

const UNIT: &str = "kor-daemon.service";
const LABEL: &str = "dev.kor.daemon";
const TASK: &str = "Kor Daemon";

/// Where the service definition lives (`None` on Windows: Task Scheduler keeps it).
pub fn definition_path() -> Option<PathBuf> {
    match os() {
        Os::Linux => dirs::config_dir().map(|d| d.join("systemd").join("user").join(UNIT)),
        Os::Mac => dirs::home_dir().map(|h| {
            h.join("Library")
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist"))
        }),
        Os::Windows | Os::Other => None,
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The definition text for `exe daemon run` on `target`.
pub fn definition(target: Os, exe: &Path) -> String {
    let exe = exe.display().to_string();
    match target {
        Os::Mac => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>{LABEL}</string>\n\
             <key>ProgramArguments</key><array><string>{}</string><string>daemon</string><string>run</string></array>\n\
             <key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
             </dict></plist>\n",
            xml(&exe)
        ),
        _ => format!(
            "[Unit]\nDescription=Kor trigger daemon\n\n[Service]\nExecStart=\"{}\" daemon run\n\
             Restart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            exe.replace('"', "\\\"")
        ),
    }
}

fn run(program: &str, args: &[&str]) -> std::io::Result<()> {
    let out = Command::new(program).args(args).no_window().output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{program}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Install and start the autostart entry for `exe`; returns where it was put.
///
/// # Errors
/// On an unsupported OS or when the service manager refuses.
pub fn install(exe: &Path) -> std::io::Result<String> {
    match os() {
        Os::Windows => {
            let tr = format!("\"{}\" daemon run", exe.display());
            run(
                "schtasks",
                &[
                    "/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", TASK, "/TR", &tr,
                ],
            )?;
            Ok(format!("Task Scheduler: {TASK}"))
        }
        target @ (Os::Linux | Os::Mac) => {
            let path =
                definition_path().ok_or_else(|| std::io::Error::other("no home directory"))?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, definition(target, exe))?;
            if target == Os::Linux {
                run("systemctl", &["--user", "daemon-reload"])?;
                run("systemctl", &["--user", "enable", "--now", UNIT])?;
            } else {
                run("launchctl", &["load", "-w", &path.display().to_string()])?;
            }
            Ok(path.display().to_string())
        }
        Os::Other => Err(std::io::Error::other("unsupported operating system")),
    }
}

/// Stop and remove the autostart entry (nothing to do when it is missing).
///
/// # Errors
/// When the service manager refuses.
pub fn uninstall() -> std::io::Result<()> {
    match os() {
        Os::Windows => run("schtasks", &["/Delete", "/F", "/TN", TASK]),
        Os::Linux | Os::Mac => {
            let Some(path) = definition_path().filter(|p| p.exists()) else {
                return Ok(());
            };
            if os() == Os::Linux {
                let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
            } else {
                let _ = run("launchctl", &["unload", "-w", &path.display().to_string()]);
            }
            std::fs::remove_file(path)
        }
        Os::Other => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_run_the_daemon() {
        let exe = Path::new("/opt/kor & co/kor");
        let unit = definition(Os::Linux, exe);
        assert!(unit.contains("ExecStart=\"/opt/kor & co/kor\" daemon run"));
        let plist = definition(Os::Mac, exe);
        assert!(plist.contains("<string>/opt/kor &amp; co/kor</string><string>daemon</string>"));
    }
}
