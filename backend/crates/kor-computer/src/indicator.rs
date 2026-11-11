//! Visible indicator while Kor controls the desktop (§19.1): every desktop action (and every pause)
//! is published to one listener. The desktop app shows a bar with a stop button; the CLI prints a
//! line on stderr when control starts and when the user interrupts it.

use std::io::Write as _;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use kor_i18n::{Localizer, args};
use serde::Serialize;

use crate::guard::Interruption;

/// Control is shown as active for this long after the last action.
pub const ACTIVE_MS: u64 = 5_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Kor is acting on the desktop now.
    pub active: bool,
    /// Tool name (`left_click`, `uia_invoke`, …).
    pub tool: String,
    /// App the action targets.
    pub app: Option<String>,
    /// Set when the user interrupted and Kor paused.
    pub paused: Option<Interruption>,
    /// Unix ms of the event.
    pub at: u64,
}

pub type Listener = Arc<dyn Fn(&Activity) + Send + Sync>;

static LISTENER: RwLock<Option<Listener>> = RwLock::new(None);
static LAST: Mutex<Option<Activity>> = Mutex::new(None);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Replace the listener (the desktop app sets its own at start).
pub fn set_listener(l: Listener) {
    *LISTENER.write().unwrap_or_else(PoisonError::into_inner) = Some(l);
}

/// Set `l` only when no listener is set yet (the CLI default must not replace the desktop's).
pub fn set_listener_if_none(l: Listener) {
    let mut w = LISTENER.write().unwrap_or_else(PoisonError::into_inner);
    if w.is_none() {
        *w = Some(l);
    }
}

/// Publish one event.
#[allow(clippy::needless_pass_by_value)] // callers build the event for this call
pub fn emit(a: Activity) {
    *LAST.lock().unwrap_or_else(PoisonError::into_inner) = Some(a.clone());
    let l = LISTENER.read().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(l) = l {
        l(&a);
    }
}

/// The latest event, with `active` cleared once [`ACTIVE_MS`] passed.
pub fn current() -> Activity {
    let mut a = LAST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default();
    if a.active && now_ms().saturating_sub(a.at) > ACTIVE_MS {
        a.active = false;
    }
    a
}

/// The CLI's indicator: one line on stderr when control (re)starts after a quiet spell and when
/// the user interrupts.
pub fn stderr_listener(l: Arc<Localizer>) -> Listener {
    let shown_at = Mutex::new(0u64);
    Arc::new(move |a: &Activity| {
        let mut last = shown_at.lock().unwrap_or_else(PoisonError::into_inner);
        let line = if let Some(p) = a.paused {
            Some(l.t_args("computer-paused-line", &args!("by" => p.key())))
        } else if a.active && a.at.saturating_sub(*last) > ACTIVE_MS {
            Some(l.t_args(
                "computer-active-line",
                &args!("app" => a.app.clone().unwrap_or_else(|| "?".into())),
            ))
        } else {
            None
        };
        if a.active {
            *last = a.at;
        }
        if let Some(line) = line {
            let mut e = std::io::stderr().lock();
            let _ = writeln!(e, "{line}");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_expires() {
        let a = Activity {
            active: true,
            tool: "left_click".into(),
            app: Some("mspaint".into()),
            paused: None,
            at: now_ms() - ACTIVE_MS - 10,
        };
        *LAST.lock().unwrap() = Some(a);
        assert!(!current().active);
    }
}
