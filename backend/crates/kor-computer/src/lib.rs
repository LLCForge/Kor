//! Desktop control for the Kor coding agent (spec §19.1, §27.7, ADR 0059).
//!
//! The model-facing tools live in `kor-tools` (`screenshot`, `left_click`, `uia_invoke`,
//! `move_window`, …); this crate holds what they share and must not differ between tools:
//!
//! - [`policy`]: per-app tiers (`blocked`, `read`, `click`, `ask`, `full`), built-in defaults by
//!   app class and `computer.apps.<app>` overrides;
//! - [`guard`]: "human first" — the user's mouse or keyboard pauses Kor until they approve;
//! - [`indicator`]: one listener sees every action (desktop bar, CLI line);
//! - audit: every action, refusal and pause goes to the audit sink (`kor_agent::audit`);
//! - [`keys`] and (feature `live`) [`live`]: screen capture and input;
//! - [`layout`]: saved window arrangements across monitors (`kor layout`).
//!
//! A desktop tool calls [`begin`] before it acts and [`Ticket::finish`] after.

pub mod guard;
pub mod indicator;
pub mod keys;
pub mod layout;
#[cfg(feature = "live")]
pub mod live;
pub mod policy;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use serde_json::{Value, json};

use crate::guard::{Guard, InputWatcher, Interruption, OsWatcher};
use crate::indicator::Activity;
use crate::policy::{Action, Verdict};

pub type AuditSink = Arc<dyn Fn(&str, Value) + Send + Sync>;

static AUDIT: RwLock<Option<AuditSink>> = RwLock::new(None);
static GUARD: Mutex<Option<Guard>> = Mutex::new(None);

/// Where computer actions are recorded (`kor-runtime` connects the hash-chained audit log).
pub fn set_audit_sink(sink: AuditSink) {
    *AUDIT.write().unwrap_or_else(PoisonError::into_inner) = Some(sink);
}

fn audit(fields: Value) {
    let sink = AUDIT.read().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(s) = sink {
        s("computer", fields);
    }
}

fn guard() -> MutexGuard<'static, Option<Guard>> {
    let mut g = GUARD.lock().unwrap_or_else(PoisonError::into_inner);
    if g.is_none() {
        *g = Some(Guard::new(Box::new(OsWatcher)));
    }
    g
}

fn with_guard<R>(f: impl FnOnce(&mut Guard) -> R) -> R {
    let mut g = guard();
    f(g.as_mut()
        .unwrap_or_else(|| unreachable!("guard() fills the slot")))
}

/// Replace the input watcher (tests use a fake one).
pub fn set_watcher(w: Box<dyn InputWatcher>) {
    *GUARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(Guard::new(w));
}

/// The app an action targets: `explicit` (a window's or program's app) or the app in front.
pub fn target_app(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(kor_platform::desktop_probe::app_name)
        .filter(|a| !a.is_empty())
        .or_else(kor_platform::desktop_probe::foreground_app)
}

/// Text the model gets when the user interrupted.
pub fn pause_message(i: Interruption) -> String {
    let what = match i {
        Interruption::Mouse => "moved the mouse",
        Interruption::Input => "pressed a key or used the mouse",
    };
    format!(
        "Paused: the user {what} while Kor controlled the desktop (human first). Nothing was done. Ask the user whether to continue; the next desktop action needs their approval."
    )
}

/// One action in progress.
#[derive(Debug)]
#[must_use = "call finish() after the action"]
pub struct Ticket {
    tool: String,
    app: Option<String>,
    action: Action,
}

impl Ticket {
    /// After the action ran (`ok`: it succeeded).
    pub fn finish(self, ok: bool) {
        with_guard(Guard::done);
        audit(json!({
            "tool": self.tool,
            "app": self.app,
            "action": self.action.key(),
            "decision": "run",
            "ok": ok,
        }));
    }
}

/// Before a desktop tool acts: the app's tier allows it and the user has not taken over.
/// `Err` is the message for the model (already audited).
pub fn begin(
    tool: &str,
    action: Action,
    app: Option<String>,
    destructive: bool,
) -> Result<Ticket, String> {
    if let Verdict::Refuse(m) =
        policy::verdict(&policy::current(), app.as_deref(), action, destructive)
    {
        audit(json!({"tool": tool, "app": app, "action": action.key(), "decision": "refused"}));
        return Err(m);
    }
    if let Err(i) = with_guard(Guard::check) {
        audit(
            json!({"tool": tool, "app": app, "action": action.key(), "decision": "paused", "by": i.key()}),
        );
        indicator::emit(Activity {
            active: false,
            tool: tool.to_owned(),
            app,
            paused: Some(i),
            at: indicator::now_ms(),
        });
        return Err(pause_message(i));
    }
    indicator::emit(Activity {
        active: true,
        tool: tool.to_owned(),
        app: app.clone(),
        paused: None,
        at: indicator::now_ms(),
    });
    Ok(Ticket {
        tool: tool.to_owned(),
        app,
        action,
    })
}

/// The permission engine may run this action without asking: `full` tier, not destructive, and
/// Kor is not paused by the user.
pub fn auto_allowed(action: Action, app: Option<&str>, destructive: bool) -> bool {
    paused().is_none()
        && policy::verdict(&policy::current(), app, action, destructive) == Verdict::Allow
}

/// Kor is paused by the user (the next desktop action asks, in every permission mode).
pub fn paused() -> Option<Interruption> {
    with_guard(|g| g.paused())
}

/// The user approved a desktop action: resume and treat their approval click as theirs.
pub fn approved() {
    with_guard(Guard::approved);
}

/// The user pressed "stop" on the indicator: pause as if they took over (the next desktop action
/// asks in every mode).
pub fn interrupt() {
    with_guard(|g| g.interrupt(Interruption::Input));
    indicator::emit(Activity {
        active: false,
        tool: String::new(),
        app: None,
        paused: Some(Interruption::Input),
        at: indicator::now_ms(),
    });
}

/// The input watcher sees input on this desktop (false e.g. on Wayland without `xprintidle`).
pub fn watching() -> bool {
    with_guard(|g| g.watching())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::guard::fake::Fake;
    use crate::policy::{Policy, Tier};

    #[test]
    fn begin_enforces_tiers_pauses_on_the_user_and_audits_everything() {
        let log: Arc<Mutex<Vec<Value>>> = Arc::default();
        let l2 = log.clone();
        set_audit_sink(Arc::new(move |_, v| l2.lock().unwrap().push(v)));
        let seen: Arc<Mutex<Vec<Activity>>> = Arc::default();
        let s2 = seen.clone();
        indicator::set_listener(Arc::new(move |a| s2.lock().unwrap().push(a.clone())));
        let f = Fake::default();
        f.advance(10_000);
        set_watcher(Box::new(f.clone()));
        let mut p = Policy::default();
        p.apps.insert("mspaint".into(), Tier::Full);
        policy::set_policy(p);

        assert!(begin("type", Action::Type, Some("pwsh".into()), false).is_err());
        assert!(begin("left_click", Action::Click, Some("chrome".into()), false).is_err());
        assert!(auto_allowed(Action::Type, Some("mspaint"), false));
        assert!(!auto_allowed(Action::Type, Some("mspaint"), true));

        begin("type", Action::Type, Some("mspaint".into()), false)
            .unwrap()
            .finish(true);
        f.advance(3_000);
        f.key();
        f.advance(1_000);
        let err = begin("left_click", Action::Click, Some("mspaint".into()), false).unwrap_err();
        assert!(err.starts_with("Paused"), "{err}");
        assert!(paused().is_some() && !auto_allowed(Action::Type, Some("mspaint"), false));
        approved();
        begin("left_click", Action::Click, Some("mspaint".into()), false)
            .unwrap()
            .finish(false);

        let log = log.lock().unwrap();
        let decisions: Vec<&str> = log
            .iter()
            .map(|v| v["decision"].as_str().unwrap())
            .collect();
        assert_eq!(decisions, ["refused", "refused", "run", "paused", "run"]);
        assert_eq!(log[2]["app"], "mspaint");
        assert_eq!(log[4]["ok"], false);
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|a| a.paused == Some(Interruption::Input)));
        assert!(seen.iter().filter(|a| a.active).count() >= 2);
    }
}
