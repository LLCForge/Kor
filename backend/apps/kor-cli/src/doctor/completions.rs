//! Completion scripts and their installation, shared by `kor completions` and `kor doctor --fix`
//! (§26.1, ADR 0050).

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write as _};

use kor_i18n::{Localizer, args};
use kor_platform::completions::{self as comp, Shell};

/// The completion script of `cmd` (the `kor` command tree) for `shell`.
pub fn script(cmd: clap::Command, shell: Shell) -> String {
    use clap_complete::Shell as C;
    let mut cmd = cmd;
    let mut out = Vec::new();
    let gen_shell = match shell {
        Shell::Bash => C::Bash,
        Shell::Zsh => C::Zsh,
        Shell::Fish => C::Fish,
        Shell::PowerShell => C::PowerShell,
        Shell::Elvish => C::Elvish,
        Shell::Nushell => return nushell(&mut cmd),
    };
    clap_complete::generate(gen_shell, &mut cmd, "kor", &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// Nushell `extern` definitions for every command (flags, values, positionals).
fn nushell(cmd: &mut clap::Command) -> String {
    cmd.build();
    let mut defs = String::new();
    let mut externs = String::new();
    walk(cmd, "kor", &mut defs, &mut externs);
    format!(
        "# kor completions for nushell (kor completions nushell)\nmodule kor-completions {{\n{defs}{externs}}}\n\nexport use kor-completions *\n"
    )
}

fn walk(cmd: &clap::Command, path: &str, defs: &mut String, externs: &mut String) {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut params = Vec::new();
    for a in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        let id = a.get_id().as_str();
        let takes =
            a.get_num_args().is_some_and(|n| n.takes_values()) && a.get_action().takes_values();
        let values: Vec<String> = a
            .get_possible_values()
            .iter()
            .map(|v| v.get_name().to_owned())
            .collect();
        let ty = if values.is_empty() {
            "string".to_owned()
        } else {
            let name = format!("nu-complete {path} {id}");
            let list: Vec<String> = values.iter().map(|v| format!("\"{}\"", quote(v))).collect();
            let _ = writeln!(
                defs,
                "  def \"{}\" [] {{ [{}] }}",
                quote(&name),
                list.join(" ")
            );
            format!("string@\"{}\"", quote(&name))
        };
        let help = a
            .get_help()
            .map(|h| format!("  # {}", h.to_string().replace('\n', " ")))
            .unwrap_or_default();
        if let Some(long) = a.get_long() {
            let short = a.get_short().map(|s| format!("(-{s})")).unwrap_or_default();
            let kind = if takes {
                format!(": {ty}")
            } else {
                String::new()
            };
            params.push(format!("    --{long}{short}{kind}{help}"));
        } else if a.is_positional() {
            let many = a.get_num_args().is_some_and(|n| n.max_values() > 1);
            let opt = if a.is_required_set() { "" } else { "?" };
            params.push(if many {
                format!("    ...{id}: {ty}{help}")
            } else {
                format!("    {id}{opt}: {ty}{help}")
            });
        }
    }
    let _ = writeln!(
        externs,
        "  export extern \"{}\" [\n{}\n  ]",
        quote(path),
        params.join("\n")
    );
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        walk(sub, &format!("{path} {}", sub.get_name()), defs, externs);
    }
}

/// Supported shells installed on this machine (their program is on PATH).
pub fn detected() -> Vec<Shell> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let dirs: Vec<_> = std::env::split_paths(&path).collect();
    Shell::ALL
        .into_iter()
        .filter(|s| {
            let names: &[&str] = if *s == Shell::PowerShell {
                &["pwsh", "powershell"]
            } else {
                &[s.program()]
            };
            names.iter().any(|n| {
                dirs.iter()
                    .any(|d| d.join(kor_platform::exe(n)).is_file() || d.join(n).is_file())
            })
        })
        .collect()
}

/// `y` from the user (stdin) to `question`; `false` when stdin is not a terminal.
pub fn confirm(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(
        line.trim(),
        "y" | "Y" | "e" | "E" | "o" | "O" | "yes" | "evet" | "oui"
    )
}

/// Install completions for `shell`, asking before a profile change unless `yes`.
pub fn install_one(l: &Localizer, cmd: clap::Command, shell: Shell, yes: bool) -> bool {
    let Some(d) = comp::Dirs::current() else {
        eprintln!("{}", l.t("completions-no-home"));
        return false;
    };
    let plan = comp::plan(shell, &d);
    if let Some((profile, line)) = &plan.profile
        && !yes
        && !confirm(&l.t_args(
            "completions-ask-profile",
            &args!("shell" => shell.name(), "profile" => profile.display().to_string(), "line" => line.clone()),
        ))
    {
        println!("{}", l.t_args("completions-skipped", &args!("shell" => shell.name())));
        return false;
    }
    match comp::install(&plan, &script(cmd, shell)) {
        Ok(()) => {
            println!(
                "{}",
                l.t_args(
                    "completions-installed",
                    &args!("shell" => shell.name(), "path" => plan.file.display().to_string())
                )
            );
            true
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args(
                    "completions-failed",
                    &args!("shell" => shell.name(), "error" => e.to_string())
                )
            );
            false
        }
    }
}
