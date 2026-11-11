//! Kor brand kit in the terminal: the gradient banner (from `assets/`) and the ember spinner.
//! Colours and frames come from `kor_core::theme` (generated from `assets/theme.json`); the
//! verb, elapsed time and tokens come from the core's `agent.status` event (§16.2): the CLI
//! only draws them, so it always shows the same word as the desktop.

use std::sync::atomic::{AtomicBool, Ordering};

use kor_agent::status::AgentStatus;
use kor_core::theme;

const BANNER: &str = include_str!("../../../../assets/banner.txt");
const BANNER_SMALL: &str = include_str!("../../../../assets/banner-small.txt");

/// Set while a permission prompt owns the terminal, so the spinner does not draw over it.
pub static PAUSED: AtomicBool = AtomicBool::new(false);

/// How many colours the terminal shows (§16.2 rule 3): truecolor, the 256-colour palette, or none
/// (`NO_COLOR`, `TERM=dumb`, not a terminal) — then nothing animates and the meaning stays in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    None,
    Ansi256,
    TrueColor,
}

impl Depth {
    /// This process's terminal.
    pub fn detect() -> Self {
        use std::io::IsTerminal as _;
        Self::from_env(
            std::env::var_os("NO_COLOR").is_some(),
            std::env::var("TERM").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
            std::io::stderr().is_terminal() && std::io::stdout().is_terminal(),
        )
    }

    /// `COLORTERM=truecolor|24bit` (and Windows consoles, which set no `TERM`) get truecolor;
    /// other terminals the 256-colour palette.
    pub fn from_env(no_color: bool, term: Option<&str>, colorterm: Option<&str>, tty: bool) -> Self {
        if no_color || !tty || term == Some("dumb") {
            return Self::None;
        }
        let truecolor = colorterm.is_some_and(|c| c.eq_ignore_ascii_case("truecolor") || c.eq_ignore_ascii_case("24bit"));
        if truecolor || term.is_none() { Self::TrueColor } else { Self::Ansi256 }
    }
}

/// Nearest colour of the xterm 6×6×6 cube (16–231).
fn ansi256((r, g, b): theme::Rgb) -> u8 {
    // Nearest xterm cube level (0, 95, 135, 175, 215, 255), not a linear split of 0–255.
    let q = |c: u8| match c {
        0..48 => 0u16,
        48..115 => 1,
        _ => (u16::from(c) - 35) / 40,
    };
    u8::try_from(16 + 36 * q(r) + 6 * q(g) + q(b)).unwrap_or(theme::ansi256::EMBER)
}

fn paint(depth: Depth, c: theme::Rgb, s: &str) -> String {
    let (r, g, b) = c;
    match depth {
        Depth::None => s.to_owned(),
        Depth::Ansi256 => format!("\x1b[38;5;{}m{s}\x1b[39m", ansi256(c)),
        Depth::TrueColor => format!("\x1b[38;2;{r};{g};{b}m{s}\x1b[39m"),
    }
}

/// Dim text; plain without colour.
fn dim(depth: Depth, s: &str) -> String {
    if depth == Depth::None { s.to_owned() } else { format!("\x1b[2m{s}\x1b[22m") }
}

/// Below this many columns the small banner is shown (§16.1).
pub const NARROW_COLUMNS: u16 = 60;

/// The banner for this terminal: small below [`NARROW_COLUMNS`], coloured as [`Depth::detect`] allows.
pub fn banner_here() -> String {
    let narrow = crossterm::terminal::size().is_ok_and(|(w, _)| w < NARROW_COLUMNS);
    banner_depth(Depth::detect(), narrow)
}

/// The banner, one gradient colour per line; plain when `color` is off.
#[cfg(test)]
pub fn banner(color: bool, narrow: bool) -> String {
    banner_depth(if color { Depth::TrueColor } else { Depth::None }, narrow)
}

/// The banner at a colour depth.
pub fn banner_depth(depth: Depth, narrow: bool) -> String {
    let text = if narrow { BANNER_SMALL } else { BANNER };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let g = &theme::GRADIENT;
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| paint(depth, g[i * g.len() / lines.len().max(1)], l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The spinner frame set in use (`ui.spinner`, §16.1): index into [`FRAME_SETS`].
static STYLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Theme frame sets by `ui.spinner` name; `ember` first (the default).
pub const FRAME_SETS: [(&str, &[&str]); 6] = [
    ("ember", &theme::frames::EMBER),
    ("braille", &theme::frames::BRAILLE),
    ("flame", &theme::frames::FLAME),
    ("cube", &theme::frames::CUBE),
    ("spark", &theme::frames::SPARK),
    ("forge", &theme::frames::FORGE),
];

/// Choose the spinner frames by name; an unknown name keeps `ember`.
pub fn set_spinner_style(name: &str) {
    let i = FRAME_SETS.iter().position(|(n, _)| *n == name).unwrap_or(0);
    STYLE.store(i, Ordering::Relaxed);
}

/// The frames of the chosen style.
pub fn spinner_frames() -> &'static [&'static str] {
    FRAME_SETS[STYLE.load(Ordering::Relaxed).min(FRAME_SETS.len() - 1)].1
}

/// One spinner line on stderr: `✶ Közlüyor… (3s)`.
#[derive(Debug, Default)]
pub struct Spinner {
    tick: usize,
    drawn: bool,
    /// The core's latest `agent.status`; nothing is drawn before the first one.
    status: Option<AgentStatus>,
    /// §28.1: context window use of the last request, in percent.
    ctx: Option<u64>,
    /// §27.16: the task's estimate, and the tokens spent so far.
    est: Option<kor_agent::cost::Preflight>,
    tokens: (u64, u64),
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the core's `agent.status` (verb, elapsed, tokens).
    pub fn set_status(&mut self, status: AgentStatus) {
        self.status = Some(status);
    }

    /// §28.1 `ctx 62%` on the spinner line.
    pub fn set_context(&mut self, percent: u64) {
        self.ctx = Some(percent);
    }

    /// §27.16 `(1m12s / ≈4m · $0.011 / ≈$0.03)` on the spinner line.
    pub fn set_estimate(&mut self, p: kor_agent::cost::Preflight) {
        self.est = Some(p);
        self.tokens = (0, 0);
    }

    pub fn add_usage(&mut self, input: u64, output: u64) {
        self.tokens.0 += input;
        self.tokens.1 += output;
    }

    /// Elapsed vs estimate and spend vs estimate, and whether the run drifts far from it.
    fn progress(&self, secs: u64) -> Option<(String, bool)> {
        let p = self.est.as_ref()?;
        #[allow(clippy::cast_precision_loss)] // token counts far below 2^52
        let spent = p.price.map_or(0.0, |(i, o)| (self.tokens.0 as f64 * i + self.tokens.1 as f64 * o) / 1_000_000.0);
        let e = kor_runtime::estimate::Estimate {
            secs: p.secs,
            usd: p.usd,
            files: p.files,
            turns: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            method: kor_runtime::estimate::Method::Prior,
            samples: 0,
            category: None,
            model: String::new(),
            local: false,
            correction: 1.0,
        };
        let text = kor_runtime::estimate::progress(secs * 1000, spent, &e);
        Some((text, kor_runtime::estimate::drifting(secs * 1000, spent, &e)))
    }

    /// The text of the current frame (without terminal control codes): frame, verb, seconds.
    pub fn frame(&self) -> Option<(&'static str, &str, u64)> {
        let s = self.status.as_ref()?;
        let frames = spinner_frames();
        Some((
            frames[self.tick % frames.len()],
            &s.verb,
            s.elapsed_ms / 1000,
        ))
    }

    /// The current frame at a colour depth, without the line-clearing prefix; `None` before the
    /// core's first `agent.status`.
    pub fn line(&self, depth: Depth) -> Option<String> {
        let (f, verb, secs) = self.frame()?;
        let g = &theme::GRADIENT;
        let color = g[self.tick % g.len()];
        let ctx = self.ctx.map(|p| format!(" · ctx {p}%")).unwrap_or_default();
        Some(match self.progress(secs) {
            // Yellow when the run is far past its estimate (§27.16).
            Some((est, drift)) => {
                let est = match depth {
                    Depth::None => est,
                    _ if drift => format!("\x1b[33m{est}\x1b[0m"),
                    _ => dim(depth, &est),
                };
                let hint = ctx.trim_start_matches(" · ").to_owned() + if ctx.is_empty() { "" } else { " · " };
                format!(
                    "{} {} {est} {}",
                    paint(depth, color, f),
                    paint(depth, theme::ember::MID, &format!("{verb}…")),
                    dim(depth, &format!("({hint}esc/ctrl+c)"))
                )
            }
            None => format!(
                "{} {} {}",
                paint(depth, color, f),
                paint(depth, theme::ember::MID, &format!("{verb}…")),
                dim(depth, &format!("({secs}s{ctx} · esc/ctrl+c)"))
            ),
        })
    }

    pub fn draw(&mut self) {
        use std::io::Write as _;
        if PAUSED.load(Ordering::Relaxed) {
            self.clear();
            return;
        }
        let Some(text) = self.line(Depth::detect()) else {
            return;
        };
        let line = format!("\r\x1b[2K{text}");
        let mut err = std::io::stderr();
        let _ = write!(err, "{line}");
        let _ = err.flush();
        self.drawn = true;
        self.tick += 1;
    }

    /// Erase the line if something was drawn.
    pub fn clear(&mut self) {
        use std::io::Write as _;
        if self.drawn {
            let mut err = std::io::stderr();
            let _ = write!(err, "\r\x1b[2K");
            let _ = err.flush();
            self.drawn = false;
        }
    }
}

/// The banner's lines without colour (the TUI paints them itself): the small one below
/// [`NARROW_COLUMNS`] columns.
pub fn banner_lines(width: u16) -> Vec<&'static str> {
    let text = if width < NARROW_COLUMNS || width < 40 { BANNER_SMALL } else { BANNER };
    text.lines().filter(|l| !l.trim().is_empty()).collect()
}

/// Frame `i` of the ember spinner in its gradient colour (single-line progress such as `kor seo`).
pub fn ember_frame(i: usize, depth: Depth) -> String {
    let f = &theme::frames::EMBER;
    let g = &theme::GRADIENT;
    paint(depth, g[i % g.len()], f[i % f.len()])
}

/// State of one row of a [`MultiSpinner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Waiting,
    Running,
    Done,
    Failed,
}

/// One braille spinner row per sub-agent (§16.1 `createMultiSpinner`, §16.2 rule 5): rows are
/// redrawn in place every [`MultiSpinner::TICK`]; finished rows show ✓ or ✗ and stay.
#[derive(Debug, Default)]
pub struct MultiSpinner {
    rows: Vec<(String, String, RowState)>,
    tick: usize,
    drawn: usize,
}

impl MultiSpinner {
    pub const TICK: std::time::Duration = std::time::Duration::from_millis(100);

    /// Set row `i` (rows are added as needed): label (agent role), detail text, state.
    pub fn set(&mut self, i: usize, label: &str, detail: &str, state: RowState) {
        while self.rows.len() <= i {
            self.rows.push((String::new(), String::new(), RowState::Waiting));
        }
        self.rows[i] = (label.to_owned(), detail.to_owned(), state);
    }

    /// The rows as text at a colour depth (no cursor movement): `⠋ backend  Running`.
    pub fn render(&self, depth: Depth) -> Vec<String> {
        let frames = &theme::frames::BRAILLE;
        let width = self.rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
        self.rows
            .iter()
            .enumerate()
            .map(|(i, (label, detail, state))| {
                let mark = match state {
                    RowState::Running => {
                        let g = &theme::GRADIENT;
                        paint(depth, g[(self.tick + i) % g.len()], frames[(self.tick + i) % frames.len()])
                    }
                    RowState::Waiting => dim(depth, "·"),
                    RowState::Done => paint(depth, theme::status::SUCCESS, "✓"),
                    RowState::Failed => paint(depth, theme::status::ERROR, "✗"),
                };
                let pad = " ".repeat(width.saturating_sub(label.chars().count()));
                format!("{mark} {label}{pad}  {}", dim(depth, detail))
            })
            .collect()
    }

    /// Redraw in place on stderr and advance the frame.
    pub fn draw(&mut self) {
        use std::io::Write as _;
        let lines = self.render(Depth::detect());
        let mut out = String::new();
        if self.drawn > 0 {
            out.push_str(&format!("\x1b[{}F", self.drawn));
        }
        for l in &lines {
            out.push_str("\x1b[2K");
            out.push_str(l);
            out.push('\n');
        }
        let mut err = std::io::stderr();
        let _ = err.write_all(out.as_bytes());
        let _ = err.flush();
        self.drawn = lines.len();
        self.tick += 1;
    }
}

/// A [`MultiSpinner`] redrawn by a background thread while it lives (terminal only); dropping it
/// draws the final state once and stops the thread.
pub struct LiveMulti {
    state: std::sync::Arc<std::sync::Mutex<MultiSpinner>>,
    stop: std::sync::Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LiveMulti {
    /// `None` when stderr is not a terminal: callers print plain lines instead.
    pub fn start() -> Option<Self> {
        use std::io::IsTerminal as _;
        if !std::io::stderr().is_terminal() {
            return None;
        }
        let state = std::sync::Arc::new(std::sync::Mutex::new(MultiSpinner::default()));
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                if !PAUSED.load(Ordering::Relaxed) {
                    let mut m = s.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !m.rows.is_empty() {
                        m.draw();
                    }
                }
                std::thread::sleep(MultiSpinner::TICK);
            }
        });
        Some(Self { state, stop, thread: Some(thread) })
    }

    pub fn set(&self, i: usize, label: &str, detail: &str, state: RowState) {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).set(i, label, detail, state);
    }
}

impl Drop for LiveMulti {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut m = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !m.rows.is_empty() {
            m.draw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_spinner_rows_animate_and_settle() {
        let mut m = MultiSpinner::default();
        m.set(1, "frontend", "waiting", RowState::Waiting);
        m.set(0, "backend", "src/auth.rs", RowState::Running);
        let a = m.render(Depth::None);
        assert_eq!(a.len(), 2);
        assert!(a[0].starts_with("⠋ backend "), "{a:?}");
        assert!(a[1].starts_with("· frontend"), "{a:?}");
        m.tick = 1;
        assert!(m.render(Depth::None)[0].starts_with("⠙ "));
        m.set(0, "backend", "merged", RowState::Done);
        m.set(1, "frontend", "failed", RowState::Failed);
        let done = m.render(Depth::None);
        assert!(done[0].starts_with("✓ ") && done[1].starts_with("✗ "), "{done:?}");
        assert!(m.render(Depth::TrueColor)[0].contains("\x1b[38;2;"));
    }

    fn spinner_at(tick: usize) -> Spinner {
        let mut s = Spinner::new();
        s.set_status(AgentStatus { verb: "Közlüyor".into(), verb_index: 1, elapsed_ms: 3_400, tokens: 120 });
        s.set_context(62);
        s.tick = tick;
        s
    }

    /// Escape bytes made visible, so snapshots are readable text.
    fn shown(s: &str) -> String {
        s.replace('\x1b', "\\e")
    }

    // §16.4 CLI snapshots (ADR 0061): banner wide/narrow, spinner truecolor / 256 / NO_COLOR, the verb cycle.
    #[test]
    fn snapshot_banner_wide_and_narrow() {
        insta::assert_snapshot!("banner_wide_plain", banner_depth(Depth::None, false));
        insta::assert_snapshot!("banner_narrow_plain", banner_depth(Depth::None, true));
        insta::assert_snapshot!("banner_wide_truecolor", shown(&banner_depth(Depth::TrueColor, false)));
        insta::assert_snapshot!("banner_narrow_256", shown(&banner_depth(Depth::Ansi256, true)));
        assert!(banner_depth(Depth::None, true).lines().all(|l| l.chars().count() < usize::from(NARROW_COLUMNS)));
    }

    #[test]
    fn snapshot_spinner_depths() {
        let s = spinner_at(2);
        insta::assert_snapshot!("spinner_truecolor", shown(&s.line(Depth::TrueColor).unwrap()));
        insta::assert_snapshot!("spinner_256", shown(&s.line(Depth::Ansi256).unwrap()));
        let plain = s.line(Depth::None).unwrap();
        assert!(!plain.contains('\x1b'), "NO_COLOR: no escape codes");
        insta::assert_snapshot!("spinner_no_color", plain);
    }

    #[test]
    fn snapshot_verb_cycle() {
        // Verbs come from the core's cycle over `assets/verbs.json`; frames from `theme.json`.
        let lines: Vec<String> = (0..8_u64)
            .map(|i| {
                let mut s = Spinner::new();
                let mut st = kor_agent::status::status_at(0, std::time::Duration::from_secs(i), 0);
                let verbs = kor_agent::status::verbs("tr");
                let n = usize::try_from(i).unwrap_or(0);
                st.verb.clone_from(&verbs[n % verbs.len()]);
                st.verb_index = n % verbs.len();
                s.set_status(st);
                s.tick = n;
                s.line(Depth::None).unwrap()
            })
            .collect();
        insta::assert_snapshot!("spinner_verb_cycle_tr", lines.join("\n"));
    }

    #[test]
    fn depth_detection() {
        assert_eq!(Depth::from_env(true, Some("xterm-256color"), Some("truecolor"), true), Depth::None);
        assert_eq!(Depth::from_env(false, Some("dumb"), None, true), Depth::None);
        assert_eq!(Depth::from_env(false, Some("xterm-256color"), None, false), Depth::None);
        assert_eq!(Depth::from_env(false, Some("xterm-256color"), None, true), Depth::Ansi256);
        assert_eq!(Depth::from_env(false, Some("xterm-256color"), Some("24bit"), true), Depth::TrueColor);
        assert_eq!(Depth::from_env(false, None, None, true), Depth::TrueColor, "Windows consoles");
        assert_eq!(ansi256(kor_core::theme::ember::MID), 214);
    }

    #[test]
    fn banner_plain_and_colored() {
        let plain = banner(false, false);
        assert!(plain.contains("██╗") && !plain.contains('\x1b'));
        assert!(banner(true, false).starts_with("\x1b[38;2;"));
        assert!(!banner(false, true).is_empty());
    }

    #[test]
    fn spinner_draws_the_core_verb() {
        let mut s = Spinner::new();
        assert!(s.frame().is_none(), "no status yet: nothing of its own");
        let st = kor_agent::status::status_at(1, std::time::Duration::from_secs(9), 0);
        s.set_status(st.clone());
        let (f0, verb, secs) = s.frame().expect("frame");
        assert_eq!((verb, secs), (st.verb.as_str(), 9));
        s.tick = 1;
        assert_ne!(s.frame().expect("frame").0, f0);
    }
}
