//! Taskbar / Dock shortcuts (Master Build Prompt §25.3, ADR 0052): the last projects, "Yeni görev" and
//! "Çalışan görevler".
//!
//! - Windows: the Jump List (`ICustomDestinationList`): a category of recent projects plus user tasks,
//!   each an `IShellLinkW` that starts the desktop with a `kor://` link (the second launch is forwarded
//!   to the running window).
//! - macOS: the Dock menu (`applicationDockMenu:` added to the app delegate); items open their `kor://`
//!   link through `NSWorkspace`, which macOS hands back to the running app.
//! - Linux: `[Desktop Action …]` groups in Kor's user-level `.desktop` entry, written only when that entry
//!   exists (the user installed the integration, `kor integrate install`).
//!
//! Callers build the [`List`]; this module only places it. [`apply`] must run on the main thread
//! (macOS `AppKit`); Windows does its COM work on its own thread.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
mod mac;

/// File name of Kor's user-level `.desktop` entry (`~/.local/share/applications/`), shared with
/// `shell_integration`; this module only rewrites its `[Desktop Action kor-*]` groups.
pub const LINUX_ENTRY_NAME: &str = "dev.kor.desktop-integration.desktop";

/// Prefix of the action ids this module owns in the `.desktop` entry.
const ACTION_PREFIX: &str = "kor-";

/// One shortcut: its label and the desktop arguments it starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub args: Vec<String>,
}

/// What to show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct List {
    /// Title of the recent projects group ("Son projeler").
    pub recent_title: String,
    pub recent: Vec<Item>,
    /// Fixed tasks ("Yeni görev", "Çalışan görevler").
    pub tasks: Vec<Item>,
}

/// `~/.local/share/applications/<LINUX_ENTRY_NAME>`.
pub fn linux_entry() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("applications").join(LINUX_ENTRY_NAME))
}

/// Place `list` for the desktop executable `exe`. On Linux nothing happens while the user-level entry
/// is missing.
///
/// # Errors
/// When the OS refuses the list or the entry cannot be written.
pub fn apply(exe: &Path, list: &List) -> io::Result<()> {
    match crate::os() {
        crate::Os::Windows => apply_windows(exe, list),
        crate::Os::Mac => apply_mac(list),
        crate::Os::Linux => match linux_entry() {
            Some(entry) if entry.is_file() => {
                let text = std::fs::read_to_string(&entry)?;
                let next = linux_actions(&text, exe, list);
                if next != text {
                    std::fs::write(&entry, next)?;
                }
                Ok(())
            }
            _ => Ok(()),
        },
        crate::Os::Other => Ok(()),
    }
}

#[cfg(windows)]
fn apply_windows(exe: &Path, list: &List) -> io::Result<()> {
    let (exe, list) = (exe.to_path_buf(), list.clone());
    // Own STA thread: the caller's apartment (WebView2 main thread) is not ours to change.
    std::thread::spawn(move || win::apply(&exe, &list))
        .join()
        .map_err(|_| io::Error::other("jump list thread panicked"))?
}

#[cfg(not(windows))]
fn apply_windows(_: &Path, _: &List) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(target_os = "macos")]
fn apply_mac(list: &List) -> io::Result<()> {
    mac::apply(list)
}

#[cfg(not(target_os = "macos"))]
fn apply_mac(_: &List) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// One Windows command-line argument, quoted the way `CommandLineToArgvW` reads it back.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut slashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

/// Arguments joined for `IShellLinkW::SetArguments`.
pub fn windows_args(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_windows_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One `Exec=` argument per the Desktop Entry spec: quoted when needed, `%` doubled.
fn desktop_arg(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    if arg.is_empty() || arg.contains(reserved) {
        let escaped: String = arg
            .chars()
            .flat_map(|c| {
                let esc = matches!(c, '"' | '`' | '$' | '\\');
                esc.then_some('\\').into_iter().chain(std::iter::once(c))
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        arg
    }
}

/// `exe args…` as a `.desktop` `Exec=` value.
pub fn desktop_exec(exe: &Path, args: &[String]) -> String {
    std::iter::once(exe.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|a| desktop_arg(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` (a `.desktop` file) with Kor's action groups replaced by `list`; other groups and other
/// actions are kept. Idempotent.
pub fn linux_actions(text: &str, exe: &Path, list: &List) -> String {
    let items: Vec<(String, &Item)> = list
        .recent
        .iter()
        .enumerate()
        .map(|(i, it)| (format!("{ACTION_PREFIX}recent-{i}"), it))
        .chain(
            list.tasks
                .iter()
                .enumerate()
                .map(|(i, it)| (format!("{ACTION_PREFIX}task-{i}"), it)),
        )
        .collect();
    let mut out = String::new();
    let mut in_ours = false;
    let mut in_main = false;
    let mut saw_actions = false;
    let ours: Vec<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
    let rewrite_actions = |line: &str| -> String {
        let kept = line
            .trim_start_matches("Actions=")
            .split(';')
            .filter(|a| !a.is_empty() && !a.starts_with(ACTION_PREFIX))
            .map(str::to_owned);
        let all: Vec<String> = kept.chain(ours.iter().map(|s| (*s).to_owned())).collect();
        if all.is_empty() {
            String::new()
        } else {
            format!("Actions={};\n", all.join(";"))
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_main && !saw_actions {
                out.push_str(&rewrite_actions("Actions="));
                saw_actions = true;
            }
            in_main = trimmed == "[Desktop Entry]";
            in_ours = trimmed
                .strip_prefix("[Desktop Action ")
                .is_some_and(|r| r.starts_with(ACTION_PREFIX));
            if in_ours {
                continue;
            }
        } else if in_ours {
            continue;
        } else if in_main && trimmed.starts_with("Actions=") {
            out.push_str(&rewrite_actions(trimmed));
            saw_actions = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_main && !saw_actions {
        out.push_str(&rewrite_actions("Actions="));
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    for (id, it) in &items {
        let name = it.title.replace(['\n', '\r'], " ");
        let _ = write!(
            out,
            "\n[Desktop Action {id}]\nName={name}\nExec={}\n",
            desktop_exec(exe, &it.args)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> List {
        List {
            recent_title: "Son projeler".into(),
            recent: vec![Item {
                title: "my app".into(),
                args: vec!["kor://open?path=/home/u/my%20app".into()],
            }],
            tasks: vec![
                Item {
                    title: "Yeni görev".into(),
                    args: vec!["kor://new".into()],
                },
                Item {
                    title: "Çalışan görevler".into(),
                    args: vec!["kor://running".into()],
                },
            ],
        }
    }

    #[test]
    fn windows_arguments_round_trip_quoting() {
        assert_eq!(quote_windows_arg("kor://new"), "kor://new");
        assert_eq!(quote_windows_arg("a b"), "\"a b\"");
        assert_eq!(quote_windows_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_windows_arg(r#"x"y"#), r#""x\"y""#);
        assert_eq!(quote_windows_arg(""), "\"\"");
        assert_eq!(
            windows_args(&["--open".into(), "C:\\p q".into()]),
            "--open \"C:\\p q\""
        );
    }

    #[test]
    fn desktop_entry_actions_are_owned_and_idempotent() {
        let base = "[Desktop Entry]\nName=Kor\nExec=kor-desktop %F\nActions=other;kor-recent-9;\n\n[Desktop Action other]\nName=Other\nExec=x\n\n[Desktop Action kor-recent-9]\nName=Old\nExec=old\n";
        let exe = Path::new("/opt/Kor/kor-desktop");
        let once = linux_actions(base, exe, &list());
        assert_eq!(linux_actions(&once, exe, &list()), once, "idempotent");
        assert!(
            once.contains("Actions=other;kor-recent-0;kor-task-0;kor-task-1;\n"),
            "{once}"
        );
        assert!(once.contains("[Desktop Action other]\nName=Other\nExec=x\n"));
        assert!(!once.contains("kor-recent-9") && !once.contains("Name=Old"));
        assert!(
            once.contains("Exec=/opt/Kor/kor-desktop \"kor://open?path=/home/u/my%%20app\"\n"),
            "{once}"
        );
        assert!(once.contains("[Desktop Action kor-task-1]\nName=Çalışan görevler\nExec=/opt/Kor/kor-desktop kor://running\n"));
        // An empty list removes Kor's groups and keeps the rest.
        let empty = linux_actions(&once, exe, &List::default());
        assert!(
            empty.contains("Actions=other;\n") && !empty.contains("Action kor-"),
            "{empty}"
        );
        // An entry without an Actions= line gets one inside [Desktop Entry].
        let added = linux_actions("[Desktop Entry]\nName=Kor\n", exe, &list());
        assert!(
            added.starts_with("[Desktop Entry]\nName=Kor\nActions=kor-recent-0;"),
            "{added}"
        );
    }

    #[test]
    fn exec_arguments_are_quoted_per_spec() {
        let exe = Path::new("/a b/kor-desktop");
        assert_eq!(
            desktop_exec(exe, &["kor://open?path=/x$y".into()]),
            "\"/a b/kor-desktop\" \"kor://open?path=/x\\$y\""
        );
    }
}
