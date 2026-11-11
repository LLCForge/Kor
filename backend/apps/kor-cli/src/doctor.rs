use std::path::Path;
use std::process::Command;

use kor_i18n::args;
use kor_secrets::{ExposeSecret, KeychainStore, SecretStore, SecretString};

pub mod completions;
/// `kor doctor --ipc-trace` (§17.2, ADR 0061).
pub mod ipc_trace;

use crate::Ctx;

const PROBE_ACCOUNT: &str = "doctor-probe";

/// Print each check; returns `true` when nothing failed.
pub fn run(ctx: &Ctx) -> bool {
    let ok = run_with(ctx, false, false, None);
    // `KOR_IPC_TRACE=<file> kor doctor` is `kor doctor --ipc-trace <file>` (§17.2, ADR 0061).
    match std::env::var_os(kor_core::ipc_trace::ENV).filter(|v| !v.is_empty()) {
        Some(file) => ipc_trace::run(ctx, Some(Path::new(&file))) && ok,
        None => ok,
    }
}

/// `kor doctor [--fix [--yes]]` (§26.1): the checks, then PATH, shell completions and Kor's own
/// folders, stale locks and interrupted writes. `--fix` repairs what it safely can, asking first
/// for each (never silently); `--yes` is the answer given in advance (the desktop's button).
/// `cmd` builds the `kor` command tree for completion scripts.
pub fn run_with(ctx: &Ctx, fix: bool, yes: bool, cmd: Option<&dyn Fn() -> clap::Command>) -> bool {
    let ok = checks(ctx);
    let l = &ctx.l;
    let ask = |what: String| -> bool {
        if yes {
            println!("  → {what}");
            true
        } else {
            completions::confirm(&l.t_args("doctor-fix-ask", &args!("what" => what)))
        }
    };
    let mut open = 0_i64;
    // PATH (§26.1): `kor` from any new terminal.
    if on_path() {
        println!("  ✓ {}", l.t("doctor-path-ok"));
    } else {
        println!("  ! {}", l.t("doctor-path-missing"));
        let exe = std::env::current_exe().ok();
        match exe {
            Some(exe) if fix && ask(l.t("doctor-fix-path")) => match kor_platform::cli_path::install(&exe) {
                Ok(c) => println!(
                    "  ✓ {}",
                    l.t_args("doctor-fixed-path", &args!("path" => c.location.map(|p| p.display().to_string()).unwrap_or_default()))
                ),
                Err(e) => {
                    open += 1;
                    println!("  ✗ {}", l.t_args("doctor-fix-failed", &args!("error" => e.to_string())));
                }
            },
            _ => open += 1,
        }
    }
    // Shell completions for the shells found on this machine.
    if let Some(dirs) = kor_platform::completions::Dirs::current() {
        for shell in completions::detected() {
            let plan = kor_platform::completions::plan(shell, &dirs);
            if kor_platform::completions::installed(&plan) {
                println!("  ✓ {}", l.t_args("doctor-completions-ok", &args!("shell" => shell.name())));
                continue;
            }
            println!("  ! {}", l.t_args("doctor-completions-missing", &args!("shell" => shell.name())));
            match cmd {
                // The profile-line question is asked by `install_one` itself.
                Some(cmd) if fix && ask(l.t_args("doctor-fix-completions", &args!("shell" => shell.name()))) => {
                    if !completions::install_one(l, cmd(), shell, yes) {
                        open += 1;
                    }
                }
                _ => open += 1,
            }
        }
    }
    // Kor's own folders, stale locks, interrupted writes.
    let issues = kor_runtime::prefs::find_issues(&kor_runtime::prefs::own_dirs(), std::time::Duration::from_secs(3600));
    for i in &issues {
        let key = match i {
            kor_runtime::prefs::Issue::MissingDir { .. } => "doctor-issue-dir",
            kor_runtime::prefs::Issue::StaleLock { .. } => "doctor-issue-lock",
            kor_runtime::prefs::Issue::StaleTemp { .. } => "doctor-issue-temp",
        };
        let what = l.t_args(key, &args!("path" => i.path().display().to_string()));
        println!("  ! {what}");
        if fix && ask(what.clone()) {
            match kor_runtime::prefs::fix(i) {
                Ok(()) => println!("  ✓ {}", l.t_args("doctor-fixed", &args!("what" => what))),
                Err(e) => {
                    open += 1;
                    println!("  ✗ {}", l.t_args("doctor-fix-failed", &args!("error" => e.to_string())));
                }
            }
        } else {
            open += 1;
        }
    }
    if open > 0 && !fix {
        println!("{}", l.t("doctor-fix-hint"));
    }
    prereqs(l);
    ok && (!fix || open == 0)
}

/// System prerequisites (§24.3, ADR 0064): status only. A missing one is not a failure (Kor keeps
/// working without it); installing is `kor prereq install`, which shows the command and asks.
fn prereqs(l: &kor_i18n::Localizer) {
    let mut missing = false;
    for s in kor_platform::sysdeps::status(&kor_platform::sysdeps::Host::detect()) {
        let mark = if s.installed == Some(true) { "✓" } else { "·" };
        println!("  {mark} {} — {}", prereq_describe(l, s.prereq), prereq_state(l, s.installed));
        missing |= s.installed == Some(false);
    }
    if missing {
        println!("{}", l.t("doctor-prereq-hint"));
    }
}

/// `prereq-docker` …: name and why Kor may need it (shared with `kor prereq`).
pub fn prereq_describe(l: &kor_i18n::Localizer, p: kor_platform::sysdeps::Prereq) -> String {
    l.t(&format!("prereq-{}", p.id()))
}

/// Present / missing / asked by the OS on first use.
pub fn prereq_state(l: &kor_i18n::Localizer, installed: Option<bool>) -> String {
    l.t(match installed {
        Some(true) => "prereq-installed",
        Some(false) => "prereq-missing",
        None => "prereq-on-first-use",
    })
}

/// A `kor` executable is in a PATH directory.
fn on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let name = kor_platform::exe("kor");
    std::env::split_paths(&path).any(|d| d.join(&name).is_file())
}

fn checks(ctx: &Ctx) -> bool {
    let l = &ctx.l;
    let mut failures: i64 = 0;
    let mut report = |ok: bool, line: String| {
        println!("  {} {line}", if ok { "✓" } else { "✗" });
        if !ok {
            failures += 1;
        }
    };

    println!("{}", l.t("doctor-title"));
    report(
        true,
        l.t_args(
            "doctor-os",
            &args!("os" => std::env::consts::OS, "arch" => std::env::consts::ARCH),
        ),
    );
    report(
        true,
        l.t_args("doctor-lang", &args!("lang" => l.lang().code())),
    );

    match &ctx.config {
        Ok(c) => report(
            true,
            l.t_args("doctor-config", &args!("count" => c.files.len())),
        ),
        Err(e) => report(false, crate::config_error(l, e)),
    }

    match probe_keychain(&KeychainStore) {
        Ok(()) => report(true, l.t("doctor-keychain-ok")),
        // Headless Linux (CI, servers) has no Secret Service; keys come from env vars there, by design.
        Err(e) if headless() => println!(
            "  ! {}",
            l.t_args("doctor-keychain-headless", &args!("error" => e))
        ),
        Err(e) => report(
            false,
            l.t_args("doctor-keychain-error", &args!("error" => e)),
        ),
    }

    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_owned();
            report(true, l.t_args("doctor-git", &args!("version" => v)));
        }
        _ => report(false, l.t("doctor-git-missing")),
    }

    let shells = find_shells();
    if shells.is_empty() {
        report(false, l.t("doctor-shell-missing"));
    } else {
        report(
            true,
            l.t_args("doctor-shell", &args!("shells" => shells.join(", "))),
        );
    }

    // §6.4: the backend `sandbox = on` uses; without one, commands ask (a warning, not a failure).
    let sbx = ctx
        .config
        .as_ref()
        .map_or_else(|_| kor_config::Sandbox::default(), |c| c.config.sandbox.clone());
    let mode = sbx.mode;
    let backend = kor_runtime::sandbox::kind(&kor_config::Sandbox {
        mode: kor_config::SandboxMode::On,
        ..sbx
    });
    if mode != kor_config::SandboxMode::On {
        report(
            true,
            l.t_args("doctor-sandbox-unused", &args!("mode" => mode.as_str())),
        );
    } else if backend.is_enforcing() {
        report(
            true,
            l.t_args(
                "doctor-sandbox",
                &args!("backend" => backend.as_str(), "mode" => mode.as_str()),
            ),
        );
    } else {
        println!("  ! {}", l.t("doctor-sandbox-missing"));
    }

    // §25.2: a failed context-menu registration is silent elsewhere; the reason shows here.
    let menu = kor_platform::shell_integration::recorded_status();
    match (&menu.last_error, menu.installed) {
        (Some(e), _) => println!("  ! {}", l.t_args("doctor-integrate-error", &args!("error" => e.clone()))),
        (None, true) => report(true, l.t("doctor-integrate-ok")),
        (None, false) => println!("  · {}", l.t("doctor-integrate-off")),
    }
    let integrations = ctx.config.as_ref().map(|c| c.config.integrations.clone()).unwrap_or_default();
    println!("  · {}", kor_runtime::autointegrate::status_line(l, &integrations));

    if ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode) {
        report(true, l.t("doctor-privacy-on"));
    }
    report(true, l.t("doctor-telemetry"));
    println!(
        "{}",
        l.t_args("doctor-summary", &args!("failures" => failures))
    );
    failures == 0
}

/// A Linux session without D-Bus (no Secret Service possible).
fn headless() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
}

/// Write, read back and delete a throwaway entry.
fn probe_keychain(store: &dyn SecretStore) -> Result<(), String> {
    let value = format!("probe-{}", std::process::id());
    store
        .set(PROBE_ACCOUNT, &SecretString::from(value.clone()))
        .map_err(|e| e.to_string())?;
    let read = store.get(PROBE_ACCOUNT).map_err(|e| e.to_string());
    let deleted = store.delete(PROBE_ACCOUNT).map_err(|e| e.to_string());
    match read? {
        Some(s) if s.expose_secret() == value => deleted.map(|_| ()),
        _ => Err("read-back mismatch".into()),
    }
}

fn find_shells() -> Vec<&'static str> {
    let candidates: &[&str] = if cfg!(windows) {
        &["pwsh", "powershell", "cmd", "bash"]
    } else {
        &["bash", "zsh", "fish", "sh", "pwsh"]
    };
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let dirs: Vec<_> = std::env::split_paths(&path).collect();
    candidates
        .iter()
        .copied()
        .filter(|name| dirs.iter().any(|d| is_executable(d, name)))
        .collect()
}

fn is_executable(dir: &Path, name: &str) -> bool {
    if cfg!(windows) {
        ["exe", "cmd", "bat", "com"]
            .iter()
            .any(|ext| dir.join(format!("{name}.{ext}")).is_file())
    } else {
        dir.join(name).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;

    #[test]
    fn probe_round_trips_and_cleans_up() {
        let store = MemoryStore::default();
        probe_keychain(&store).unwrap();
        assert!(store.get(PROBE_ACCOUNT).unwrap().is_none());
    }
}
