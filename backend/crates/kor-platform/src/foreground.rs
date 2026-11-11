//! Is another program full screen (a game, a video, a presentation)? The desktop's always-on-top
//! status window hides then (§27.8, ADR 0051). `None` when the platform cannot tell.
//!
//! - Windows: `SHQueryUserNotificationState` (busy, Direct3D full screen, presentation mode).
//! - Linux (X11): `xprop` reads `_NET_WM_STATE_FULLSCREEN` of the active window.
//! - macOS: `osascript` asks System Events for the front window's `AXFullScreen` (needs the
//!   Accessibility permission the user grants; without it the answer is `None`).

/// Whether the foreground program is full screen.
pub fn fullscreen_app_active() -> Option<bool> {
    imp()
}

#[cfg(windows)]
#[allow(unsafe_code)] // one Win32 call with an out-parameter
fn imp() -> Option<bool> {
    use windows_sys::Win32::UI::Shell::{
        QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        SHQueryUserNotificationState,
    };
    let mut state = 0;
    // SAFETY: `state` is a valid, writable i32 for the call's duration.
    let hr = unsafe { SHQueryUserNotificationState(&raw mut state) };
    (hr >= 0).then_some(matches!(
        state,
        QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
    ))
}

#[cfg(target_os = "linux")]
fn imp() -> Option<bool> {
    let active = run("xprop", &["-root", "_NET_ACTIVE_WINDOW"])?;
    let id = parse_active_window(&active)?;
    let state = run("xprop", &["-id", &id, "_NET_WM_STATE"])?;
    Some(state.contains("_NET_WM_STATE_FULLSCREEN"))
}

#[cfg(target_os = "macos")]
fn imp() -> Option<bool> {
    let out = run(
        "osascript",
        &[
            "-e",
            "tell application \"System Events\" to get value of attribute \"AXFullScreen\" of front window of (first process whose frontmost is true)",
        ],
    )?;
    match out.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn imp() -> Option<bool> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run(program: &str, args: &[&str]) -> Option<String> {
    use crate::NoWindow as _;
    let out = std::process::Command::new(program)
        .args(args)
        .no_window()
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007` → `0x3a00007` (`0x0`: none).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_active_window(line: &str) -> Option<String> {
    let id = line.rsplit('#').next()?.trim().split(',').next()?.trim();
    (id.starts_with("0x")
        && id != "0x0"
        && id.len() > 2
        && id[2..].bytes().all(|b| b.is_ascii_hexdigit()))
    .then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_window_id_is_parsed_strictly() {
        assert_eq!(
            parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007").as_deref(),
            Some("0x3a00007")
        );
        assert_eq!(
            parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0"),
            None
        );
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW:  not found."), None);
        assert_eq!(parse_active_window("# 0x12; rm -rf /"), None);
    }

    #[test]
    fn query_does_not_panic() {
        let _ = fullscreen_app_active();
    }
}
