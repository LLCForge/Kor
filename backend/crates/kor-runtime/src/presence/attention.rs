//! "Call me" (§28.4): which channels an event uses, from `[notify]` settings.
//!
//! - A muted category is only stored (bell list), nothing else.
//! - The window is focused and the user was active within `idle_secs`: quiet inline highlight.
//! - Otherwise: OS toast (`toast`), sound (`sound`), and for events that wait for the user
//!   (approval, question) taskbar flash / Dock bounce (`flash`) and terminal `BEL` (`bell`).
//! - Quiet hours silence sound, `BEL` and flash; the toast still appears (silently).

use serde::{Deserialize, Serialize};

use super::notifications::Category;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent channels
pub struct Policy {
    pub toast: bool,
    pub bell: bool,
    pub flash: bool,
    pub sound: bool,
    pub idle_secs: u64,
    /// Start and end hour (local), end exclusive; may wrap midnight.
    pub quiet: Option<(u8, u8)>,
    pub muted: Vec<Category>,
}

impl From<&kor_config::Notify> for Policy {
    fn from(n: &kor_config::Notify) -> Self {
        Self {
            toast: n.toast,
            bell: n.bell,
            flash: n.flash,
            sound: n.sound,
            idle_secs: n.idle_secs,
            quiet: quiet_hours(&n.quiet_hours),
            muted: n.muted.iter().filter_map(|m| Category::parse(m)).collect(),
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::from(&kor_config::Notify::default())
    }
}

/// Where the user is.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    pub focused: bool,
    /// Seconds since the last key / mouse input in Kor's window.
    pub idle_secs: u64,
    /// Local hour 0–23.
    pub hour: u8,
    /// Notifications paused from the tray.
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // independent channels
pub struct Alert {
    /// Only an inline highlight in the open window.
    pub inline: bool,
    pub toast: bool,
    pub sound: bool,
    pub flash: bool,
    pub bell: bool,
}

/// `"22-08"` → `(22, 8)`; empty or invalid → none.
pub fn quiet_hours(s: &str) -> Option<(u8, u8)> {
    let (a, b) = s.trim().split_once('-')?;
    let a: u8 = a.trim().parse().ok()?;
    let b: u8 = b.trim().parse().ok()?;
    (a < 24 && b < 24 && a != b).then_some((a, b))
}

pub fn in_quiet(q: Option<(u8, u8)>, hour: u8) -> bool {
    match q {
        None => false,
        Some((a, b)) if a < b => (a..b).contains(&hour),
        Some((a, b)) => hour >= a || hour < b,
    }
}

pub fn decide(p: &Policy, cat: Category, cx: Context) -> Alert {
    if p.muted.contains(&cat) || cx.paused {
        return Alert::default();
    }
    if cx.focused && cx.idle_secs < p.idle_secs {
        return Alert {
            inline: true,
            ..Alert::default()
        };
    }
    let quiet = in_quiet(p.quiet, cx.hour);
    let waits = cat.waits_for_user();
    Alert {
        inline: false,
        toast: p.toast,
        sound: p.sound && !quiet,
        flash: p.flash && waits && !quiet,
        bell: p.bell && waits && !quiet,
    }
}

/// Terminal escape for "Kor waits": `BEL` and the window title (`OSC 0`), when allowed now.
/// Control characters in the title are removed.
/// §28.4 sound for the CLI/TUI: on when `notify.sound` is set, outside quiet hours, category not muted.
pub fn terminal_sound(p: &Policy, cat: Category, hour: u8) -> bool {
    p.sound && !p.muted.contains(&cat) && !in_quiet(p.quiet, hour)
}

pub fn terminal_call(p: &Policy, cat: Category, hour: u8, title: &str) -> Option<String> {
    if p.muted.contains(&cat) || !p.bell || in_quiet(p.quiet, hour) {
        return None;
    }
    let clean: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    Some(format!("\x1b]0;{clean}\x07\x07"))
}

/// A budget limit reached `alert_at` percent (§27.2 budget notifications; the agent's own
/// on-limit action is unchanged).
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetAlert {
    /// `daily` or `monthly`.
    pub scope: &'static str,
    pub spent: f64,
    pub limit: f64,
    pub percent: u32,
    pub estimated: bool,
}

/// The fullest limit at or over `alert_at` percent of the cost report, if any.
pub fn budget_alert(r: &crate::cost::Report, alert_at: u8) -> Option<BudgetAlert> {
    [
        ("daily", r.daily_limit_usd, &r.today),
        ("monthly", r.monthly_limit_usd, &r.month),
    ]
    .into_iter()
    .filter(|(_, limit, _)| *limit > 0.0)
    .map(|(scope, limit, row)| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a percentage
        let percent = (row.usd / limit * 100.0).floor().max(0.0) as u32;
        BudgetAlert {
            scope,
            spent: row.usd,
            limit,
            percent,
            estimated: row.estimated,
        }
    })
    .filter(|a| a.percent >= u32::from(alert_at))
    .max_by_key(|a| a.percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(focused: bool, idle: u64, hour: u8) -> Context {
        Context {
            focused,
            idle_secs: idle,
            hour,
            paused: false,
        }
    }

    #[test]
    fn active_focused_user_gets_inline_only() {
        let p = Policy::default();
        let a = decide(&p, Category::Approval, cx(true, 5, 12));
        assert!(a.inline && !a.toast && !a.flash && !a.sound);
    }

    #[test]
    fn idle_or_away_user_is_called() {
        let p = Policy::default();
        for c in [cx(true, 31, 12), cx(false, 0, 12)] {
            let a = decide(&p, Category::Approval, c);
            assert!(!a.inline && a.toast && a.flash && a.sound && a.bell);
        }
        let done = decide(&p, Category::Done, cx(false, 0, 12));
        assert!(done.toast && !done.flash && !done.bell);
    }

    #[test]
    fn quiet_hours_mute_and_paused_mute_all() {
        let mut p = Policy {
            quiet: quiet_hours("22-08"),
            ..Policy::default()
        };
        let a = decide(&p, Category::Approval, cx(false, 0, 23));
        assert!(a.toast && !a.sound && !a.flash && !a.bell);
        assert!(decide(&p, Category::Approval, cx(false, 0, 9)).sound);
        p.muted = vec![Category::Done];
        assert_eq!(
            decide(&p, Category::Done, cx(false, 0, 12)),
            Alert::default()
        );
        let paused = Context {
            paused: true,
            ..cx(false, 0, 12)
        };
        assert_eq!(decide(&p, Category::Approval, paused), Alert::default());
    }

    #[test]
    fn quiet_hour_parsing_and_wrapping() {
        assert_eq!(quiet_hours(" 9 - 17 "), Some((9, 17)));
        assert_eq!(quiet_hours("24-1"), None);
        assert_eq!(quiet_hours("5-5"), None);
        assert_eq!(quiet_hours(""), None);
        assert!(in_quiet(Some((9, 17)), 9) && !in_quiet(Some((9, 17)), 17));
        assert!(in_quiet(Some((22, 8)), 2) && !in_quiet(Some((22, 8)), 8));
    }

    #[test]
    fn zero_idle_secs_always_notifies() {
        let p = Policy {
            idle_secs: 0,
            ..Policy::default()
        };
        let a = decide(&p, Category::Approval, cx(true, 0, 12));
        assert!(!a.inline && a.toast, "idle_secs = 0 means always notify");
    }

    #[test]
    fn terminal_call_strips_controls() {
        let p = Policy::default();
        let s = terminal_call(&p, Category::Approval, 12, "Kor\x1b]evil\x07 waits").unwrap();
        assert_eq!(s, "\x1b]0;Kor]evil waits\x07\x07");
        let off = Policy {
            bell: false,
            ..Policy::default()
        };
        assert!(terminal_call(&off, Category::Approval, 12, "x").is_none());
    }

    #[test]
    fn budget_alert_picks_the_fullest_limit_over_the_threshold() {
        let mut r = crate::cost::Report {
            daily_limit_usd: 10.0,
            monthly_limit_usd: 100.0,
            ..crate::cost::Report::default()
        };
        r.today.usd = 7.0;
        r.month.usd = 50.0;
        assert!(budget_alert(&r, 80).is_none());
        r.today.usd = 8.5;
        r.month.usd = 95.0;
        let a = budget_alert(&r, 80).unwrap();
        assert_eq!((a.scope, a.percent), ("monthly", 95));
        r.monthly_limit_usd = 0.0;
        assert_eq!(budget_alert(&r, 80).unwrap().scope, "daily");
        r.daily_limit_usd = 0.0;
        assert!(budget_alert(&r, 80).is_none());
    }
}
