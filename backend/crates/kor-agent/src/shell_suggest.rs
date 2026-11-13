//! Inline command suggestions (§15.4, `[shell] suggest`): the rest of the command line the user
//! is typing. Shell history and the project's own commands answer first (no model, no network);
//! only when they have nothing is the configured model asked, after a short pause and within a
//! time budget. A model that is not on this computer gets the typed line only, never history.

use std::future::Future;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::shellint::ShellKind;

/// History read from the end of the file (large histories stay fast).
const HISTORY_TAIL_BYTES: u64 = 256 * 1024;
/// Project files larger than this are not read.
const MAX_PROJECT_FILE: u64 = 1024 * 1024;
/// History lines given to a local model.
const PROMPT_HISTORY: usize = 20;
/// Longest suggestion kept.
const MAX_SUGGESTION: usize = 300;

/// What one suggestion is made from.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub shell: String,
    pub os: String,
    pub line: String,
    /// Oldest first.
    pub history: Vec<String>,
    pub project: Vec<String>,
}

/// The shell's own history file when the integration did not pass one (`--history`).
pub fn default_history(
    kind: ShellKind,
    home: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> PathBuf {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    match kind {
        ShellKind::Bash => var("HISTFILE").unwrap_or_else(|| home.join(".bash_history")),
        ShellKind::Zsh => var("HISTFILE").unwrap_or_else(|| home.join(".zsh_history")),
        ShellKind::Fish => var("XDG_DATA_HOME")
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("fish")
            .join("fish_history"),
        ShellKind::PowerShell => var("APPDATA")
            .map_or_else(
                || home.join(".local").join("share").join("powershell"),
                |a| a.join("Microsoft").join("Windows").join("PowerShell"),
            )
            .join("PSReadLine")
            .join("ConsoleHost_history.txt"),
    }
}

/// Commands from the end of a history file, oldest first; a missing file gives none.
pub fn read_history(kind: ShellKind, path: &Path) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(HISTORY_TAIL_BYTES);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    // The first line may be cut in the middle when only the tail was read.
    let text = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    parse_history(kind, text)
}

/// History file text to commands, oldest first. Multi-line entries are skipped.
pub fn parse_history(kind: ShellKind, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let cmd = match kind {
            ShellKind::Bash if line.starts_with('#') => continue,
            // Extended history: `: 1700000000:0;command`.
            ShellKind::Zsh if line.starts_with(": ") => {
                line.split_once(';').map_or(line, |(_, c)| c)
            }
            ShellKind::Fish => match line.strip_prefix("- cmd: ") {
                Some(c) if !c.contains("\\n") => c,
                _ => continue,
            },
            _ => line,
        };
        let cmd = cmd.trim();
        if cmd.is_empty() || cmd.ends_with('\\') || cmd.ends_with('`') {
            continue;
        }
        let cmd = if kind == ShellKind::Fish {
            cmd.replace("\\\\", "\\")
        } else {
            cmd.to_owned()
        };
        out.push(cmd);
    }
    out
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_PROJECT_FILE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The project's own commands in `dir`: package.json scripts, Makefile targets, cargo.
pub fn project_commands(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(text) = read_small(&dir.join("package.json"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(scripts) = v.get("scripts").and_then(serde_json::Value::as_object)
    {
        let runner = [
            ("pnpm-lock.yaml", "pnpm run"),
            ("yarn.lock", "yarn"),
            ("bun.lock", "bun run"),
            ("bun.lockb", "bun run"),
        ]
        .into_iter()
        .find(|(lock, _)| dir.join(lock).is_file())
        .map_or("npm run", |(_, r)| r);
        out.extend(scripts.keys().map(|k| format!("{runner} {k}")));
    }
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        let Some(text) = read_small(&dir.join(name)) else {
            continue;
        };
        for line in text.lines() {
            let Some((target, rest)) = line.split_once(':') else {
                continue;
            };
            let ok = !target.is_empty()
                && !target.starts_with('.')
                && !rest.starts_with('=')
                && target
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c));
            if ok {
                out.push(format!("make {target}"));
            }
        }
        break;
    }
    if dir.join("Cargo.toml").is_file() {
        out.extend(
            ["build", "test", "run", "check", "clippy", "fmt"].map(|c| format!("cargo {c}")),
        );
    }
    out
}

/// The rest of `full` after `line`, when `full` continues what was typed.
fn rest_of(line: &str, full: &str) -> Option<String> {
    let rest = full.strip_prefix(line)?;
    (!rest.is_empty() && !rest.contains(['\n', '\r']) && rest.len() <= MAX_SUGGESTION)
        .then(|| rest.to_owned())
}

/// Newest history entry, then the first project command, that continues the typed line.
pub fn instant(ctx: &Context) -> Option<String> {
    if ctx.line.trim().is_empty() {
        return None;
    }
    ctx.history
        .iter()
        .rev()
        .chain(&ctx.project)
        .find_map(|c| rest_of(&ctx.line, c))
}

/// Prompt for the model. History goes only to a model on this computer (`local`), redacted.
pub fn prompt(ctx: &Context, local: bool) -> String {
    let red = kor_secrets::Redactor::default();
    let mut s = format!(
        "Complete the command line a user is typing in {} on {}.\n",
        ctx.shell, ctx.os
    );
    if !ctx.project.is_empty() {
        s.push_str("Commands this project defines:\n");
        for c in ctx.project.iter().take(30) {
            s.push_str(c);
            s.push('\n');
        }
    }
    if local && !ctx.history.is_empty() {
        s.push_str("Recent commands, oldest first:\n");
        let skip = ctx.history.len().saturating_sub(PROMPT_HISTORY);
        for c in &ctx.history[skip..] {
            s.push_str(c);
            s.push('\n');
        }
    }
    s.push_str("Typed so far: ");
    s.push_str(&ctx.line);
    s.push_str("\nReply with the whole completed command on one line and nothing else.");
    red.redact(&s).into_owned()
}

/// The suggestion in a model reply: its first command line, when it continues the typed line.
pub fn from_reply(line: &str, reply: &str) -> Option<String> {
    let first = reply
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("```"))?;
    let cmd = first.trim_matches('`').trim();
    let cmd = ["$ ", "> ", "PS> "]
        .iter()
        .find_map(|p| cmd.strip_prefix(p))
        .unwrap_or(cmd);
    rest_of(line, cmd)
}

/// History and project first; otherwise `model` (`local` = on this computer) after `pause`,
/// abandoned when it takes longer than `budget`.
pub async fn suggest<F, Fut>(
    ctx: &Context,
    model: Option<(F, bool)>,
    pause: Duration,
    budget: Duration,
) -> Option<String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    if ctx.line.trim().is_empty() {
        return None;
    }
    if let Some(s) = instant(ctx) {
        return Some(s);
    }
    let (ask, local) = model?;
    // A new keystroke kills this process during the pause, so a fast typist never reaches the model.
    tokio::time::sleep(pause).await;
    let reply = tokio::time::timeout(budget, ask(prompt(ctx, local)))
        .await
        .ok()?
        .ok()?;
    from_reply(&ctx.line, &reply)
}

/// Shell code for the `kor shell install` block that shows suggestions (`[shell] suggest`).
/// zsh draws them in grey as you type (→ accepts). bash, fish and `PowerShell` have no
/// script interface for grey inline text: Alt+K fills in Kor's suggestion (their own history
/// suggestions stay as they are).
pub fn snippet(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Zsh => ZSH,
        ShellKind::Bash => BASH,
        ShellKind::Fish => FISH,
        ShellKind::PowerShell => POWERSHELL,
    }
}

const ZSH: &str = r#"zmodload zsh/system 2>/dev/null
autoload -Uz add-zle-hook-widget
typeset -g __kor_sfd=0 __kor_spid=0
__kor_suggest_clear() { POSTDISPLAY=; region_highlight=("${(@)region_highlight:#*fg=8}") }
__kor_suggest_stop() {
  if (( __kor_sfd )); then zle -F $__kor_sfd 2>/dev/null; exec {__kor_sfd}<&-; __kor_sfd=0; fi
  if (( __kor_spid )); then kill -TERM $__kor_spid 2>/dev/null; __kor_spid=0; fi
}
__kor_suggest_done() {
  local fd=$1 s
  read -r -u $fd s
  zle -F $fd; exec {fd}<&-; __kor_sfd=0; __kor_spid=0
  [[ -n $s && $CURSOR -eq ${#BUFFER} ]] || return
  __kor_suggest_clear; POSTDISPLAY=$s
  region_highlight+=("${#BUFFER} $(( ${#BUFFER} + ${#s} )) fg=8")
  zle -R
}
__kor_suggest_ask() {
  __kor_suggest_stop; __kor_suggest_clear
  [[ -n ${BUFFER// } && $CURSOR -eq ${#BUFFER} && ${#BUFFER} -lt 300 ]] || return
  exec {__kor_sfd}< <(
    echo $sysparams[pid]
    exec kor shell suggest --shell zsh --history "${HISTFILE:-$HOME/.zsh_history}" -- "$BUFFER" 2>/dev/null
  )
  command true
  read -r -u $__kor_sfd __kor_spid
  zle -F -w $__kor_sfd __kor_suggest_done
}
__kor_self_insert() { zle .self-insert; __kor_suggest_ask }
__kor_backward_delete() { zle .backward-delete-char; __kor_suggest_ask }
__kor_accept_suggestion() {
  if [[ -n $POSTDISPLAY && $CURSOR -eq ${#BUFFER} ]]; then
    BUFFER+=$POSTDISPLAY; CURSOR=${#BUFFER}; __kor_suggest_clear
  else
    zle .forward-char
  fi
}
__kor_line_done() { __kor_suggest_stop; __kor_suggest_clear }
zle -N self-insert __kor_self_insert
zle -N backward-delete-char __kor_backward_delete
zle -N __kor_accept_suggestion
zle -N __kor_suggest_done
add-zle-hook-widget line-finish __kor_line_done
bindkey '^[[C' __kor_accept_suggestion
bindkey '^[OC' __kor_accept_suggestion
"#;

const BASH: &str = r#"__kor_suggest() {
  [ "$READLINE_POINT" -eq "${#READLINE_LINE}" ] || return
  local s
  s=$(kor shell suggest --shell bash --history "${HISTFILE:-$HOME/.bash_history}" -- "$READLINE_LINE" 2>/dev/null)
  [ -n "$s" ] || return
  READLINE_LINE+=$s
  READLINE_POINT=${#READLINE_LINE}
}
[[ $- == *i* ]] && bind -x '"\ek": __kor_suggest'
"#;

const FISH: &str = r#"function __kor_suggest
    set -l line (commandline)
    test (commandline -C) -eq (string length -- "$line"); or return
    set -l s (kor shell suggest --shell fish -- "$line" 2>/dev/null)
    test -n "$s"; and commandline -i -- "$s"
end
status is-interactive; and bind \ek __kor_suggest
"#;

const POWERSHELL: &str = r#"if (Get-Module PSReadLine) {
    Set-PSReadLineKeyHandler -Chord 'Alt+k' -BriefDescription KorSuggest -ScriptBlock {
        $korLine = $null; $korCursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$korLine, [ref]$korCursor)
        if ($korCursor -ne $korLine.Length) { return }
        $korHist = (Get-PSReadLineOption).HistorySavePath
        $korS = kor shell suggest --shell powershell --history "$korHist" -- "$korLine" 2>$null
        if ($korS) { [Microsoft.PowerShell.PSConsoleReadLine]::Insert("$korS") }
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(line: &str) -> Context {
        Context {
            shell: "bash".into(),
            os: "linux".into(),
            line: line.into(),
            history: vec![
                "git status".into(),
                "git commit -m 'old'".into(),
                "git checkout main".into(),
            ],
            project: vec!["npm run build".into(), "npm run test".into()],
        }
    }

    #[test]
    fn histories_parse_for_every_shell() {
        let bash = "#1700000000\nls -la\nmake \\\n\ngit push\n";
        assert_eq!(parse_history(ShellKind::Bash, bash), ["ls -la", "git push"]);
        let zsh = ": 1700000000:0;cargo test\nplain cmd\n";
        assert_eq!(
            parse_history(ShellKind::Zsh, zsh),
            ["cargo test", "plain cmd"]
        );
        let fish = "- cmd: echo a\\\\b\n  when: 1700000000\n- cmd: echo x\\ny\n- cmd: ls\n";
        assert_eq!(parse_history(ShellKind::Fish, fish), ["echo a\\b", "ls"]);
        let ps = "Get-ChildItem\r\nWrite-Host `\r\nGet-Process\r\n";
        assert_eq!(
            parse_history(ShellKind::PowerShell, ps),
            ["Get-ChildItem", "Get-Process"]
        );
        let home = Path::new("/h");
        assert_eq!(
            default_history(ShellKind::Fish, home, |_| None),
            home.join(".local/share/fish/fish_history")
        );
        let hf = |k: &str| (k == "HISTFILE").then(|| "/x/h".to_owned());
        assert_eq!(
            default_history(ShellKind::Zsh, home, hf),
            PathBuf::from("/x/h")
        );
    }

    #[test]
    fn only_the_tail_of_a_large_history_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h");
        let body = "old command\n".repeat(30_000) + "newest one\n";
        std::fs::write(&p, &body).unwrap();
        let h = read_history(ShellKind::Bash, &p);
        assert_eq!(h.last().map(String::as_str), Some("newest one"));
        assert!(h.len() < 30_000 && h.iter().all(|c| c == "old command" || c == "newest one"));
        assert!(read_history(ShellKind::Bash, &dir.path().join("missing")).is_empty());
    }

    #[test]
    fn project_commands_come_from_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        std::fs::write(d.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(
            d.join("Makefile"),
            "CC := gcc\n.PHONY: all\nall: build\nbuild:\n\tcc x.c\nX = 1\n",
        )
        .unwrap();
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        let c = project_commands(d);
        assert!(c.contains(&"pnpm run dev".to_owned()), "{c:?}");
        assert!(c.contains(&"make all".to_owned()) && c.contains(&"make build".to_owned()));
        assert!(!c.iter().any(|x| x.contains("PHONY") || x.contains("CC")));
        assert!(c.contains(&"cargo clippy".to_owned()));
    }

    #[test]
    fn history_wins_newest_first_then_project() {
        assert_eq!(instant(&ctx("git c")).as_deref(), Some("heckout main"));
        assert_eq!(instant(&ctx("npm run t")).as_deref(), Some("est"));
        assert_eq!(instant(&ctx("git status")), None, "nothing to add");
        assert_eq!(instant(&ctx("  ")), None);
    }

    #[test]
    fn replies_are_cleaned_and_must_continue_the_line() {
        assert_eq!(
            from_reply("docker ru", "```sh\n$ docker run --rm -it alpine\n```").as_deref(),
            Some("n --rm -it alpine")
        );
        assert_eq!(from_reply("ls", "`ls -la`").as_deref(), Some(" -la"));
        assert_eq!(from_reply("ls", "rm -rf /"), None, "a different command");
        assert_eq!(from_reply("ls", ""), None);
    }

    #[test]
    fn remote_models_never_see_history() {
        let c = ctx("doc");
        let local = prompt(&c, true);
        let remote = prompt(&c, false);
        assert!(local.contains("git checkout main") && local.contains("Typed so far: doc"));
        assert!(!remote.contains("git checkout main") && remote.contains("npm run build"));
    }

    #[tokio::test]
    async fn the_model_is_asked_only_when_history_has_nothing() {
        let z = Duration::ZERO;
        let budget = Duration::from_millis(500);
        let never = |_: String| async { Err::<String, _>("must not be called".to_owned()) };
        assert_eq!(
            suggest(&ctx("git c"), Some((never, true)), z, budget)
                .await
                .as_deref(),
            Some("heckout main")
        );
        let mock = |p: String| async move {
            assert!(p.contains("Typed so far: docker ru"));
            Ok("docker run --rm hello-world".to_owned())
        };
        assert_eq!(
            suggest(&ctx("docker ru"), Some((mock, false)), z, budget)
                .await
                .as_deref(),
            Some("n --rm hello-world")
        );
        let slow = |_: String| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("docker run".to_owned())
        };
        let t = std::time::Instant::now();
        assert_eq!(
            suggest(&ctx("docker ru"), Some((slow, true)), z, budget).await,
            None
        );
        assert!(t.elapsed() < Duration::from_secs(2), "budget enforced");
        let no_model = Some((never, true)).filter(|_| false);
        assert_eq!(suggest(&ctx("docker ru"), no_model, z, budget).await, None);
    }

    #[test]
    fn snippets_call_the_suggest_command() {
        for k in ShellKind::ALL {
            assert!(
                snippet(k).contains(&format!("kor shell suggest --shell {}", k.name())),
                "{k:?}"
            );
        }
    }
}
