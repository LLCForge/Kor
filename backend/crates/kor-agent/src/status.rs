//! `agent.status {verb, elapsed, tokens}` (spec §16.2 rule 2, §16.3, ADR 0053).
//!
//! The thinking verb is chosen here, in the core, so the CLI spinner, the TUI and the desktop
//! `KorStatus` show the same word at the same moment. Surfaces only draw: `elapsed_ms` comes
//! from an [`Instant`] in the core and `tokens` from the provider's usage reports.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::AgentEvent;

const VERBS_JSON: &str = include_str!("../../../../assets/verbs.json");

/// How often a status event is sent while a task runs.
pub const TICK: Duration = Duration::from_secs(1);
/// How long one verb stays before the next one.
pub const VERB_EVERY: Duration = Duration::from_secs(4);

/// One status snapshot, sent as [`AgentEvent::Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    /// The verb in the status language (`Közlüyor`, `Forging`, …), without the ellipsis.
    pub verb: String,
    /// Index into the verb list, so a surface can show the same verb in another language.
    pub verb_index: usize,
    /// Milliseconds since the task started.
    pub elapsed_ms: u64,
    /// Input + output tokens the provider reported for this task so far.
    pub tokens: u64,
}

static VERBS: LazyLock<[Vec<String>; 3]> = LazyLock::new(|| {
    let v: serde_json::Value = serde_json::from_str(VERBS_JSON).unwrap_or_default();
    let list = |lang: &str, fallback: &str| -> Vec<String> {
        let l: Vec<String> = v[lang]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if l.is_empty() {
            vec![fallback.to_owned()]
        } else {
            l
        }
    };
    [
        list("tr", "Düşünüyor"),
        list("en", "Thinking"),
        list("fr", "Réfléchit"),
    ]
});

/// 0 = tr, 1 = en, 2 = fr.
static LANG: AtomicU8 = AtomicU8::new(0);

fn lang_index(code: &str) -> u8 {
    match code {
        "en" => 1,
        "fr" => 2,
        _ => 0,
    }
}

/// Selects the language of [`AgentStatus::verb`] (`tr`, `en`, `fr`; anything else is `tr`).
/// `kor_runtime::prepare` calls it with the user's `ui.language`.
pub fn set_lang(code: &str) {
    LANG.store(lang_index(code), Ordering::Relaxed);
}

/// The verb list of a language.
pub fn verbs(code: &str) -> &'static [String] {
    &VERBS[usize::from(lang_index(code))]
}

/// The status of a task that started `elapsed` ago, starting at verb `offset`.
pub fn status_at(offset: usize, elapsed: Duration, tokens: u64) -> AgentStatus {
    let list = &VERBS[usize::from(LANG.load(Ordering::Relaxed))];
    let step = usize::try_from(elapsed.as_secs() / VERB_EVERY.as_secs()).unwrap_or(0);
    let verb_index = offset.wrapping_add(step) % list.len();
    AgentStatus {
        verb: list[verb_index].clone(),
        verb_index,
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        tokens,
    }
}

/// A running status ticker; [`Ticker::finish`] flushes and stops it.
#[derive(Debug)]
pub struct Ticker {
    handle: JoinHandle<()>,
    done: oneshot::Sender<()>,
}

impl Ticker {
    /// Forwards what is still queued, then stops. Senders that outlive the task (a tool's
    /// progress sink kept by a background job) then fail quietly instead of keeping it alive.
    pub async fn finish(self) {
        let _ = self.done.send(());
        let _ = self.handle.await;
    }
}

/// Puts a status ticker between the agent loop and `events`: every event is forwarded in
/// order, usage is summed into `tokens`, and an [`AgentEvent::Status`] goes out at the start and
/// every [`TICK`].
pub fn forward(events: UnboundedSender<AgentEvent>) -> (UnboundedSender<AgentEvent>, Ticker) {
    let (tx, mut rx) = unbounded_channel::<AgentEvent>();
    let (done, mut done_rx) = oneshot::channel::<()>();
    let started = Instant::now();
    // A different verb each task, chosen from the clock (no RNG dependency).
    let offset = usize::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0);
    let handle = tokio::spawn(async move {
        let mut tokens = 0u64;
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let pass = |ev: AgentEvent, tokens: &mut u64| {
            if let AgentEvent::Usage(u) = &ev {
                *tokens = tokens.saturating_add(u.input).saturating_add(u.output);
            }
            events.send(ev).is_ok()
        };
        // The status at the start, before any queued event can end the task (`biased` below).
        tick.tick().await;
        let _ = events.send(AgentEvent::Status(status_at(offset, started.elapsed(), 0)));
        loop {
            tokio::select! {
                biased;
                ev = rx.recv() => {
                    let Some(ev) = ev else { break };
                    if !pass(ev, &mut tokens) {
                        break;
                    }
                }
                _ = &mut done_rx => {
                    while let Ok(ev) = rx.try_recv() {
                        if !pass(ev, &mut tokens) {
                            break;
                        }
                    }
                    break;
                }
                _ = tick.tick() => {
                    let _ = events.send(AgentEvent::Status(status_at(offset, started.elapsed(), tokens)));
                }
            }
        }
    });
    (tx, Ticker { handle, done })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_load_from_assets() {
        for code in ["tr", "en", "fr"] {
            assert!(verbs(code).len() >= 10, "{code}");
        }
        assert_eq!(verbs("tr")[1], "Közlüyor");
        assert_eq!(verbs("xx")[0], verbs("tr")[0]);
    }

    #[test]
    fn verb_rotates_every_four_seconds_and_reports_elapsed() {
        let a = status_at(0, Duration::from_millis(3_900), 7);
        let b = status_at(0, Duration::from_millis(4_100), 7);
        assert_eq!(a.verb_index, 0);
        assert_eq!(b.verb_index, 1);
        assert_eq!(b.elapsed_ms, 4_100);
        assert_eq!(b.tokens, 7);
        let wrap = status_at(verbs("tr").len() - 1, Duration::from_secs(4), 0);
        assert_eq!(wrap.verb_index, 0);
    }

    #[tokio::test]
    async fn forward_keeps_order_counts_tokens_and_ticks() {
        let (out, mut rx) = unbounded_channel();
        let (tx, ticker) = forward(out);
        tx.send(AgentEvent::Text("a".into())).expect("send");
        tx.send(AgentEvent::Usage(kor_core::Usage {
            input: 10,
            output: 5,
            ..kor_core::Usage::default()
        }))
        .expect("send");
        tx.send(AgentEvent::Text("b".into())).expect("send");
        // A lingering clone (like a kept progress sink) must not keep the ticker alive.
        let lingering = tx.clone();
        drop(tx);
        ticker.finish().await;
        assert!(lingering.send(AgentEvent::Text("late".into())).is_err());
        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            seen.push(ev);
        }
        let texts: Vec<_> = seen
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["a", "b"]);
        // The first interval tick fires immediately, so a status is always sent.
        assert!(seen.iter().any(|e| matches!(e, AgentEvent::Status(_))));
    }
}
