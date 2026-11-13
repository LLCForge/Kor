//! Shell integration (§15.4): the startup-file block that records the last failed command, the
//! record's file format, `kor shell capture`, and the `kor fix` / `kor why` prompt and reply.
//!
//! The hooks record the command line, exit code, shell and directory everywhere. Output cannot be
//! read back from a terminal after the fact, so it is kept only when the command ran through
//! `kor shell capture -- <command>` (and, for `PowerShell`, the error record's message).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use kor_platform::NoWindow as _;
use serde::Serialize;

pub const BEGIN: &str = "# >>> kor shell integration >>>";
pub const END: &str = "# <<< kor shell integration <<<";
const MAGIC: &str = "kor-last-failure 1";
/// Output lines kept in a record.
pub const KEEP_LINES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellKind {
    pub const ALL: [Self; 4] = [Self::Bash, Self::Zsh, Self::Fish, Self::PowerShell];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().trim_end_matches(".exe") {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            "powershell" | "pwsh" => Some(Self::PowerShell),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    /// The user's shell from `$SHELL`, else `PowerShell` when its module path is set.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if let Some(sh) = env("SHELL")
            && let Some(k) = sh.rsplit(['/', '\\']).next().and_then(Self::parse)
        {
            return Some(k);
        }
        env("PSModulePath").map(|_| Self::PowerShell)
    }

    /// `PowerShell` on Windows, bash elsewhere.
    pub fn os_default() -> Self {
        if kor_platform::os() == kor_platform::Os::Windows {
            Self::PowerShell
        } else {
            Self::Bash
        }
    }

    /// Startup file the block goes into. `PowerShell` asks `pwsh`/`powershell` for `$PROFILE`
    /// (the Documents folder may be redirected), falling back to the default location.
    pub fn rc_path(self, home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
        let xdg = || {
            env("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map_or_else(|| home.join(".config"), PathBuf::from)
        };
        match self {
            Self::Bash => home.join(".bashrc"),
            Self::Zsh => env("ZDOTDIR")
                .filter(|v| !v.is_empty())
                .map_or_else(|| home.to_owned(), PathBuf::from)
                .join(".zshrc"),
            Self::Fish => xdg().join("fish").join("config.fish"),
            Self::PowerShell => powershell_profile().unwrap_or_else(|| {
                if kor_platform::os() == kor_platform::Os::Windows {
                    home.join("Documents")
                        .join("PowerShell")
                        .join("Microsoft.PowerShell_profile.ps1")
                } else {
                    xdg()
                        .join("powershell")
                        .join("Microsoft.PowerShell_profile.ps1")
                }
            }),
        }
    }

    /// Program and flag that run one command line in this shell (`kor fix`).
    pub fn runner(self) -> Vec<(&'static str, &'static [&'static str])> {
        match self {
            Self::Bash => vec![("bash", &["-c"])],
            Self::Zsh => vec![("zsh", &["-c"])],
            Self::Fish => vec![("fish", &["-c"])],
            Self::PowerShell => vec![
                ("pwsh", &["-NoProfile", "-Command"]),
                ("powershell", &["-NoProfile", "-Command"]),
            ],
        }
    }
}

fn powershell_profile() -> Option<PathBuf> {
    ["pwsh", "powershell"].iter().find_map(|exe| {
        let out = Command::new(kor_platform::exe(exe))
            .args(["-NoProfile", "-NonInteractive", "-Command", "$PROFILE"])
            .stdin(Stdio::null())
            .no_window()
            .output()
            .ok()?;
        let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !p.is_empty()).then(|| PathBuf::from(p))
    })
}

/// POSIX single quotes; Windows paths get forward slashes (Git Bash, MSYS and WSL accept them).
fn sh_quote(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn fish_quote(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'"))
}

fn ps_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "''"))
}

/// The whole block (markers included) that records failures into `record`, with the inline
/// suggestion code when `suggest` (`[shell] suggest`).
pub fn snippet(kind: ShellKind, record: &Path, suggest: bool) -> String {
    let body = match kind {
        ShellKind::Bash => format!(
            r#"__kor_last_status() {{
  local s=$? c
  if [ "$s" -ne 0 ]; then
    c=$(HISTTIMEFORMAT= builtin history 1 2>/dev/null | sed 's/^ *[0-9]* *//')
    c=${{c//$'\n'/ }}
    case "$c" in
      ""|kor|kor\ *) ;;
      *) {{ printf '{MAGIC}\nexit %s\nshell bash\ncwd %s\ncmd %s\n' "$s" "$PWD" "$c" > {f}; }} 2>/dev/null ;;
    esac
  fi
  return $s
}}
case ";$PROMPT_COMMAND;" in
  *";__kor_last_status;"*) ;;
  *) PROMPT_COMMAND="__kor_last_status${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}" ;;
esac
"#,
            f = sh_quote(record)
        ),
        ShellKind::Zsh => format!(
            r#"__kor_preexec() {{ __kor_cmd=$1 }}
__kor_precmd() {{
  local s=$?
  if [[ $s -ne 0 && -n $__kor_cmd && $__kor_cmd != kor && $__kor_cmd != 'kor '* ]]; then
    {{ printf '{MAGIC}\nexit %s\nshell zsh\ncwd %s\ncmd %s\n' "$s" "$PWD" "${{__kor_cmd//$'\n'/ }}" > {f}; }} 2>/dev/null
  fi
  __kor_cmd=
}}
autoload -Uz add-zsh-hook
add-zsh-hook preexec __kor_preexec
add-zsh-hook precmd __kor_precmd
"#,
            f = sh_quote(record)
        ),
        ShellKind::Fish => format!(
            r#"function __kor_postexec --on-event fish_postexec
    set -l s $status
    test $s -ne 0; or return
    set -l c (string join ' ' -- (string split \n -- $argv[1]))
    string match -q -r '^kor( |$)' -- "$c"; and return
    printf '{MAGIC}\nexit %s\nshell fish\ncwd %s\ncmd %s\n' $s $PWD "$c" > {f}
end
"#,
            f = fish_quote(record)
        ),
        ShellKind::PowerShell => format!(
            r#"if (-not (Test-Path variable:global:__KorPrompt)) {{ $global:__KorPrompt = $function:prompt }}
function global:prompt {{
    $korOk = $?
    $korCode = $global:LASTEXITCODE
    $korErr = if ($global:Error.Count) {{ $global:Error[0] }} else {{ $null }}
    $korNewErr = $null -ne $korErr -and -not [object]::ReferenceEquals($korErr, $global:__KorErr)
    $global:__KorErr = $korErr
    $korLast = Get-History -Count 1
    if ($korLast -and $korLast.Id -ne $global:__KorLastId) {{
        $global:__KorLastId = $korLast.Id
        $korLine = $korLast.CommandLine -replace '\r?\n', ' '
        if (-not $korOk -and $korLine -notmatch '^kor(\s|$)') {{
            $korExit = if ($korNewErr -or -not $korCode) {{ 1 }} else {{ $korCode }}
            $korOut = if ($korNewErr) {{ "$korErr" -split '\r?\n' }} else {{ @() }}
            $korRec = @('{MAGIC}', "exit $korExit", 'shell powershell', "cwd $PWD", "cmd $korLine", 'output') + $korOut
            try {{ Set-Content -LiteralPath {f} -Encoding utf8 -Value $korRec }} catch {{ }}
        }}
    }}
    if ($global:__KorPrompt) {{ & $global:__KorPrompt }} else {{ "PS $PWD> " }}
}}
"#,
            f = ps_quote(record)
        ),
    };
    let extra = if suggest {
        crate::shell_suggest::snippet(kind)
    } else {
        ""
    };
    format!("{BEGIN}\n{body}{extra}{END}\n")
}

fn block_range(rc: &str) -> Option<(usize, usize)> {
    let start = rc.find(BEGIN)?;
    let end_rel = rc[start..].find(END)?;
    let mut end = start + end_rel + END.len();
    if rc[end..].starts_with("\r\n") {
        end += 2;
    } else if rc[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}

pub fn has_block(rc: &str) -> bool {
    block_range(rc).is_some()
}

/// `rc` with Kor's block replaced (or appended), keeping the file's line endings.
pub fn with_block(rc: &str, block: &str) -> String {
    let block = if rc.contains("\r\n") {
        block.replace('\n', "\r\n")
    } else {
        block.to_owned()
    };
    match block_range(rc) {
        Some((s, e)) => format!("{}{block}{}", &rc[..s], &rc[e..]),
        None if rc.is_empty() => block,
        None => {
            let nl = if rc.contains("\r\n") { "\r\n" } else { "\n" };
            let gap = nl.repeat(if rc.ends_with('\n') { 1 } else { 2 });
            format!("{rc}{gap}{block}")
        }
    }
}

/// `rc` without Kor's block; `None` when it is not there.
pub fn without_block(rc: &str) -> Option<String> {
    let (s, e) = block_range(rc)?;
    Some(format!("{}{}", &rc[..s], &rc[e..]))
}

/// The last failed command, as written by the shell hooks or `kor shell capture`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub command: String,
    pub exit: i32,
    pub shell: String,
    pub cwd: String,
    pub output: Vec<String>,
}

/// Parse a record; `None` for anything that is not one (or has no command).
pub fn parse(text: &str) -> Option<Failure> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim_end() != MAGIC {
        return None;
    }
    let mut f = Failure {
        exit: 1,
        ..Failure::default()
    };
    for line in lines.by_ref() {
        if line.trim_end() == "output" {
            break;
        }
        let (k, v) = line.split_once(' ').unwrap_or((line, ""));
        match k {
            "exit" => f.exit = v.trim().parse().unwrap_or(1),
            "shell" => v.trim().clone_into(&mut f.shell),
            "cwd" => v.trim_end().clone_into(&mut f.cwd),
            "cmd" => v.trim().clone_into(&mut f.command),
            _ => {}
        }
    }
    let out: Vec<String> = lines.map(str::to_owned).collect();
    let skip = out.len().saturating_sub(KEEP_LINES);
    f.output = out.into_iter().skip(skip).collect();
    (!f.command.is_empty()).then_some(f)
}

pub fn render(f: &Failure) -> String {
    let one = |s: &str| s.replace(['\r', '\n'], " ");
    let mut s = format!(
        "{MAGIC}\nexit {}\nshell {}\ncwd {}\ncmd {}\n",
        f.exit,
        one(&f.shell),
        one(&f.cwd),
        one(&f.command)
    );
    if !f.output.is_empty() {
        s.push_str("output\n");
        for l in &f.output {
            s.push_str(l);
            s.push('\n');
        }
    }
    s
}

/// Run `program args` with output passed through, keeping the last [`KEEP_LINES`] lines.
/// Output is relayed line by line (no TTY: colors and progress bars may change).
///
/// # Errors
/// When the program cannot be started.
pub fn capture(program: &str, args: &[String]) -> std::io::Result<(i32, Vec<String>)> {
    capture_command(Command::new(program).no_window().args(args))
}

/// [`capture`] for a prepared command (directory, window flags); stdout/stderr are piped here.
///
/// # Errors
/// When the program cannot be started.
pub fn capture_command(cmd: &mut Command) -> std::io::Result<(i32, Vec<String>)> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let tail = Arc::new(Mutex::new(VecDeque::with_capacity(KEEP_LINES)));
    let pump = |r: Box<dyn std::io::Read + Send>, err: bool| {
        let tail = Arc::clone(&tail);
        std::thread::spawn(move || {
            let mut r = BufReader::new(r);
            let mut buf = Vec::new();
            while r.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
                let _ = if err {
                    std::io::stderr().write_all(&buf)
                } else {
                    std::io::stdout().write_all(&buf)
                };
                let line = String::from_utf8_lossy(&buf).trim_end().to_owned();
                let mut t = tail.lock().unwrap_or_else(PoisonError::into_inner);
                if t.len() == KEEP_LINES {
                    t.pop_front();
                }
                t.push_back(line);
                buf.clear();
            }
        })
    };
    let mut threads = Vec::new();
    if let Some(o) = child.stdout.take() {
        threads.push(pump(Box::new(o), false));
    }
    if let Some(e) = child.stderr.take() {
        threads.push(pump(Box::new(e), true));
    }
    let status = child.wait()?;
    for t in threads {
        let _ = t.join();
    }
    let lines = tail
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect();
    Ok((status.code().unwrap_or(1), lines))
}

/// Prompt for `kor fix` (`explain_only = false`) and `kor why`. The record is redacted first.
pub fn fix_prompt(f: &Failure, os: &str, explain_only: bool) -> String {
    let red = kor_secrets::Redactor::default();
    let output = if f.output.is_empty() {
        "(not captured)".to_owned()
    } else {
        f.output.join("\n")
    };
    let task = if explain_only {
        "Explain in 2-5 sentences why it failed. Reply with the explanation only."
    } else {
        "Explain in 2-4 sentences why it failed, then propose one corrected command for the same \
         shell. Reply exactly in this form:\nEXPLANATION: <text>\nCOMMAND: <one line, or NONE if \
         no command would fix it>"
    };
    let text = format!(
        "A command failed in the user's terminal.\nOS: {os}\nShell: {}\nDirectory: {}\nCommand: \
         {}\nExit code: {}\nLast output lines:\n{output}\n\n{task}",
        f.shell, f.cwd, f.command, f.exit
    );
    red.redact(&text).into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fix {
    pub explanation: String,
    pub command: Option<String>,
}

/// Read a `kor fix` reply: `EXPLANATION:` text and a `COMMAND:` line (`NONE` for no fix).
pub fn parse_fix(reply: &str) -> Fix {
    let mut command = None;
    let mut explanation = Vec::new();
    for line in reply.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            continue;
        }
        if let Some(rest) = strip_label(t, "COMMAND:") {
            let c = rest.trim().trim_matches('`').trim();
            if !c.is_empty() && !c.eq_ignore_ascii_case("none") {
                command = Some(c.to_owned());
            }
        } else if let Some(rest) = strip_label(t, "EXPLANATION:") {
            explanation.push(rest.trim().to_owned());
        } else if command.is_none() {
            explanation.push(t.to_owned());
        }
    }
    Fix {
        explanation: explanation.join("\n").trim().to_owned(),
        command,
    }
}

fn strip_label<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let line = line.trim_start_matches(['*', '#', ' ']);
    let head = line.get(..label.len())?;
    head.eq_ignore_ascii_case(label)
        .then(|| line[label.len()..].trim_start_matches(['*', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_have_markers_and_quote_the_record_path() {
        let p = Path::new("/home/a b/it's/last-failure.txt");
        for k in ShellKind::ALL {
            let s = snippet(k, p, false);
            assert!(s.starts_with(BEGIN) && s.trim_end().ends_with(END), "{k:?}");
            assert!(s.contains(MAGIC), "{k:?}");
        }
        assert!(
            snippet(ShellKind::Bash, p, false).contains(r"'/home/a b/it'\''s/last-failure.txt'")
        );
        assert!(snippet(ShellKind::Zsh, p, false).contains("add-zsh-hook precmd __kor_precmd"));
        assert!(snippet(ShellKind::Fish, p, false).contains(r"'/home/a b/it\'s/last-failure.txt'"));
        let ps = snippet(
            ShellKind::PowerShell,
            Path::new(r"C:\Users\o'n\kor\f.txt"),
            false,
        );
        assert!(ps.contains(r"-LiteralPath 'C:\Users\o''n\kor\f.txt'"));
        assert!(snippet(ShellKind::Bash, Path::new(r"C:\x\f"), false).contains("'C:/x/f'"));
        let on = snippet(ShellKind::Zsh, p, true);
        assert!(on.contains("kor shell suggest") && on.trim_end().ends_with(END));
        assert!(!snippet(ShellKind::Zsh, p, false).contains("kor shell suggest"));
    }

    #[test]
    fn block_install_is_idempotent_and_removable() {
        let block = snippet(ShellKind::Bash, Path::new("/f"), false);
        let rc = "export A=1";
        let once = with_block(rc, &block);
        assert!(once.starts_with("export A=1\n\n# >>>"));
        assert_eq!(with_block(&once, &block), once);
        assert_eq!(without_block(&once).unwrap(), "export A=1\n\n");
        assert!(without_block(rc).is_none());
        let crlf = with_block("a\r\n", &block);
        assert!(crlf.contains("\r\n# >>> kor") && !crlf.replace("\r\n", "").contains('\n'));
        assert_eq!(without_block(&crlf).unwrap(), "a\r\n\r\n");
        let updated = with_block(&once, &snippet(ShellKind::Bash, Path::new("/g"), false));
        assert!(updated.contains("'/g'") && !updated.contains("'/f'"));
        assert_eq!(with_block("", &block), block);
    }

    #[test]
    fn records_round_trip_and_bad_ones_are_ignored() {
        let f = Failure {
            command: "cargo biuld".into(),
            exit: 101,
            shell: "bash".into(),
            cwd: "/w".into(),
            output: vec!["error: no such command".into(), String::new()],
        };
        assert_eq!(parse(&render(&f)).unwrap(), f);
        let ps = "\u{feff}kor-last-failure 1\r\nexit 1\r\nshell powershell\r\ncwd C:\\w\r\ncmd \
                  Get-Itm x\r\noutput\r\nThe term 'Get-Itm' is not recognized\r\n";
        let p = parse(ps).unwrap();
        assert_eq!(p.command, "Get-Itm x");
        assert_eq!(p.cwd, "C:\\w");
        assert_eq!(p.output, vec!["The term 'Get-Itm' is not recognized"]);
        let long = format!("{MAGIC}\ncmd x\noutput\n{}", "l\n".repeat(100));
        assert_eq!(parse(&long).unwrap().output.len(), KEEP_LINES);
        assert!(parse("hello").is_none());
        assert!(parse(&format!("{MAGIC}\nexit 2\n")).is_none());
    }

    #[test]
    fn shells_are_detected() {
        let env = |k: &str| (k == "SHELL").then(|| "/usr/bin/zsh".to_owned());
        assert_eq!(ShellKind::detect(env), Some(ShellKind::Zsh));
        let ps = |k: &str| (k == "PSModulePath").then(String::new);
        assert_eq!(ShellKind::detect(ps), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::detect(|_| None), None);
        assert_eq!(ShellKind::parse("PWSH.EXE"), Some(ShellKind::PowerShell));
        let home = Path::new("/h");
        let none = |_: &str| None;
        assert_eq!(ShellKind::Bash.rc_path(home, none), home.join(".bashrc"));
        let zd = |k: &str| (k == "ZDOTDIR").then(|| "/z".to_owned());
        assert_eq!(
            ShellKind::Zsh.rc_path(home, zd),
            Path::new("/z").join(".zshrc")
        );
        assert_eq!(
            ShellKind::Fish.rc_path(home, none),
            home.join(".config").join("fish").join("config.fish")
        );
    }

    #[test]
    fn fix_prompt_redacts_and_reply_parses() {
        let f = Failure {
            command: "curl -H 'Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456' x".into(),
            exit: 6,
            shell: "bash".into(),
            ..Failure::default()
        };
        let p = fix_prompt(&f, "linux", false);
        assert!(!p.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert!(p.contains("COMMAND:") && p.contains("(not captured)"));
        assert!(!fix_prompt(&f, "linux", true).contains("COMMAND:"));

        let r = parse_fix("EXPLANATION: Typo in the subcommand.\nCOMMAND: `cargo build`\n");
        assert_eq!(r.explanation, "Typo in the subcommand.");
        assert_eq!(r.command.as_deref(), Some("cargo build"));
        let r = parse_fix("**Explanation:** Network is down.\n**Command:** NONE");
        assert_eq!(r.explanation, "Network is down.");
        assert_eq!(r.command, None);
        assert_eq!(parse_fix("Just text.").explanation, "Just text.");
    }

    #[test]
    fn capture_keeps_output_and_exit_code() {
        let (code, lines) = capture(&kor_platform::exe("cargo"), &["--bogus-flag".into()]).unwrap();
        assert_ne!(code, 0);
        assert!(!lines.is_empty());
        assert!(capture("kor-no-such-program-xyz", &[]).is_err());
    }
}
