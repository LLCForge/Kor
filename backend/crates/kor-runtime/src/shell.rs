//! Shell integration flow shared by `kor shell|fix|why` and the desktop `shell.*` bridge (§15.4):
//! where the startup files and the failure record live, install/uninstall plans, status, and
//! running a suggested fix. Text formats and hooks are in `kor_agent::shellint`; callers ask the
//! user and print.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use kor_agent::shellint::{self, Failure, ShellKind};

/// Why an install/uninstall plan could not be made.
#[derive(Debug)]
pub enum ShellError {
    /// No Kor home directory (nowhere to keep the record).
    NoHome,
    /// The startup file location is unknown (no home directory).
    RcUnknown(ShellKind),
    /// Reading the startup file failed (other than "not found").
    Io(PathBuf, std::io::Error),
}

/// `<kor home>/shell/last-failure.txt`.
pub fn record_path() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("shell").join("last-failure.txt"))
}

/// Startup file for `k` in the user's home.
pub fn rc_for(k: ShellKind) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(k.rc_path(&home, |v| std::env::var(v).ok()))
}

/// The shell named by `name`, else the user's shell from the environment.
pub fn kind(name: Option<&str>) -> Option<ShellKind> {
    match name {
        Some(s) => ShellKind::parse(s),
        None => ShellKind::detect(|k| std::env::var(k).ok()),
    }
}

/// Existing startup file text ("" when missing); other read errors stop the change.
///
/// # Errors
/// Any read error except "not found".
pub fn read_rc(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// A pending change to a startup file. `new` is `None` when there is nothing to do.
#[derive(Debug, Clone)]
pub struct Plan {
    pub rc: PathBuf,
    /// The block being added (empty for uninstall).
    pub block: String,
    pub new: Option<String>,
    /// Record path whose directory is created on install.
    record: Option<PathBuf>,
}

/// Plan adding (or refreshing) Kor's block for `k`.
///
/// # Errors
/// See [`ShellError`].
pub fn plan_install(k: ShellKind) -> Result<Plan, ShellError> {
    let record = record_path().ok_or(ShellError::NoHome)?;
    let rc = rc_for(k).ok_or(ShellError::RcUnknown(k))?;
    install_plan(k, rc, record)
}

fn install_plan(k: ShellKind, rc: PathBuf, record: PathBuf) -> Result<Plan, ShellError> {
    let old = read_rc(&rc).map_err(|e| ShellError::Io(rc.clone(), e))?;
    let block = shellint::snippet(k, &record);
    let new = shellint::with_block(&old, &block);
    Ok(Plan {
        new: (new != old).then_some(new),
        rc,
        block,
        record: Some(record),
    })
}

/// Plan removing Kor's block for `k`.
///
/// # Errors
/// See [`ShellError`].
pub fn plan_uninstall(k: ShellKind) -> Result<Plan, ShellError> {
    uninstall_plan(rc_for(k).ok_or(ShellError::RcUnknown(k))?)
}

fn uninstall_plan(rc: PathBuf) -> Result<Plan, ShellError> {
    let old = read_rc(&rc).map_err(|e| ShellError::Io(rc.clone(), e))?;
    Ok(Plan {
        new: shellint::without_block(&old),
        rc,
        block: String::new(),
        record: None,
    })
}

/// Write the plan (no-op when there is nothing to change).
///
/// # Errors
/// Directory creation or write errors.
pub fn apply(plan: &Plan) -> std::io::Result<()> {
    let Some(new) = &plan.new else { return Ok(()) };
    for dir in [plan.record.as_deref(), Some(plan.rc.as_path())]
        .into_iter()
        .flatten()
        .filter_map(Path::parent)
    {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&plan.rc, new)
}

/// The user's shell (`$SHELL`, `PowerShell` module path), else the OS default.
pub fn default_kind() -> ShellKind {
    kind(None).unwrap_or_else(ShellKind::os_default)
}

/// The shell itself exists on this computer (not whether Kor's block is installed): one of its
/// programs is on `PATH`; on Windows also Windows `PowerShell` 5.1 under `SystemRoot` and Git
/// Bash under `Program Files` / the per-user Git folder (WSL's `bash.exe` is on `PATH`).
pub fn present(k: ShellKind) -> bool {
    if k.runner()
        .iter()
        .any(|(exe, _)| kor_platform::os_integration::on_path(exe))
    {
        return true;
    }
    if kor_platform::os() != kor_platform::Os::Windows {
        return false;
    }
    let under = |var: &str, rest: &[&str]| {
        std::env::var_os(var).is_some_and(|v| {
            rest.iter()
                .fold(PathBuf::from(v), |p, s| p.join(s))
                .is_file()
        })
    };
    match k {
        ShellKind::PowerShell => under(
            "SystemRoot",
            &["System32", "WindowsPowerShell", "v1.0", "powershell.exe"],
        ),
        ShellKind::Bash => {
            under("ProgramFiles", &["Git", "bin", "bash.exe"])
                || under("LOCALAPPDATA", &["Programs", "Git", "bin", "bash.exe"])
        }
        ShellKind::Zsh | ShellKind::Fish => false,
    }
}

/// Every shell with a known startup file, and whether Kor's block is in it.
pub fn status() -> Vec<(ShellKind, PathBuf, bool)> {
    ShellKind::ALL
        .into_iter()
        .filter_map(|k| {
            let rc = rc_for(k)?;
            let on = read_rc(&rc).is_ok_and(|t| shellint::has_block(&t));
            Some((k, rc, on))
        })
        .collect()
}

/// The last recorded failure, if any.
pub fn load_failure() -> Option<Failure> {
    load_failure_from(&record_path()?)
}

fn load_failure_from(path: &Path) -> Option<Failure> {
    shellint::parse(&std::fs::read_to_string(path).ok()?)
}

/// Record a failed `kor shell capture` run (best effort, like the shell hooks).
pub fn record_failure(command: &[String], exit: i32, cwd: &Path, output: Vec<String>) {
    if let Some(path) = record_path() {
        save_failure_to(&path, command, exit, cwd, output);
    }
}

fn save_failure_to(path: &Path, command: &[String], exit: i32, cwd: &Path, output: Vec<String>) {
    let f = Failure {
        command: command.join(" "),
        exit,
        shell: ShellKind::detect(|k| std::env::var(k).ok())
            .map_or("", ShellKind::name)
            .to_owned(),
        cwd: cwd.display().to_string(),
        output,
    };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(path))
        .and_then(|()| std::fs::write(path, shellint::render(&f)));
}

/// Why a suggested fix could not run.
#[derive(Debug)]
pub enum RunError {
    /// None of the recorded shell's programs is installed.
    ShellMissing(ShellKind),
    Io(std::io::Error),
}

/// Run `cmd` in the shell that recorded `f` (else the user's, else the OS default), in the
/// recorded directory when it still exists. `run` starts the prepared command; a missing program
/// moves on to the next runner (`pwsh`, then `powershell`).
///
/// # Errors
/// See [`RunError`].
pub fn run_fix<T>(
    f: &Failure,
    cmd: &str,
    mut run: impl FnMut(&mut Command) -> std::io::Result<T>,
) -> Result<T, RunError> {
    let shell = ShellKind::parse(&f.shell)
        .or_else(|| ShellKind::detect(|k| std::env::var(k).ok()))
        .unwrap_or_else(ShellKind::os_default);
    let cwd = Some(PathBuf::from(&f.cwd)).filter(|p| p.is_dir());
    for (exe, flags) in shell.runner() {
        let mut c = Command::new(exe);
        c.args(flags).arg(cmd);
        if let Some(d) = &cwd {
            c.current_dir(d);
        }
        match run(&mut c) {
            Ok(t) => return Ok(t),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(RunError::Io(e)),
        }
    }
    Err(RunError::ShellMissing(shell))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_apply_and_uninstall_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("cfg").join(".bashrc");
        let record = dir
            .path()
            .join("home")
            .join("shell")
            .join("last-failure.txt");
        let p = install_plan(ShellKind::Bash, rc.clone(), record.clone()).unwrap();
        assert!(p.new.is_some() && p.block.contains(shellint::BEGIN));
        apply(&p).unwrap();
        assert!(record.parent().unwrap().is_dir());
        assert!(shellint::has_block(&read_rc(&rc).unwrap()));
        let again = install_plan(ShellKind::Bash, rc.clone(), record).unwrap();
        assert!(again.new.is_none());
        let u = uninstall_plan(rc.clone()).unwrap();
        apply(&u).unwrap();
        assert_eq!(read_rc(&rc).unwrap(), "");
        assert!(uninstall_plan(rc).unwrap().new.is_none());
    }

    #[test]
    fn failures_are_saved_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell").join("last-failure.txt");
        assert!(load_failure_from(&path).is_none());
        save_failure_to(
            &path,
            &["cargo".into(), "biuld".into()],
            101,
            Path::new("/w"),
            vec!["error".into()],
        );
        let f = load_failure_from(&path).unwrap();
        assert_eq!((f.command.as_str(), f.exit), ("cargo biuld", 101));
        assert_eq!(f.output, vec!["error"]);
    }

    #[test]
    fn fix_runs_in_recorded_shell_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let f = Failure {
            command: "x".into(),
            shell: "powershell".into(),
            cwd: dir.path().display().to_string(),
            ..Failure::default()
        };
        let mut seen = Vec::new();
        let r = run_fix(&f, "ls", |c| {
            seen.push(c.get_program().to_string_lossy().into_owned());
            assert_eq!(c.get_current_dir(), Some(dir.path()));
            assert_eq!(c.get_args().last().unwrap(), "ls");
            Err::<(), _>(std::io::Error::from(ErrorKind::NotFound))
        });
        assert!(matches!(
            r,
            Err(RunError::ShellMissing(ShellKind::PowerShell))
        ));
        assert_eq!(seen, ["pwsh", "powershell"]);
        let gone = Failure {
            shell: "bash".into(),
            cwd: "/no/such/dir/kor".into(),
            ..f
        };
        let ok = run_fix(&gone, "ls", |c| Ok(c.get_current_dir().is_none()));
        assert!(matches!(ok, Ok(true)));
    }
}
