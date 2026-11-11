//! Respecting the terminal (Master Build Prompt §28.9, §28.11, ADR 0050): colour only when wanted
//! (`NO_COLOR`, `TERM=dumb`, not a terminal), inline images only where the terminal says it can
//! show them (kitty / sixel, detected from the environment; nothing is written to probe), the
//! readability settings of the line interface (wrap width, tool output detail, motion).

use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};

use base64::Engine as _;
use kor_config::{ImageProtocol, Verbosity};

/// A terminal that cannot take escape sequences.
pub fn dumb() -> bool {
    dumb_from(std::env::var("TERM").ok().as_deref())
}

fn dumb_from(term: Option<&str>) -> bool {
    term.is_some_and(|t| t == "dumb")
}

/// Colour on stdout: a terminal, no `NO_COLOR` (any value), not `TERM=dumb`.
pub fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && !dumb() && std::io::stdout().is_terminal()
}

/// What inline image protocol the terminal speaks, from `TERM`, `TERM_PROGRAM` and the
/// terminals' own variables. Inside tmux/screen nothing (passthrough is not guaranteed).
pub fn image_protocol(setting: ImageProtocol) -> Option<ImageProtocol> {
    let var = |k: &str| std::env::var(k).ok();
    detect(
        setting,
        &Env {
            term: var("TERM"),
            program: var("TERM_PROGRAM"),
            kitty: var("KITTY_WINDOW_ID").is_some(),
            konsole: var("KONSOLE_VERSION").and_then(|v| v.parse().ok()),
            wt: var("WT_SESSION").is_some(),
            tmux: var("TMUX").is_some(),
            tty: std::io::stdout().is_terminal(),
        },
    )
}

#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)] // what the environment says, one flag each
struct Env {
    term: Option<String>,
    program: Option<String>,
    kitty: bool,
    konsole: Option<u32>,
    wt: bool,
    tmux: bool,
    tty: bool,
}

fn detect(setting: ImageProtocol, e: &Env) -> Option<ImageProtocol> {
    let term = e.term.as_deref().unwrap_or("");
    if setting == ImageProtocol::Off || !e.tty || dumb_from(Some(term)) || e.tmux || term.starts_with("screen") || term.starts_with("tmux") {
        return None;
    }
    if setting != ImageProtocol::Auto {
        return Some(setting);
    }
    let program = e.program.as_deref().unwrap_or("");
    if e.kitty || term.contains("kitty") || term == "xterm-ghostty" || matches!(program, "ghostty" | "WezTerm") {
        return Some(ImageProtocol::Kitty);
    }
    if term.starts_with("foot") || term.contains("mlterm") || term.contains("contour") || program == "iTerm.app" || e.wt || e.konsole.is_some_and(|v| v >= 220_400) {
        return Some(ImageProtocol::Sixel);
    }
    None
}

// ------------------------------------------------------------------------------------------------
// Inline images
// ------------------------------------------------------------------------------------------------

const MAX_PX: u32 = 800;

/// The kitty graphics protocol escape for a PNG (sent in 4096-byte base64 chunks).
pub fn kitty(png: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    let mut out = String::new();
    for (i, c) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 { format!("f=100,a=T,m={more}") } else { format!("m={more}") };
        let _ = write!(out, "\x1b_G{head};{}\x1b\\", String::from_utf8_lossy(c));
    }
    out.push('\n');
    out
}

/// A sixel image (6×6×6 colour cube, run-length coded).
pub fn sixel(img: &image::RgbaImage) -> String {
    let (w, h) = img.dimensions();
    let idx = |p: &image::Rgba<u8>| -> usize {
        if p[3] < 128 {
            return usize::MAX;
        }
        let q = |v: u8| usize::from(v) * 5 / 255;
        q(p[0]) * 36 + q(p[1]) * 6 + q(p[2])
    };
    let mut out = format!("\x1bPq\"1;1;{w};{h}");
    for i in 0..216_u32 {
        let pct = |v: u32| v * 100 / 5;
        let _ = write!(out, "#{i};2;{};{};{}", pct(i / 36), pct(i / 6 % 6), pct(i % 6));
    }
    for band in (0..h).step_by(6) {
        let mut used = [false; 216];
        for y in band..(band + 6).min(h) {
            for x in 0..w {
                let c = idx(img.get_pixel(x, y));
                if c != usize::MAX {
                    used[c] = true;
                }
            }
        }
        for (c, _) in used.iter().enumerate().filter(|(_, u)| **u) {
            let _ = write!(out, "#{c}");
            let mut run: Option<(u8, u32)> = None;
            let flush = |run: Option<(u8, u32)>, out: &mut String| {
                if let Some((ch, n)) = run {
                    if n > 3 {
                        let _ = write!(out, "!{n}{}", char::from(ch));
                    } else {
                        for _ in 0..n {
                            out.push(char::from(ch));
                        }
                    }
                }
            };
            for x in 0..w {
                let mut bits = 0u8;
                for dy in 0..6 {
                    let y = band + dy;
                    if y < h && idx(img.get_pixel(x, y)) == c {
                        bits |= 1 << dy;
                    }
                }
                let ch = 63 + bits;
                run = match run {
                    Some((r, n)) if r == ch => Some((r, n + 1)),
                    other => {
                        flush(other, &mut out);
                        Some((ch, 1))
                    }
                };
            }
            flush(run, &mut out);
            out.push('$');
        }
        out.push('-');
    }
    out.push_str("\x1b\\\n");
    out
}

/// Show an image file inline when the terminal can; `false` = the caller prints the path.
pub fn show_image(path: &std::path::Path, proto: Option<ImageProtocol>) -> bool {
    let Some(proto) = proto else { return false };
    let Ok(img) = image::open(path) else { return false };
    let img = if img.width() > MAX_PX { img.resize(MAX_PX, MAX_PX * 4, image::imageops::FilterType::Triangle) } else { img };
    let seq = match proto {
        ImageProtocol::Kitty => {
            let mut png = Vec::new();
            if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
                return false;
            }
            kitty(&png)
        }
        ImageProtocol::Sixel => sixel(&img.to_rgba8()),
        ImageProtocol::Auto | ImageProtocol::Off => return false,
    };
    let mut out = std::io::stdout();
    out.write_all(seq.as_bytes()).and_then(|()| out.flush()).is_ok()
}

/// Image files named in a tool's output (media tools report the saved path).
pub fn image_paths(output: &str) -> Vec<std::path::PathBuf> {
    output
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | ','))
        .map(|w| w.trim_end_matches(['.', ':']))
        .filter(|w| {
            let lw = w.to_lowercase();
            [".png", ".jpg", ".jpeg", ".webp", ".gif"].iter().any(|e| lw.ends_with(e))
        })
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_file())
        .collect()
}

// ------------------------------------------------------------------------------------------------
// Display settings of the line interface (§28.9)
// ------------------------------------------------------------------------------------------------

static WRAP: AtomicU16 = AtomicU16::new(u16::MAX);
static VERBOSITY: AtomicU8 = AtomicU8::new(1);
static STILL: AtomicBool = AtomicBool::new(false);
static IMAGES: AtomicU8 = AtomicU8::new(0);

/// Take `ui.*` / `tui.*` from the config for this process.
pub fn init(c: &kor_config::Config) {
    crate::brand::set_spinner_style(&c.ui.spinner);
    // 0 = no wrapping; `u16::MAX` = the available width (panel / terminal).
    let width = if !c.ui.wrap {
        0
    } else if c.ui.wrap_width > 0 {
        c.ui.wrap_width
    } else {
        crossterm::terminal::size().map_or(u16::MAX, |(w, _)| w.saturating_sub(1).max(10))
    };
    WRAP.store(width, Ordering::Relaxed);
    set_verbosity(c.ui.tool_output);
    STILL.store(c.ui.screen_reader || c.ui.reduced_motion || dumb(), Ordering::Relaxed);
    IMAGES.store(
        match image_protocol(c.tui.images) {
            Some(ImageProtocol::Kitty) => 1,
            Some(ImageProtocol::Sixel) => 2,
            _ => 0,
        },
        Ordering::Relaxed,
    );
}

pub fn wrap_width() -> usize {
    usize::from(WRAP.load(Ordering::Relaxed))
}

pub fn images() -> Option<ImageProtocol> {
    match IMAGES.load(Ordering::Relaxed) {
        1 => Some(ImageProtocol::Kitty),
        2 => Some(ImageProtocol::Sixel),
        _ => None,
    }
}

/// No spinner or animation (screen reader, reduced motion, dumb terminal).
pub fn still() -> bool {
    STILL.load(Ordering::Relaxed)
}

pub fn verbosity() -> Verbosity {
    match VERBOSITY.load(Ordering::Relaxed) {
        0 => Verbosity::Quiet,
        2 => Verbosity::Verbose,
        _ => Verbosity::Normal,
    }
}

/// `/verbosity quiet|normal|verbose`: takes effect at once, also during a running task.
pub fn set_verbosity(v: Verbosity) {
    VERBOSITY.store(
        match v {
            Verbosity::Quiet => 0,
            Verbosity::Normal => 1,
            Verbosity::Verbose => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn parse_verbosity(s: &str) -> Option<Verbosity> {
    match s.trim() {
        "quiet" | "sessiz" | "silencieux" => Some(Verbosity::Quiet),
        "normal" => Some(Verbosity::Normal),
        "verbose" | "ayrıntılı" | "détaillé" => Some(Verbosity::Verbose),
        _ => None,
    }
}

/// Word wrap of streamed model text: whole words move to the next line at the wrap width.
#[derive(Debug, Default)]
pub struct Wrapper {
    width: usize,
    col: usize,
    word: String,
}

impl Wrapper {
    pub fn new(width: usize) -> Self {
        Self { width, ..Self::default() }
    }

    /// Text ready to print (a word still being streamed is held back).
    pub fn push(&mut self, s: &str) -> String {
        // Off, or "the terminal's width": the terminal wraps by itself.
        if self.width < 10 || self.width >= usize::from(u16::MAX) {
            return s.to_owned();
        }
        let mut out = String::new();
        for c in s.chars() {
            if c == '\n' {
                self.emit_word(&mut out);
                out.push('\n');
                self.col = 0;
            } else if c == ' ' {
                self.emit_word(&mut out);
                if self.col < self.width {
                    out.push(' ');
                    self.col += 1;
                }
            } else {
                self.word.push(c);
            }
        }
        out
    }

    fn emit_word(&mut self, out: &mut String) {
        let n = self.word.chars().count();
        if n == 0 {
            return;
        }
        if self.col > 0 && self.col + n > self.width {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            self.col = 0;
        }
        out.push_str(&self.word);
        self.col += n;
        self.word.clear();
    }

    /// The held-back word (end of a reply).
    pub fn flush(&mut self) -> String {
        let mut out = String::new();
        self.emit_word(&mut out);
        self.col = 0;
        out
    }
}

static JSON: AtomicBool = AtomicBool::new(false);

/// `kor -p --json` (§28.1): events as JSON lines instead of text.
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

pub fn json() -> bool {
    JSON.load(Ordering::Relaxed)
}

/// One event as a JSON line: the desktop wire shape, plus `context.usage` with the breakdown,
/// the total and the percent of the window (every figure ≈).
pub fn json_event(ev: &kor_agent::AgentEvent) -> Option<String> {
    let v = match ev {
        kor_agent::AgentEvent::Context(b) => serde_json::json!({
            "type": "context.usage",
            "breakdown": b,
            "total": b.total(),
            "percent": b.percent(b.total()),
        }),
        kor_agent::AgentEvent::Usage(u) => serde_json::json!({ "type": "usage", "input": u.input, "output": u.output }),
        other => kor_agent::remote::wire::event_json(other.clone())?,
    };
    Some(v.to_string())
}

/// Terminal title, only with `[tui] title = true`: pushed on the xterm title stack, so leaving
/// restores the user's own title exactly.
pub fn push_title(title: &str) -> String {
    format!("\x1b[22;0t\x1b]0;{title}\x07")
}

pub const POP_TITLE: &str = "\x1b[23;0t";

#[cfg(test)]
mod tests {
    use super::*;

    fn env(term: &str) -> Env {
        Env { term: Some(term.into()), tty: true, ..Env::default() }
    }

    #[test]
    fn image_protocol_from_the_environment_only() {
        assert_eq!(detect(ImageProtocol::Auto, &env("xterm-kitty")), Some(ImageProtocol::Kitty));
        assert_eq!(detect(ImageProtocol::Auto, &Env { program: Some("WezTerm".into()), ..env("xterm-256color") }), Some(ImageProtocol::Kitty));
        assert_eq!(detect(ImageProtocol::Auto, &env("foot")), Some(ImageProtocol::Sixel));
        assert_eq!(detect(ImageProtocol::Auto, &Env { wt: true, ..env("xterm-256color") }), Some(ImageProtocol::Sixel));
        assert_eq!(detect(ImageProtocol::Auto, &env("xterm-256color")), None, "unknown terminal: fall back to the path");
        assert_eq!(detect(ImageProtocol::Auto, &env("dumb")), None);
        assert_eq!(detect(ImageProtocol::Kitty, &env("dumb")), None, "a dumb terminal never gets escapes");
        assert_eq!(detect(ImageProtocol::Auto, &Env { tmux: true, ..env("xterm-kitty") }), None);
        assert_eq!(detect(ImageProtocol::Auto, &Env { tty: false, ..env("xterm-kitty") }), None);
        assert_eq!(detect(ImageProtocol::Off, &env("xterm-kitty")), None);
        assert_eq!(detect(ImageProtocol::Sixel, &env("xterm-256color")), Some(ImageProtocol::Sixel));
        assert!(dumb_from(Some("dumb")) && !dumb_from(None));
    }

    #[test]
    fn kitty_and_sixel_encodings() {
        let k = kitty(&vec![7u8; 5000]);
        assert!(k.starts_with("\x1b_Gf=100,a=T,m=1;"));
        assert!(k.contains("\x1b_Gm=0;"));
        let mut img = image::RgbaImage::new(3, 7);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        let s = sixel(&img);
        assert!(s.starts_with("\x1bPq\"1;1;3;7"));
        assert!(s.ends_with("\x1b\\\n"));
        // Red is colour 5*36 = 180; two bands of six rows.
        assert!(s.contains("#180~~~$-"), "{s}");
        assert_eq!(s.matches('-').count(), 2);
    }

    #[test]
    fn wrapper_moves_whole_words() {
        let mut w = Wrapper::new(12);
        let mut out = w.push("hello wor");
        out.push_str(&w.push("ld again and again\nnext"));
        out.push_str(&w.flush());
        assert_eq!(out, "hello world\nagain and\nagain\nnext");
        assert_eq!(Wrapper::new(0).push("a b"), "a b");
        assert_eq!(parse_verbosity("ayrıntılı"), Some(Verbosity::Verbose));
        set_verbosity(Verbosity::Quiet);
        assert_eq!(verbosity(), Verbosity::Quiet);
        set_verbosity(Verbosity::Normal);
        assert!(push_title("Kor").starts_with("\x1b[22;0t"));
        let b = kor_agent::context::Breakdown { window: 1000, history: 250, ..Default::default() };
        let line = json_event(&kor_agent::AgentEvent::Context(b)).unwrap();
        assert!(line.contains("\"type\":\"context.usage\"") && line.contains("\"percent\":25"), "{line}");
    }
}
