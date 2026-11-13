//! Operating-system differences behind one API (Master Build Prompt §2, rule 2).
//!
//! Other crates must not branch on the OS themselves (`cfg(windows)`, `cfg(unix)`, `target_os`);
//! they call this crate. `tests/leak.rs` enforces it with a ratchet: the number of such branches
//! outside this crate may only go down.

use std::path::PathBuf;

pub mod appcontainer;
// branding.toml (§22.2), Chrome native host registration (§24.4), Layer C installs (§24.3); ADR 0064.
pub mod brand;
pub mod native_host;
pub mod sysdeps;
pub use brand::brand;
pub mod cli_path;
// Shell completion files and profile lines (§26.1, ADR 0050).
pub mod completions;
// Idle time, front app, GPUs, battery, keyboard layout (§19, ADR 0059).
pub mod desktop_probe;
// Executables on PATH for the tool catalog (§26.2, ADR 0056).
pub mod on_path;
// Full-screen foreground app (ADR 0051, PiP window).
pub mod foreground;
// OS integration (ADR 0052): context menus, terminals, jump list / dock menu, §27.10 items,
// WSL paths, `kor mount`.
pub mod jumplist;
pub mod mount;
// Browser extension native messaging host manifests (§18, ADR 0058).
pub mod native_messaging;
pub mod os_integration;
// This process's resident memory (§32, ADR 0062).
pub mod procmem;
pub mod service;
pub mod shell_integration;
// Notification sounds for the CLI/TUI (§28.4): the bundled WAVs through the OS player.
pub mod sound;
// Machine load for the peer list (§31.1, ADR 0057).
pub mod sysload;
// Taskbar overlay badge (ADR 0051).
pub mod taskbar;
pub mod terminal;
// OS notifications with click callbacks (ADR 0051).
pub mod toast;
// Virtual desktops / workspaces of windows for saved layouts (§27.7).
pub mod vdesktop;
pub mod wsl;

/// Kor's user configuration directory: `%APPDATA%\kor`, `~/Library/Application Support/kor`,
/// `~/.config/kor`.
pub fn config_dir() -> Option<PathBuf> {
    // Same overrides as `kor_config::user_dir` / `kor_runtime::kor_home` (isolated E2E runs, ADR 0061).
    if let Some(d) = std::env::var_os("KOR_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::config_dir().map(|d| d.join("kor"))
}

/// Kor's local data (sessions, undo journals, indexes): `%LOCALAPPDATA%\kor`,
/// `~/Library/Application Support/kor`, `~/.local/share/kor`.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KOR_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::data_local_dir().map(|d| d.join("kor"))
}

/// Disposable caches (model catalog): `%LOCALAPPDATA%\kor`, `~/Library/Caches/kor`, `~/.cache/kor`.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KOR_CACHE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::cache_dir().map(|d| d.join("kor"))
}

/// Desktop operating system family, decided at run time so callers need no `cfg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
    Other,
}

/// The OS Kor runs on.
pub fn os() -> Os {
    match std::env::consts::OS {
        "windows" => Os::Windows,
        "macos" => Os::Mac,
        "linux" => Os::Linux,
        _ => Os::Other,
    }
}

/// Exclusive lock shared by every process on this machine, held until dropped.
pub struct FileLock {
    _held: fslock::LockFile,
}

impl std::fmt::Debug for FileLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileLock")
    }
}

/// Take the OS file lock on `path` (`LockFileEx` / `flock`), creating the file if missing.
/// Blocks while another process (or another handle in this one) holds it.
pub fn lock_file(path: &std::path::Path) -> std::io::Result<FileLock> {
    let mut f = fslock::LockFile::open(path)?;
    f.lock()?;
    Ok(FileLock { _held: f })
}

/// [`lock_file`] without waiting: `None` while another process holds it (`kor daemon`: one
/// instance per user).
pub fn try_lock_file(path: &std::path::Path) -> std::io::Result<Option<FileLock>> {
    let mut f = fslock::LockFile::open(path)?;
    Ok(f.try_lock()?.then_some(FileLock { _held: f }))
}

/// `name` with the platform's executable suffix (`git` → `git.exe` on Windows).
pub fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Name of the Python 3 interpreter on `PATH` (`python` on Windows, `python3` elsewhere).
pub fn python_command() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// Text editor used when neither `$VISUAL` nor `$EDITOR` is set.
pub fn default_editor() -> &'static str {
    if cfg!(windows) { "notepad" } else { "vi" }
}

/// Mark a file executable (`rwxr-xr-x` on Unix; Windows needs nothing).
///
/// # Errors
/// If the permissions cannot be set.
pub fn make_executable(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Owner user and group ids of `path` on Unix (Docker sandbox `--user`); `None` on Windows.
pub fn owner_ids(path: &std::path::Path) -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let m = std::fs::metadata(path).ok()?;
        Some((m.uid(), m.gid()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Windows `CREATE_NO_WINDOW`: a console program started from a GUI must not flash a window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start a child process without opening a console window (no-op outside Windows).
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

impl NoWindow for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }
}

/// Start a child that outlives its parent's session: its own process group on Unix (a closed
/// SSH channel or terminal does not take it down), `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`
/// on Windows. Give it stdio that is not the parent's (files or null).
pub trait Detach {
    fn detach(&mut self) -> &mut Self;
}

impl Detach for std::process::Command {
    fn detach(&mut self) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            self.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            self.creation_flags(0x0000_0008 | 0x0000_0200);
        }
        self
    }
}

/// Start `cmd` (already [`Detach`]ed) so it outlives the session that started it. On Windows a
/// process started over SSH sits in the session's job object, which `OpenSSH` closes (killing
/// the job) when the connection ends: ask to break away from it, and when the job does not allow
/// that, start it inside the job after all.
pub fn spawn_outliving(cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED | CREATE_BREAKAWAY_FROM_JOB);
        if let Ok(c) = cmd.spawn() {
            return Ok(c);
        }
        cmd.creation_flags(DETACHED);
    }
    cmd.spawn()
}

/// Copy the symbolic link `src` to `dst` (a tree copy). Unix: the same link. Windows (creating
/// links needs a privilege): a linked file is copied, a linked folder skipped (following it could
/// loop).
pub fn copy_link(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(std::fs::read_link(src)?, dst);
    #[cfg(not(unix))]
    match std::fs::metadata(src) {
        Ok(m) if m.is_file() => std::fs::copy(src, dst).map(|_| ()),
        _ => Ok(()),
    }
}

/// Stop a process and everything it started (dev servers, shells running builds).
/// Windows: `taskkill /T /F`; Unix: SIGTERM to the process group (the child must lead one).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("kill");
        c.args(["-TERM", "--", &format!("-{pid}")]);
        c
    };
    let _ = cmd
        .no_window()
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_end_in_kor_and_exe_suffix_matches_the_os() {
        for d in [config_dir(), data_dir(), cache_dir()]
            .into_iter()
            .flatten()
        {
            assert!(d.ends_with("kor"), "{}", d.display());
        }
        let ext = std::path::Path::new(&exe("git"))
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        assert_eq!(ext, cfg!(windows));
        assert_eq!(os() == Os::Windows, cfg!(windows));
        assert_eq!(os() == Os::Mac, cfg!(target_os = "macos"));
        assert_eq!(os() == Os::Linux, cfg!(target_os = "linux"));
    }

    #[test]
    fn no_window_still_runs_the_program() {
        let out = std::process::Command::new(exe("cargo"))
            .arg("--version")
            .no_window()
            .output()
            .unwrap();
        assert!(out.status.success());
    }
}
