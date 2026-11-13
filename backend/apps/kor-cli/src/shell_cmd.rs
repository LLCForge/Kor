//! `kor shell install|uninstall|status|capture`, `kor fix` and `kor why` (§15.4). Logic lives in
//! `kor_agent::shellint` and `kor_runtime::shell` (shared with the desktop `shell.*` bridge); this
//! module asks, prints and runs.

use std::io::{IsTerminal as _, Write as _};

use clap::Subcommand;
use kor_agent::shellint::{self, Failure, ShellKind};
use kor_i18n::{Localizer, args};
use kor_runtime::shell::{self as core, ShellError};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum ShellCmd {
    /// Add Kor's block to the shell's startup file (bash, zsh, fish, powershell).
    Install {
        shell: Option<String>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Remove Kor's block from the shell's startup file.
    Uninstall {
        shell: Option<String>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Where the integration is installed and the last recorded failure.
    Status,
    /// Run a command and keep its last output lines if it fails (`kor shell capture -- make`).
    Capture {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    /// The rest of a command line being typed (`[shell] suggest`); called by the shell integration.
    #[command(hide = true)]
    Suggest {
        #[arg(long)]
        shell: Option<String>,
        /// The shell's history file.
        #[arg(long)]
        history: Option<std::path::PathBuf>,
        #[arg(allow_hyphen_values = true, default_value = "")]
        line: String,
    },
}

fn err(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

/// `[y/N]` question; no answer, or no terminal without `--yes`, means no.
pub(crate) fn confirm(l: &Localizer, question: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("{}", l.t("flow-needs-yes"));
        return false;
    }
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    let a = line.trim().to_lowercase();
    let ok = ["y", "yes", "e", "evet", "o", "oui"].contains(&a.as_str());
    if !ok {
        println!("{}", l.t("flow-cancelled"));
    }
    ok
}

fn kind(l: &Localizer, shell: Option<&str>) -> Option<ShellKind> {
    let k = core::kind(shell);
    if k.is_none() {
        eprintln!("{}", l.t("shell-unknown"));
    }
    k
}

pub fn run(ctx: &Ctx, cmd: ShellCmd) -> bool {
    let l = &ctx.l;
    match cmd {
        ShellCmd::Install { shell, yes } => install(l, shell.as_deref(), yes),
        ShellCmd::Uninstall { shell, yes } => uninstall(l, shell.as_deref(), yes),
        ShellCmd::Status => status(l),
        ShellCmd::Capture { command } => capture(l, &command),
        ShellCmd::Suggest {
            shell,
            history,
            line,
        } => suggest(l, shell.as_deref(), history.as_deref(), &line),
    }
}

/// Plan error as the CLI reports it; `NoHome` stays silent as before.
fn plan_err(l: &Localizer, e: ShellError) -> bool {
    match e {
        ShellError::NoHome => false,
        ShellError::RcUnknown(k) => {
            err(l, l.t_args("shell-rc-unknown", &args!("shell" => k.name())))
        }
        ShellError::Io(p, e) => err(l, format!("{}: {e}", kor_tools::display_path(&p))),
    }
}

fn install(l: &Localizer, shell: Option<&str>, yes: bool) -> bool {
    let Some(k) = kind(l, shell) else {
        return false;
    };
    let plan = match core::plan_install(k) {
        Ok(p) => p,
        Err(e) => return plan_err(l, e),
    };
    let shown = kor_tools::display_path(&plan.rc);
    if plan.new.is_none() {
        println!("{}", l.t_args("shell-already", &args!("path" => shown)));
        return true;
    }
    println!(
        "{}

{}",
        l.t_args("shell-will-change", &args!("path" => shown.clone())),
        plan.block
    );
    if !confirm(l, &l.t("flow-confirm"), yes) {
        return false;
    }
    if let Err(e) = core::apply(&plan) {
        return err(l, format!("{shown}: {e}"));
    }
    println!(
        "{}",
        l.t_args(
            "shell-installed",
            &args!("shell" => k.name(), "path" => shown)
        )
    );
    println!("{}", l.t("shell-limits"));
    true
}

fn uninstall(l: &Localizer, shell: Option<&str>, yes: bool) -> bool {
    let Some(k) = kind(l, shell) else {
        return false;
    };
    let plan = match core::plan_uninstall(k) {
        Ok(p) => p,
        Err(e) => return plan_err(l, e),
    };
    let shown = kor_tools::display_path(&plan.rc);
    if plan.new.is_none() {
        println!(
            "{}",
            l.t_args("shell-not-installed", &args!("path" => shown))
        );
        return true;
    }
    println!(
        "{}",
        l.t_args("shell-will-remove", &args!("path" => shown.clone()))
    );
    if !confirm(l, &l.t("flow-confirm"), yes) {
        return false;
    }
    if let Err(e) = core::apply(&plan) {
        return err(l, format!("{shown}: {e}"));
    }
    println!("{}", l.t_args("shell-removed", &args!("path" => shown)));
    true
}

fn status(l: &Localizer) -> bool {
    for (k, rc, on) in core::status() {
        println!(
            "{}",
            l.t_args(
                "shell-status-line",
                &args!(
                    "shell" => k.name(),
                    "path" => kor_tools::display_path(&rc),
                    "state" => l.t(if on {
                        "shell-state-on"
                    } else if core::present(k) {
                        "shell-state-off"
                    } else {
                        "shell-state-absent"
                    })
                )
            )
        );
    }
    if let Some(p) = core::record_path() {
        println!(
            "{}",
            l.t_args(
                "shell-record-path",
                &args!("path" => kor_tools::display_path(&p))
            )
        );
    }
    match core::load_failure() {
        Some(f) => println!("{}", last_line(l, &f)),
        None => println!("{}", l.t("shell-no-record")),
    }
    println!("{}", l.t("shell-limits"));
    true
}

fn last_line(l: &Localizer, f: &Failure) -> String {
    l.t_args(
        "shell-last",
        &args!("command" => f.command.clone(), "code" => f.exit, "shell" => f.shell.clone()),
    )
}

fn capture(l: &Localizer, command: &[String]) -> bool {
    let Some((program, rest)) = command.split_first() else {
        return false;
    };
    let (code, output) = match shellint::capture(program, rest) {
        Ok(r) => r,
        Err(e) => return err(l, format!("{program}: {e}")),
    };
    if code != 0 {
        let cwd = std::env::current_dir().unwrap_or_default();
        core::record_failure(command, code, &cwd, output);
    }
    code == 0
}

/// Prints the suggestion (nothing when there is none); quiet on errors, the shell is mid-typing.
fn suggest(
    l: &Localizer,
    shell: Option<&str>,
    history: Option<&std::path::Path>,
    line: &str,
) -> bool {
    let Some(k) = core::kind(shell) else {
        return false;
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let Ok(rt) = crate::providers::runtime() else {
        return false;
    };
    if let Some(s) = rt.block_on(core::suggest(l, k, line, history, &cwd)) {
        println!("{s}");
    }
    true
}

/// `kor fix` (`explain_only = false`) and `kor why`.
pub fn fix(ctx: &Ctx, model: Option<&str>, explain_only: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(f) = core::load_failure() else {
        println!("{}", l.t("shell-no-record"));
        return Ok(false);
    };
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => return Ok(err(l, e)),
    };
    let (provider, model) = match kor_runtime::oneshot_provider(l, loaded, model) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Ok(false);
        }
    };
    println!("{}", last_line(l, &f));
    eprintln!(
        "{}",
        l.t_args(
            "flow-sending",
            &args!("provider" => provider.manifest().name.clone())
        )
    );
    let prompt = shellint::fix_prompt(&f, std::env::consts::OS, explain_only);
    let reply = crate::providers::runtime()?
        .block_on(kor_agent::gitflow::ask(&provider, &model, prompt, 1024));
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            return Ok(err(
                l,
                l.t_args("flow-model-failed", &args!("error" => e.to_string())),
            ));
        }
    };
    if explain_only {
        println!("\n{}", reply.trim());
        return Ok(true);
    }
    let fix = shellint::parse_fix(&reply);
    println!("\n{}", fix.explanation);
    let Some(cmd) = fix.command else {
        println!("{}", l.t("fix-no-suggestion"));
        return Ok(true);
    };
    println!(
        "\n{}",
        l.t_args("fix-suggested", &args!("command" => cmd.clone()))
    );
    // Explicit confirmation only: no `--yes` for running a model-written command.
    if !confirm(l, &l.t("fix-run-confirm"), false) {
        return Ok(false);
    }
    match core::run_fix(&f, &cmd, std::process::Command::status) {
        Ok(s) => Ok(s.success()),
        Err(core::RunError::Io(e)) => Ok(err(l, e)),
        Err(core::RunError::ShellMissing(shell)) => Ok(err(
            l,
            l.t_args("fix-shell-missing", &args!("shell" => shell.name())),
        )),
    }
}
