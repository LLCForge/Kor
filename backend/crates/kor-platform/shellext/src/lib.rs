//! Kor in the Windows 11 Explorer context menu and on the Widgets board (Master Build Prompt
//! §25.2, §27.10; ADR 0052).
//!
//! Windows 11 shows classic registry verbs (`HKCU\Software\Classes\…\shell`, written by
//! `kor_platform::shell_integration`) only under "Show more options". Entries in the new top-level
//! menu must come from an `IExplorerCommand` COM server owned by a package identity: this DLL,
//! registered through a sparse package (`kor_platform::shell_integration::sparse`) whose
//! external location is Kor's install folder. The same package hosts `kor-widgets.exe`, the
//! Widgets board provider.
//!
//! [`menu`] and [`widget_card`] are plain logic (tested on every OS); the COM servers exist only
//! on Windows.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub mod menu;
pub mod widget_card;

/// CLSID of the context-menu command (the sparse manifest names the same value).
pub const SHELL_CLSID: &str = "BEFBB221-CF76-40D8-87EC-2FD10BEBE592";
/// CLSID of the Widgets provider (`kor-widgets.exe`).
pub const WIDGET_CLSID: &str = "EF27EF40-0316-45BE-907E-CAEB2E2A8D39";

#[cfg(windows)]
mod com;
#[cfg(windows)]
pub mod widgets;
#[cfg(windows)]
#[allow(unsafe_code, missing_debug_implementations, clippy::pedantic)]
pub mod widgets_bindings;

/// Most paths passed to one launch (same limit as `kor_runtime::osint::MAX_PATHS`).
pub const MAX_PATHS: usize = 256;

/// Start `kor-desktop` (found from `module_dir`) with `args`. The desktop's single-instance
/// plugin hands them to an already running window.
///
/// # Errors
/// When `kor-desktop.exe` is not installed next to or above `module_dir`, or cannot be started.
pub fn launch_desktop(module_dir: &Path, args: &[OsString]) -> std::io::Result<()> {
    let exe = menu::desktop_candidates(module_dir)
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "kor-desktop.exe not found")
        })?;
    std::process::Command::new(exe).args(args).spawn().map(drop)
}

/// The folder of the running executable (the widgets exe lives in `<install>\shellext\`).
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_fails_cleanly_without_the_desktop() {
        let d = tempfile::tempdir().unwrap();
        let e = super::launch_desktop(d.path(), &[]).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn clsids_are_distinct_guids() {
        for c in [super::SHELL_CLSID, super::WIDGET_CLSID] {
            assert_eq!(c.len(), 36);
            assert!(c.chars().all(|ch| ch == '-' || ch.is_ascii_hexdigit()));
        }
        assert_ne!(super::SHELL_CLSID, super::WIDGET_CLSID);
    }
}
