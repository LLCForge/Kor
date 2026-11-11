//! "Human first" (§19.1): when the user moves the mouse or presses a key while Kor controls the
//! desktop, the next action is not run; Kor pauses until the user approves the next step.
//!
//! The watcher reports the time since the last input (Kor's own injected input counts too) and the
//! pointer position. After each action the guard remembers when it ended and where the pointer
//! was; before the next one, input newer than that (plus a short grace for injected events still
//! being delivered) or a moved pointer means a human took over.

use std::fmt::Debug;
use std::sync::OnceLock;
use std::time::Instant;

use serde::Serialize;

/// Injected input can be delivered a little after the call that sent it returns.
pub const GRACE_MS: u64 = 250;
/// Pointer movement below this (pixels on either axis) is jitter, not the user.
const CURSOR_SLACK: i32 = 3;

/// Source of "when did the user last touch the machine".
pub trait InputWatcher: Send + Sync + Debug {
    /// Milliseconds since the last keyboard / mouse input, `None` when unknown.
    fn idle_ms(&self) -> Option<u64>;
    /// Pointer position, `None` when unknown.
    fn cursor(&self) -> Option<(i32, i32)>;
    /// Monotonic milliseconds (tests drive their own clock).
    fn now_ms(&self) -> u64 {
        static START: OnceLock<Instant> = OnceLock::new();
        u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// The OS watcher: `kor_platform::desktop_probe::idle_ms` and the input library's pointer.
#[derive(Debug, Default)]
pub struct OsWatcher;

impl InputWatcher for OsWatcher {
    fn idle_ms(&self) -> Option<u64> {
        kor_platform::desktop_probe::idle_ms()
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        #[cfg(feature = "live")]
        {
            crate::live::cursor()
        }
        #[cfg(not(feature = "live"))]
        {
            None
        }
    }
}

/// What the user did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Interruption {
    /// The pointer moved.
    Mouse,
    /// A key press or other input (keyboard, wheel, touch).
    Input,
}

impl Interruption {
    pub fn key(self) -> &'static str {
        match self {
            Self::Mouse => "mouse",
            Self::Input => "input",
        }
    }
}

#[derive(Debug)]
pub struct Guard {
    watcher: Box<dyn InputWatcher>,
    /// End of Kor's last action and the pointer then.
    last: Option<(u64, Option<(i32, i32)>)>,
    paused: Option<Interruption>,
}

impl Guard {
    pub fn new(watcher: Box<dyn InputWatcher>) -> Self {
        Self {
            watcher,
            last: None,
            paused: None,
        }
    }

    pub fn paused(&self) -> Option<Interruption> {
        self.paused
    }

    /// Before an action: `Err` when the user took over since Kor's last action (and from then on,
    /// until [`Guard::approved`]).
    pub fn check(&mut self) -> Result<(), Interruption> {
        if let Some(p) = self.paused {
            return Err(p);
        }
        let Some((at, cursor)) = self.last else {
            return Ok(());
        };
        let since = self.watcher.now_ms().saturating_sub(at);
        let moved = match (cursor, self.watcher.cursor()) {
            (Some((x0, y0)), Some((x1, y1))) => {
                (x1 - x0).abs() > CURSOR_SLACK || (y1 - y0).abs() > CURSOR_SLACK
            }
            _ => false,
        };
        let touched = self
            .watcher
            .idle_ms()
            .is_some_and(|idle| idle.saturating_add(GRACE_MS) < since);
        let hit = if moved {
            Some(Interruption::Mouse)
        } else if touched {
            Some(Interruption::Input)
        } else {
            None
        };
        match hit {
            Some(i) => {
                self.paused = Some(i);
                Err(i)
            }
            None => Ok(()),
        }
    }

    /// After an action: input before this moment was Kor's own.
    pub fn done(&mut self) {
        self.last = Some((self.watcher.now_ms(), self.watcher.cursor()));
    }

    /// The user approved the next step (the click on "approve" is theirs, not an interruption).
    pub fn approved(&mut self) {
        self.paused = None;
        self.last = None;
    }

    /// Pause without waiting for the watcher (the indicator's stop button).
    pub fn interrupt(&mut self, i: Interruption) {
        self.paused = Some(i);
    }

    /// Whether the watcher can see input at all on this desktop.
    pub fn watching(&self) -> bool {
        self.watcher.idle_ms().is_some() || self.watcher.cursor().is_some()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

    use super::InputWatcher;

    /// A watcher the test moves by hand.
    #[derive(Debug, Default, Clone)]
    pub struct Fake {
        pub now: Arc<AtomicU64>,
        pub last_input: Arc<AtomicU64>,
        pub x: Arc<AtomicI32>,
    }

    impl Fake {
        pub fn advance(&self, ms: u64) {
            self.now.fetch_add(ms, Ordering::SeqCst);
        }
        /// The user touches the keyboard now.
        pub fn key(&self) {
            self.last_input
                .store(self.now.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        pub fn move_mouse(&self, dx: i32) {
            self.x.fetch_add(dx, Ordering::SeqCst);
            self.key();
        }
    }

    impl InputWatcher for Fake {
        fn idle_ms(&self) -> Option<u64> {
            Some(self.now.load(Ordering::SeqCst) - self.last_input.load(Ordering::SeqCst))
        }
        fn cursor(&self) -> Option<(i32, i32)> {
            Some((self.x.load(Ordering::SeqCst), 10))
        }
        fn now_ms(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;

    #[test]
    fn kor_own_input_does_not_trip_but_the_users_does() {
        let f = Fake::default();
        f.advance(1000);
        let mut g = Guard::new(Box::new(f.clone()));
        assert!(g.check().is_ok(), "first action");
        // Kor's injected input lands just before the action ends.
        f.key();
        f.advance(100);
        g.done();
        f.advance(5_000);
        assert!(g.check().is_ok(), "nobody touched anything");
        g.done();

        f.advance(2_000);
        f.key();
        f.advance(1_000);
        assert_eq!(g.check(), Err(Interruption::Input));
        assert_eq!(g.check(), Err(Interruption::Input), "stays paused");
        g.approved();
        assert!(g.check().is_ok());
        g.done();

        f.advance(500);
        f.move_mouse(40);
        assert_eq!(g.check(), Err(Interruption::Mouse));
        assert!(g.watching());
    }

    #[test]
    fn injected_input_within_the_grace_is_kors() {
        let f = Fake::default();
        f.advance(10_000);
        let mut g = Guard::new(Box::new(f.clone()));
        g.done();
        f.advance(200);
        f.key(); // delivered late, 200 ms after the call returned
        f.advance(300);
        assert!(g.check().is_ok());
    }
}
