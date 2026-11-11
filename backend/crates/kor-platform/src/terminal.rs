//! Opening `kor` in the user's terminal ("Kor CLI ile aç", Master Build Prompt §25.1, ADR 0052).
//!
//! The terminal is the user's choice (`kor config shell.terminal <id>`) when it is installed,
//! otherwise the platform default: Windows Terminal, else `cmd` (Windows 11 hands a new console
//! to the user's default terminal app); Terminal.app on macOS; on Linux `$TERMINAL`, then the
//! desktop's own terminal (Konsole on KDE, xfce4-terminal on Xfce, GNOME Terminal…), then the
//! first known one on `PATH`. [`command`] is pure (tested for every terminal); [`open`] runs it.

use std::io;
use std::path::{Path, PathBuf};

use crate::Os;

/// Terminals Kor knows how to start in a folder with a command: `(id, name, os)`.
pub const KNOWN: &[(&str, &str, Os)] = &[
    ("wt", "Windows Terminal", Os::Windows),
    ("pwsh", "PowerShell 7", Os::Windows),
    ("powershell", "Windows PowerShell", Os::Windows),
    ("cmd", "Command Prompt", Os::Windows),
    ("terminal", "Terminal.app", Os::Mac),
    ("iterm", "iTerm2", Os::Mac),
    ("gnome-terminal", "GNOME Terminal", Os::Linux),
    ("konsole", "Konsole", Os::Linux),
    ("xfce4-terminal", "Xfce Terminal", Os::Linux),
    ("tilix", "Tilix", Os::Linux),
    ("x-terminal-emulator", "Debian default terminal", Os::Linux),
    ("xterm", "xterm", Os::Linux),
    ("kitty", "kitty", Os::Other),
    ("alacritty", "Alacritty", Os::Other),
    ("wezterm", "WezTerm", Os::Other),
];

/// A program to start: executable, arguments, working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Windows: give the program a new console window (console programs started from the GUI).
    pub new_console: bool,
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// POSIX shell single-quoted word.
pub fn sh_quote(v: &str) -> String {
    if !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        return v.to_owned();
    }
    format!("'{}'", v.replace('\'', r"'\''"))
}

/// `AppleScript` string literal.
fn as_quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', r"\\").replace('"', "\\\""))
}

/// `cmd.exe` argument (inside `/k "…"`): quoted, `"` and `%` neutralised.
fn cmd_quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "").replace('%', "%%"))
}

/// `PowerShell` single-quoted literal.
fn ps_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

#[allow(clippy::too_many_lines)] // one arm per terminal
/// The command for terminal `id` on `os`, opening `root` and running `kor args…`; `None` for an
/// unknown id or one that does not exist on `os`.
pub fn command(os: Os, id: &str, root: &Path, kor: &Path, args: &[String]) -> Option<Launch> {
    let known = KNOWN.iter().find(|(k, _, _)| *k == id)?;
    if known.2 != Os::Other && known.2 != os {
        return None;
    }
    let exe_text = s(kor);
    let mut run_line: Vec<String> = vec![exe_text.clone()];
    run_line.extend(args.iter().cloned());
    let shell_line = format!(
        "cd {} && {}",
        sh_quote(&s(root)),
        run_line.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ")
    );
    let l = |program: &str, a: Vec<String>, new_console: bool| Launch {
        program: program.to_owned(),
        args: a,
        cwd: root.to_path_buf(),
        new_console,
    };
    Some(match id {
        "wt" => {
            let mut a = vec!["-d".to_owned(), s(root)];
            a.extend(run_line);
            l("wt.exe", a, false)
        }
        "pwsh" | "powershell" => {
            let line = std::iter::once(format!("& {}", ps_quote(&exe_text)))
                .chain(args.iter().map(|a| ps_quote(a)))
                .collect::<Vec<_>>()
                .join(" ");
            l(
                if id == "pwsh" { "pwsh.exe" } else { "powershell.exe" },
                vec!["-NoExit".into(), "-Command".into(), line],
                true,
            )
        }
        "cmd" => {
            let line = run_line.iter().map(|a| cmd_quote(a)).collect::<Vec<_>>().join(" ");
            l("cmd.exe", vec!["/s".into(), "/k".into(), format!("\"{line}\"")], true)
        }
        "terminal" => l(
            "osascript",
            vec![
                "-e".into(),
                format!("tell application \"Terminal\" to do script {}", as_quote(&shell_line)),
                "-e".into(),
                "tell application \"Terminal\" to activate".into(),
            ],
            false,
        ),
        "iterm" => l(
            "osascript",
            vec![
                "-e".into(),
                format!(
                    "tell application \"iTerm\" to create window with default profile command {}",
                    as_quote(&format!("/bin/zsh -lc {}", sh_quote(&format!("{shell_line}; exec $SHELL"))))
                ),
                "-e".into(),
                "tell application \"iTerm\" to activate".into(),
            ],
            false,
        ),
        "gnome-terminal" => {
            let mut a = vec![format!("--working-directory={}", s(root)), "--".into()];
            a.extend(run_line);
            l("gnome-terminal", a, false)
        }
        "konsole" => {
            let mut a = vec!["--workdir".into(), s(root), "-e".into()];
            a.extend(run_line);
            l("konsole", a, false)
        }
        "xfce4-terminal" => {
            let mut a = vec!["--working-directory".into(), s(root), "-x".into()];
            a.extend(run_line);
            l("xfce4-terminal", a, false)
        }
        "tilix" => l(
            "tilix",
            vec!["-w".into(), s(root), "-e".into(), run_line.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ")],
            false,
        ),
        "x-terminal-emulator" | "xterm" => {
            let mut a = vec!["-e".to_owned()];
            a.extend(run_line);
            l(id, a, false)
        }
        "kitty" => {
            let mut a = vec!["--directory".into(), s(root)];
            a.extend(run_line);
            l(if os == Os::Windows { "kitty.exe" } else { "kitty" }, a, false)
        }
        "alacritty" => {
            let mut a = vec!["--working-directory".into(), s(root), "-e".into()];
            a.extend(run_line);
            l(if os == Os::Windows { "alacritty.exe" } else { "alacritty" }, a, false)
        }
        "wezterm" => {
            let mut a = vec!["start".into(), "--cwd".into(), s(root), "--".into()];
            a.extend(run_line);
            l(if os == Os::Windows { "wezterm.exe" } else { "wezterm" }, a, false)
        }
        _ => return None,
    })
}

/// Facts [`choose`] decides on (injected in tests).
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Terminal ids whose program is installed.
    pub installed: Vec<String>,
    /// `$TERMINAL` (Linux convention).
    pub terminal_var: Option<String>,
    /// `$XDG_CURRENT_DESKTOP`.
    pub desktop: Option<String>,
}

/// Is terminal `id` installed on this machine?
fn present(os: Os, id: &str) -> bool {
    match (os, id) {
        (Os::Windows, "cmd" | "powershell") | (Os::Mac, "terminal") => true,
        (Os::Mac, "iterm") => Path::new("/Applications/iTerm.app").is_dir(),
        (Os::Mac, "kitty") => Path::new("/Applications/kitty.app").is_dir() || crate::on_path::which(id).is_some(),
        (Os::Mac, "alacritty") => Path::new("/Applications/Alacritty.app").is_dir() || crate::on_path::which(id).is_some(),
        (Os::Mac, "wezterm") => Path::new("/Applications/WezTerm.app").is_dir() || crate::on_path::which(id).is_some(),
        (_, "wt") => crate::on_path::which("wt").is_some(),
        _ => crate::on_path::which(id).is_some(),
    }
}

impl Env {
    /// This machine.
    pub fn current() -> Self {
        let os = crate::os();
        Self {
            installed: KNOWN
                .iter()
                .filter(|(_, _, o)| *o == os || *o == Os::Other)
                .filter(|(id, _, _)| present(os, id))
                .map(|(id, _, _)| (*id).to_owned())
                .collect(),
            terminal_var: std::env::var("TERMINAL").ok().filter(|v| !v.trim().is_empty()),
            desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
        }
    }
}

/// The terminal id to use: `preferred` when installed, otherwise the platform default.
pub fn choose(os: Os, preferred: Option<&str>, env: &Env) -> Option<String> {
    let has = |id: &str| env.installed.iter().any(|i| i == id);
    if let Some(p) = preferred.map(str::trim).filter(|p| !p.is_empty())
        && has(p)
    {
        return Some(p.to_owned());
    }
    let order: Vec<&str> = match os {
        Os::Windows => vec!["wt", "pwsh", "cmd"],
        Os::Mac => vec!["terminal"],
        _ => {
            let de = env.desktop.as_deref().unwrap_or_default().to_ascii_uppercase();
            let mut v: Vec<&str> = Vec::new();
            if let Some(t) = env.terminal_var.as_deref()
                && let Some(name) = Path::new(t).file_name().and_then(|n| n.to_str())
                && let Some((id, _, _)) = KNOWN.iter().find(|(k, _, _)| *k == name)
            {
                v.push(id);
            }
            if de.contains("KDE") {
                v.push("konsole");
            }
            if de.contains("XFCE") {
                v.push("xfce4-terminal");
            }
            v.extend(["gnome-terminal", "konsole", "xfce4-terminal", "tilix", "kitty", "alacritty", "wezterm", "x-terminal-emulator", "xterm"]);
            v
        }
    };
    order.into_iter().find(|id| has(id)).map(str::to_owned)
}

/// Ids of the installed terminals (`kor config shell.terminal` choices, `kor doctor`).
pub fn installed() -> Vec<String> {
    Env::current().installed
}

/// Open a terminal at `root` running `kor args…`; `preferred` is `shell.terminal`.
///
/// # Errors
/// When no known terminal is installed or it cannot be started.
pub fn open(root: &Path, preferred: Option<&str>, kor: &Path, args: &[String]) -> io::Result<()> {
    let os = crate::os();
    let id = choose(os, preferred, &Env::current())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no known terminal is installed"))?;
    let launch = command(os, &id, root, kor, args)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, format!("terminal `{id}` is not supported here")))?;
    spawn(&launch)
}

fn spawn(l: &Launch) -> io::Result<()> {
    let mut c = std::process::Command::new(&l.program);
    c.args(&l.args)
        .current_dir(&l.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NEW_CONSOLE: a console of its own (the user's default terminal on Windows 11).
        c.creation_flags(if l.new_console { 0x0000_0010 } else { 0x0800_0000 });
    }
    c.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(ids: &[&str]) -> Env {
        Env { installed: ids.iter().map(|s| (*s).to_owned()).collect(), ..Env::default() }
    }

    #[test]
    fn every_known_terminal_has_a_command_on_its_os() {
        let root = Path::new("/p q");
        let kor = Path::new("/bin/kor");
        let args = vec!["fix it".to_owned()];
        for (id, _, os) in KNOWN {
            let target = if *os == Os::Other { Os::Linux } else { *os };
            let l = command(target, id, root, kor, &args).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(l.cwd, root);
            let joined = l.args.join(" ");
            assert!(joined.contains("kor"), "{id}: {joined}");
        }
        assert!(command(Os::Linux, "wt", root, kor, &[]).is_none());
        assert!(command(Os::Linux, "nope", root, kor, &[]).is_none());
    }

    #[test]
    fn commands_quote_paths_and_arguments() {
        let gt = command(Os::Linux, "gnome-terminal", Path::new("/a b"), Path::new("/k"), &["x y".into()]).unwrap();
        assert_eq!(gt.args, vec!["--working-directory=/a b", "--", "/k", "x y"]);
        let t = command(Os::Mac, "terminal", Path::new("/it's"), Path::new("/k"), &[]).unwrap();
        assert_eq!(t.args[1], r#"tell application "Terminal" to do script "cd '/it'\\''s' && /k""#);
        let cmd = command(Os::Windows, "cmd", Path::new(r"C:\p"), Path::new(r"C:\K\kor.exe"), &["50%".into()]).unwrap();
        assert_eq!(cmd.args[2], r#"""C:\K\kor.exe" "50%%"""#);
        assert!(cmd.new_console);
        let wt = command(Os::Windows, "wt", Path::new(r"C:\p"), Path::new(r"C:\K\kor.exe"), &[]).unwrap();
        assert_eq!(wt.args, vec!["-d", r"C:\p", r"C:\K\kor.exe"]);
        let ps = command(Os::Windows, "pwsh", Path::new(r"C:\p"), Path::new(r"C:\it's\kor.exe"), &[]).unwrap();
        assert_eq!(ps.args[2], r"& 'C:\it''s\kor.exe'");
    }

    #[test]
    fn choice_prefers_the_setting_then_the_platform_default() {
        assert_eq!(choose(Os::Windows, Some("pwsh"), &env(&["wt", "pwsh", "cmd"])).as_deref(), Some("pwsh"));
        assert_eq!(choose(Os::Windows, Some("alacritty"), &env(&["cmd"])).as_deref(), Some("cmd"));
        assert_eq!(choose(Os::Windows, None, &env(&["wt", "cmd"])).as_deref(), Some("wt"));
        assert_eq!(choose(Os::Mac, None, &env(&["terminal", "iterm"])).as_deref(), Some("terminal"));
        let mut kde = env(&["gnome-terminal", "konsole"]);
        kde.desktop = Some("KDE".into());
        assert_eq!(choose(Os::Linux, None, &kde).as_deref(), Some("konsole"));
        let mut var = env(&["gnome-terminal", "kitty"]);
        var.terminal_var = Some("/usr/bin/kitty".into());
        assert_eq!(choose(Os::Linux, None, &var).as_deref(), Some("kitty"));
        assert_eq!(choose(Os::Linux, None, &env(&[])), None);
    }
}
