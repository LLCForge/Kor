//! Which virtual desktop (Windows) / workspace (Linux) / Space (macOS) a window is on, and moving
//! it back there, for saved window layouts (§27.7).
//!
//! - Windows: the documented `IVirtualDesktopManager` (`GetWindowDesktopId`,
//!   `MoveWindowToDesktop`). A desktop is its GUID, stable across restarts. Windows lets
//!   `MoveWindowToDesktop` move only windows of the calling process, so for other apps' windows it
//!   fails with "access denied"; that is reported per window ("move it by hand"), never hidden.
//!   The undocumented `IVirtualDesktopManagerInternal` that could move any window changes between
//!   Windows builds and is not used.
//! - Linux (X11 and `XWayland` windows): EWMH `_NET_WM_DESKTOP` through `wmctrl` (`wmctrl -lp`,
//!   `wmctrl -i -r <id> -t <n>`); a workspace is its number. Windows are matched by process id and
//!   title. Native Wayland windows have no common protocol for this: unknown there.
//! - macOS: Spaces have no public API (a window's Space can be neither read nor set without
//!   private `CGS*` calls), so nothing is recorded.

/// A window as the window tools list it.
#[derive(Debug, Clone, Copy)]
pub struct WindowRef<'a> {
    /// `HWND` on Windows; the window tools' own handle elsewhere.
    pub hwnd: i64,
    /// Owning process (Linux matching); 0 when unknown.
    pub pid: u32,
    pub title: &'a str,
}

/// The desktop of each window: a GUID (Windows) or workspace number (Linux); `None` when unknown
/// or shown on every desktop.
pub fn desktops_of(windows: &[WindowRef<'_>]) -> Vec<Option<String>> {
    imp::desktops_of(windows)
}

/// Move `w` to `desktop` (as [`desktops_of`] reported it).
pub fn move_window(w: WindowRef<'_>, desktop: &str) -> Result<(), String> {
    imp::move_window(w, desktop)
}

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` (upper case) for a GUID's `u128` value.
pub fn guid_string(v: u128) -> String {
    let h = format!("{v:032X}");
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Inverse of [`guid_string`] (braces optional).
pub fn parse_guid(s: &str) -> Option<u128> {
    let hex: String = s
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split('-')
        .collect();
    (hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
}

/// One `wmctrl -lp` row: window id, desktop (`None` for sticky `-1`), pid, title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WmRow {
    pub id: String,
    pub desktop: Option<u32>,
    pub pid: u32,
    pub title: String,
}

/// `0x03a00003  1 4242 host Title with  spaces` → row.
pub fn parse_wmctrl(text: &str) -> Vec<WmRow> {
    fn field(s: &str) -> Option<(&str, &str)> {
        let s = s.trim_start();
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        (end > 0).then(|| (&s[..end], &s[end..]))
    }
    text.lines()
        .filter_map(|l| {
            let (id, rest) = field(l)?;
            let (desk, rest) = field(rest)?;
            let (pid, rest) = field(rest)?;
            let (_host, rest) = field(rest).unwrap_or(("", ""));
            Some(WmRow {
                id: id.to_owned(),
                desktop: desk.parse::<i64>().ok().and_then(|d| u32::try_from(d).ok()),
                pid: pid.parse().unwrap_or(0),
                title: rest.trim_start_matches([' ', '\t']).to_owned(),
            })
        })
        .collect()
}

/// The row for `w`: same process and title, else the process's only window.
pub fn find_row<'a>(rows: &'a [WmRow], w: &WindowRef<'_>) -> Option<&'a WmRow> {
    let mine: Vec<&WmRow> = rows.iter().filter(|r| r.pid == w.pid).collect();
    mine.iter()
        .find(|r| r.title == w.title)
        .or_else(|| (mine.len() == 1).then(|| &mine[0]))
        .copied()
}

#[cfg(windows)]
mod imp {
    use super::{WindowRef, guid_string, parse_guid};

    /// Run `f` with an `IVirtualDesktopManager` on a fresh STA thread.
    #[allow(unsafe_code)] // COM activation; the interface is released when `f` returns
    fn with_manager<R: Send>(
        f: impl FnOnce(&windows::Win32::UI::Shell::IVirtualDesktopManager) -> R + Send,
    ) -> Result<R, String> {
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
        std::thread::scope(|s| {
            s.spawn(|| {
                // SAFETY: COM is initialised on this new thread and uninitialised after the
                // interface (dropped at the end of the block) is released.
                unsafe {
                    CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                        .ok()
                        .map_err(|e| e.to_string())?;
                    let r = CoCreateInstance::<_, IVirtualDesktopManager>(
                        &VirtualDesktopManager,
                        None,
                        CLSCTX_ALL,
                    )
                    .map(|m| f(&m))
                    .map_err(|e| format!("virtual desktops are not available: {e}"));
                    CoUninitialize();
                    r
                }
            })
            .join()
            .map_err(|_| "virtual desktop query failed".to_owned())?
        })
    }

    fn hwnd(h: i64) -> windows::Win32::Foundation::HWND {
        windows::Win32::Foundation::HWND(isize::try_from(h).unwrap_or(0) as *mut core::ffi::c_void)
    }

    #[allow(unsafe_code)] // COM method calls on a live interface
    pub(super) fn desktops_of(windows: &[WindowRef<'_>]) -> Vec<Option<String>> {
        let hwnds: Vec<i64> = windows.iter().map(|w| w.hwnd).collect();
        with_manager(|m| {
            hwnds
                .iter()
                // SAFETY: the manager is valid for the call; a stale HWND only returns an error.
                .map(|h| unsafe { m.GetWindowDesktopId(hwnd(*h)) }.ok())
                .map(|g| {
                    g.filter(|g| g.to_u128() != 0)
                        .map(|g| guid_string(g.to_u128()))
                })
                .collect()
        })
        .unwrap_or_else(|_| vec![None; windows.len()])
    }

    #[allow(unsafe_code)] // COM method call on a live interface
    pub(super) fn move_window(w: WindowRef<'_>, desktop: &str) -> Result<(), String> {
        let id = parse_guid(desktop).ok_or_else(|| format!("invalid desktop id {desktop}"))?;
        let guid = windows::core::GUID::from_u128(id);
        let h = w.hwnd;
        with_manager(move |m| {
            // SAFETY: the manager is valid for the call; a stale HWND only returns an error.
            unsafe { m.MoveWindowToDesktop(hwnd(h), &raw const guid) }.map_err(|e| {
                if e.code().0 == windows::Win32::Foundation::E_ACCESSDENIED.0 {
                    "Windows lets only the app itself move this window to another virtual desktop; move it by hand (Win+Tab)".to_owned()
                } else {
                    e.to_string()
                }
            })
        })?
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{WindowRef, find_row, parse_wmctrl};

    fn rows() -> Option<Vec<super::WmRow>> {
        let out = std::process::Command::new("wmctrl")
            .arg("-lp")
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_wmctrl(&String::from_utf8_lossy(&out.stdout)))
    }

    pub(super) fn desktops_of(windows: &[WindowRef<'_>]) -> Vec<Option<String>> {
        let rows = rows().unwrap_or_default();
        windows
            .iter()
            .map(|w| find_row(&rows, w)?.desktop.map(|d| d.to_string()))
            .collect()
    }

    pub(super) fn move_window(w: WindowRef<'_>, desktop: &str) -> Result<(), String> {
        let n: u32 = desktop
            .parse()
            .map_err(|_| format!("invalid workspace {desktop}"))?;
        let rows = rows().ok_or(
            "wmctrl is not installed or there is no X11 window manager (native Wayland windows cannot be moved between workspaces)",
        )?;
        let row = find_row(&rows, &w).ok_or("the window is not visible to wmctrl")?;
        let ok = std::process::Command::new("wmctrl")
            .args(["-i", "-r", &row.id, "-t", &n.to_string()])
            .stdin(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            Ok(())
        } else {
            Err(format!("wmctrl could not move the window to workspace {n}"))
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::WindowRef;

    pub(super) fn desktops_of(windows: &[WindowRef<'_>]) -> Vec<Option<String>> {
        vec![None; windows.len()]
    }

    pub(super) fn move_window(_w: WindowRef<'_>, _desktop: &str) -> Result<(), String> {
        Err("macOS has no public API to move a window to another Space; move it by hand".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_round_trip() {
        let v = 0x1234_5678_9abc_def0_1122_3344_5566_7788_u128;
        let s = guid_string(v);
        assert_eq!(s, "{12345678-9ABC-DEF0-1122-334455667788}");
        assert_eq!(parse_guid(&s), Some(v));
        assert_eq!(parse_guid("12345678-9abc-def0-1122-334455667788"), Some(v));
        assert_eq!(parse_guid("{1234}"), None);
        assert_eq!(parse_guid("zz345678-9abc-def0-1122-334455667788"), None);
    }

    #[test]
    fn wmctrl_rows_and_matching() {
        let rows = parse_wmctrl(
            "0x03a00003  1 4242 host main.rs - Code\n0x04000001 -1 99   host Panel\n0x05000002  0 4242 host  Two  spaces\nbroken\n",
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].desktop, Some(1));
        assert_eq!(rows[0].title, "main.rs - Code");
        assert_eq!(rows[1].desktop, None, "sticky");
        assert_eq!(rows[2].title, "Two  spaces");
        let w = |pid, title| WindowRef {
            hwnd: 0,
            pid,
            title,
        };
        assert_eq!(
            find_row(&rows, &w(4242, "Two  spaces")).unwrap().id,
            "0x05000002"
        );
        assert!(
            find_row(&rows, &w(4242, "renamed")).is_none(),
            "two windows: ambiguous"
        );
        assert_eq!(find_row(&rows, &w(99, "renamed")).unwrap().id, "0x04000001");
        assert!(find_row(&rows, &w(7, "x")).is_none());
    }

    #[test]
    fn unknown_windows_have_no_desktop() {
        let d = desktops_of(&[WindowRef {
            hwnd: 0,
            pid: 0,
            title: "",
        }]);
        assert_eq!(d, [None]);
        assert!(
            move_window(
                WindowRef {
                    hwnd: 0,
                    pid: 0,
                    title: ""
                },
                "nonsense"
            )
            .is_err()
        );
    }
}
