//! OS notifications with a click callback (§27.2, ADR 0051): Windows toast, macOS Notification
//! Center, libnotify / the freedesktop notification service. Feature `toast` (the desktop app
//! enables it; the CLI does not link a notification stack).
//!
//! The notification is silent: the desktop plays its own sound when the "call me" policy allows.

/// One notification.
#[derive(Debug, Clone, Default)]
pub struct Toast {
    pub title: String,
    pub body: String,
    /// Windows `AppUserModelID` the installer registered (`None`: development build — Windows
    /// then shows it under `PowerShell`). Ignored elsewhere.
    pub app_id: Option<String>,
}

/// Show `t`; `on_click` runs (on a background thread) when the user clicks it.
///
/// # Errors
/// When the notification service is unavailable.
#[cfg(feature = "toast")]
pub fn show(t: &Toast, on_click: impl FnOnce() + Send + 'static) -> Result<(), String> {
    let mut n = notify_rust::Notification::new();
    n.summary(&clip(&t.title, 120))
        .body(&clip(&t.body, 400))
        .appname("Kor");
    set_app_id(&mut n, t.app_id.as_deref());
    let handle = n.show().map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("kor-toast".into())
        .spawn(move || {
            let _ = handle.wait_for_response(|r: &notify_rust::NotificationResponse| {
                if matches!(
                    r,
                    notify_rust::NotificationResponse::Default
                        | notify_rust::NotificationResponse::Action(_)
                ) {
                    on_click();
                }
            });
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(all(feature = "toast", windows))]
fn set_app_id(n: &mut notify_rust::Notification, id: Option<&str>) {
    if let Some(id) = id {
        n.app_id(id);
    }
}

#[cfg(all(feature = "toast", not(windows)))]
fn set_app_id(_n: &mut notify_rust::Notification, _id: Option<&str>) {}

/// Without the `toast` feature nothing is shown.
///
/// # Errors
/// Always: notifications are not built in.
#[cfg(not(feature = "toast"))]
pub fn show(_t: &Toast, _on_click: impl FnOnce() + Send + 'static) -> Result<(), String> {
    Err("OS notifications are not built into this program".into())
}

/// At most `n` characters (OS notification limits; Windows truncates silently).
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_counts_characters() {
        assert_eq!(clip("çğış", 4), "çğış");
        assert_eq!(clip("çğışö", 4), "çğı…");
    }
}
