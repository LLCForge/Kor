//! Putting the desktop's bundled `kor` CLI on the user's PATH (Master Build Prompt §12.3, ADR 0032).
//!
//! The desktop ships `kor` next to its own executable. Only on explicit user consent (the caller
//! asks) this module makes it callable from a terminal, idempotently, and undoes exactly what it
//! did:
//! - Windows: the bundle directory is added to the per-user `Path` (`HKCU\Environment`, kept as
//!   `REG_EXPAND_SZ`, no admin) and running programs are told (`WM_SETTINGCHANGE`).
//! - macOS / Linux: `~/.local/bin/kor` links to the bundled binary (a copy when it lives inside an
//!   `AppImage` mount, which disappears on exit). If `~/.local/bin` is not on PATH, one marked line
//!   is added to `~/.zprofile` (macOS) or `~/.profile` (Linux).
//! - Linux `.deb` / `.rpm` already install `kor` in `/usr/bin`: nothing to do.
//!
//! A `kor` that Kor did not place is never overwritten or removed. What was done is recorded in
//! `<data dir>/cli-path.txt` so that removal only touches Kor's own changes.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod win;

/// Comment that marks the line Kor adds to a shell profile.
const MARK: &str = "# added by Kor desktop (kor CLI)";

/// Where the CLI stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliPath {
    /// The bundled binary.
    pub source: PathBuf,
    /// `kor` can be started from a new terminal.
    pub installed: bool,
    /// The directory added to PATH (Windows) or the link (macOS / Linux).
    pub location: Option<PathBuf>,
    /// Installed by a system package (`.deb` / `.rpm`): nothing to install or remove.
    pub by_package: bool,
    /// Kor's own change exists and can be removed.
    pub removable: bool,
}

/// What [`install`] / [`remove`] work on: home, Kor's data dir, the inherited PATH, the platform.
#[derive(Debug, Clone)]
struct Env {
    home: PathBuf,
    data: PathBuf,
    path: Vec<PathBuf>,
    mac: bool,
    /// Running from an `AppImage` (the source disappears when the app exits).
    appimage: bool,
}

impl Env {
    fn current() -> io::Result<Self> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, "home directory not found");
        Ok(Self {
            home: dirs::home_dir().ok_or_else(missing)?,
            data: crate::data_dir().ok_or_else(missing)?,
            path: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            mac: crate::os() == crate::Os::Mac,
            appimage: std::env::var_os("APPIMAGE").is_some(),
        })
    }
    fn link(&self) -> PathBuf {
        self.home.join(".local").join("bin").join(crate::exe("kor"))
    }
    fn profile(&self) -> PathBuf {
        self.home
            .join(if self.mac { ".zprofile" } else { ".profile" })
    }
    fn record(&self) -> PathBuf {
        self.data.join("cli-path.txt")
    }
    /// The source's directory is already on PATH (system package) and survives the app.
    fn by_package(&self, source: &Path) -> bool {
        !self.appimage
            && source
                .parent()
                .is_some_and(|d| self.path.iter().any(|p| p == d))
    }
}

/// Current state for the bundled binary at `source`.
pub fn status(source: &Path) -> CliPath {
    if crate::os() == crate::Os::Windows {
        return windows_status(source);
    }
    match Env::current() {
        Ok(env) => home_status(&env, source),
        Err(_) => CliPath {
            source: source.to_owned(),
            installed: false,
            location: None,
            by_package: false,
            removable: false,
        },
    }
}

/// Make `kor` callable from new terminals. Idempotent.
///
/// # Errors
/// If `source` is missing, a foreign `kor` is in the way, or the change cannot be written.
pub fn install(source: &Path) -> io::Result<CliPath> {
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "bundled kor CLI not found",
        ));
    }
    if crate::os() == crate::Os::Windows {
        let dir = source.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let current = windows_user_path()?;
        if let Some(next) = path_list_add(&current, &dir.to_string_lossy()) {
            windows_set_user_path(&next)?;
        }
        return Ok(windows_status(source));
    }
    let env = Env::current()?;
    home_install(&env, source)?;
    Ok(home_status(&env, source))
}

/// Undo [`install`]. Idempotent; leaves anything Kor did not create.
///
/// # Errors
/// If the change cannot be written.
pub fn remove(source: &Path) -> io::Result<CliPath> {
    if crate::os() == crate::Os::Windows {
        if let Some(dir) = source.parent() {
            let current = windows_user_path()?;
            if let Some(next) = path_list_remove(&current, &dir.to_string_lossy()) {
                windows_set_user_path(&next)?;
            }
        }
        return Ok(windows_status(source));
    }
    let env = Env::current()?;
    home_remove(&env)?;
    Ok(home_status(&env, source))
}

/// Refresh a copied CLI after the desktop was updated (`AppImage`); no-op otherwise or when the
/// user never installed it.
///
/// # Errors
/// If the copy cannot be written.
pub fn refresh(source: &Path) -> io::Result<()> {
    if crate::os() == crate::Os::Windows {
        return Ok(());
    }
    let env = Env::current()?;
    let link = env.link();
    let ours = std::fs::read_to_string(env.record())
        .is_ok_and(|r| r.lines().any(|l| l == format!("link {}", link.display())));
    if ours && env.appimage && std::fs::read(&link).ok() != std::fs::read(source).ok() {
        place(source, &link, true)?;
    }
    Ok(())
}

#[cfg(windows)]
use win::{set_user_path as windows_set_user_path, user_path as windows_user_path};

#[cfg(not(windows))]
fn windows_user_path() -> io::Result<String> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(windows))]
fn windows_set_user_path(_: &str) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

fn windows_status(source: &Path) -> CliPath {
    let dir = source.parent().map(Path::to_path_buf);
    let installed = dir.as_ref().is_some_and(|d| {
        windows_user_path().is_ok_and(|p| path_list_has(&p, &d.to_string_lossy()))
    });
    CliPath {
        source: source.to_owned(),
        installed: installed && source.is_file(),
        location: dir,
        by_package: false,
        removable: installed,
    }
}

fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

fn path_list_has(list: &str, dir: &str) -> bool {
    list.split(';').any(|e| same_dir(e, dir))
}

/// `list` with `dir` appended, or `None` when it is already there.
fn path_list_add(list: &str, dir: &str) -> Option<String> {
    if path_list_has(list, dir) {
        return None;
    }
    let base = list.trim_end_matches(';');
    Some(if base.is_empty() {
        dir.to_owned()
    } else {
        format!("{base};{dir}")
    })
}

/// `list` without `dir`, or `None` when it was not there.
fn path_list_remove(list: &str, dir: &str) -> Option<String> {
    path_list_has(list, dir).then(|| {
        list.split(';')
            .filter(|e| !same_dir(e, dir))
            .collect::<Vec<_>>()
            .join(";")
    })
}

fn home_status(env: &Env, source: &Path) -> CliPath {
    if env.by_package(source) {
        return CliPath {
            source: source.to_owned(),
            installed: true,
            location: source.parent().map(Path::to_path_buf),
            by_package: true,
            removable: false,
        };
    }
    let link = env.link();
    let removable = std::fs::read_to_string(env.record()).is_ok_and(|r| !r.trim().is_empty());
    CliPath {
        source: source.to_owned(),
        installed: link.is_file() && removable,
        location: Some(link),
        by_package: false,
        removable,
    }
}

/// Symlink `link` → `source`, or copy it when `copy` (or symlinks are unavailable).
fn place(source: &Path, link: &Path, copy: bool) -> io::Result<()> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(link);
    #[cfg(unix)]
    if !copy {
        return std::os::unix::fs::symlink(source, link);
    }
    let _ = copy;
    std::fs::copy(source, link)?;
    crate::make_executable(link)
}

fn home_install(env: &Env, source: &Path) -> io::Result<()> {
    if env.by_package(source) {
        return Ok(());
    }
    let link = env.link();
    let record = env.record();
    let mut done: Vec<String> = std::fs::read_to_string(&record)
        .map(|r| r.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    let link_line = format!("link {}", link.display());
    let ours = done.contains(&link_line) || std::fs::read_link(&link).is_ok_and(|t| t == source);
    if link.symlink_metadata().is_ok() && !ours {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} belongs to another installation", link.display()),
        ));
    }
    place(source, &link, env.appimage)?;
    if !done.contains(&link_line) {
        done.push(link_line);
    }
    let bin_dir = link.parent().map(Path::to_path_buf).unwrap_or_default();
    let profile = env.profile();
    let text = std::fs::read_to_string(&profile).unwrap_or_default();
    if !env.path.contains(&bin_dir) && !text.contains(".local/bin") {
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let export = format!("{sep}export PATH=\"$HOME/.local/bin:$PATH\" {MARK}\n");
        std::fs::write(&profile, format!("{text}{export}"))?;
        done.push(format!("profile {}", profile.display()));
    }
    if let Some(dir) = record.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&record, done.join("\n") + "\n")
}

fn home_remove(env: &Env) -> io::Result<()> {
    let record = env.record();
    let Ok(text) = std::fs::read_to_string(&record) else {
        return Ok(());
    };
    for line in text.lines() {
        if let Some(link) = line.strip_prefix("link ") {
            match std::fs::remove_file(link) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        } else if let Some(profile) = line.strip_prefix("profile ")
            && let Ok(body) = std::fs::read_to_string(profile)
        {
            let kept: String = body
                .split_inclusive('\n')
                .filter(|l| !l.contains(MARK))
                .collect();
            std::fs::write(profile, kept)?;
        }
    }
    std::fs::remove_file(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_list_is_edited_once_and_undone_exactly() {
        let list = r"%USERPROFILE%\.cargo\bin;C:\Tools";
        let dir = r"C:\Users\user\AppData\Local\Kor";
        let added = path_list_add(list, dir).unwrap();
        assert_eq!(added, format!("{list};{dir}"));
        assert!(path_list_add(&added, &format!("{}\\", dir.to_uppercase())).is_none());
        assert_eq!(
            path_list_remove(&added, dir).unwrap(),
            list,
            "%VARS% kept unexpanded"
        );
        assert!(path_list_remove(list, dir).is_none());
        assert_eq!(path_list_add("", dir).unwrap(), dir);
        // Read-only: the real user Path is readable (never written by tests).
        if crate::os() == crate::Os::Windows {
            assert!(windows_user_path().is_ok());
        }
    }

    fn env(root: &Path, appimage: bool) -> Env {
        Env {
            home: root.join("home"),
            data: root.join("data"),
            path: vec![PathBuf::from("/usr/bin")],
            mac: false,
            appimage,
        }
    }

    #[test]
    fn home_install_is_idempotent_reversible_and_never_takes_a_foreign_kor() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("app").join(crate::exe("kor"));
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, b"kor").unwrap();
        // Copy mode works everywhere (symlinks need Unix); the logic is the same.
        let e = env(dir.path(), true);
        let profile = e.profile();
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, "export EDITOR=vi").unwrap();

        home_install(&e, &src).unwrap();
        home_install(&e, &src).unwrap();
        let s = home_status(&e, &src);
        assert!(s.installed && s.removable && !s.by_package);
        assert_eq!(std::fs::read(e.link()).unwrap(), b"kor");
        let body = std::fs::read_to_string(&profile).unwrap();
        assert_eq!(body.matches(MARK).count(), 1, "{body}");

        home_remove(&e).unwrap();
        home_remove(&e).unwrap();
        assert!(!e.link().exists());
        assert_eq!(
            std::fs::read_to_string(&profile).unwrap(),
            "export EDITOR=vi\n"
        );
        assert!(!home_status(&e, &src).installed);

        // Someone else's ~/.local/bin/kor stays untouched.
        std::fs::write(e.link(), b"other").unwrap();
        assert!(home_install(&e, &src).is_err());
        home_remove(&e).unwrap();
        assert_eq!(std::fs::read(e.link()).unwrap(), b"other");
    }

    #[test]
    fn package_installs_need_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let e = env(dir.path(), false);
        let s = home_status(&e, Path::new("/usr/bin/kor"));
        assert!(s.by_package && s.installed && !s.removable);
        home_install(&e, Path::new("/usr/bin/kor")).unwrap();
        assert!(!e.record().exists());
    }
}
