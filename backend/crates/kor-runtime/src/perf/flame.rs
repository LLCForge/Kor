//! Flamegraph SVG from collapsed stacks. Plain shapes, text and `<title>` tooltips only: no
//! script, no external references, so it can be shown as an image (`<img>`) in the desktop.
//! (The `inferno` crate embeds JavaScript for its interactive view; a static writer is enough.)

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{Profile, frame_text};

const WIDTH: f64 = 1200.0;
const ROW: f64 = 17.0;
const PAD: f64 = 10.0;
const TOP: f64 = 34.0;
/// Narrower boxes are not drawn (they are invisible anyway and bloat the file).
const MIN_PX: f64 = 0.3;

#[derive(Default)]
struct Node {
    value: u64,
    children: BTreeMap<u32, Node>,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// Warm colour from the name, stable between runs.
fn color(name: &str) -> (u8, u8, u8) {
    let h = name.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    let [a, b, c, _] = h.to_le_bytes();
    (205 + a % 50, 80 + b % 120, 30 + c % 50)
}

#[allow(clippy::cast_precision_loss)] // weights far below 2^52 in practice
fn pct(part: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { part as f64 * 100.0 / total as f64 }
}

/// Flamegraph of `p` (root at the bottom). `title` is shown on top.
#[allow(clippy::cast_precision_loss)]
pub fn svg(p: &Profile, title: &str) -> String {
    let mut root = Node::default();
    let mut depth = 0usize;
    for (stack, &w) in &p.stacks {
        root.value += w;
        let mut n = &mut root;
        for &f in stack {
            n = n.children.entry(f).or_default();
            n.value += w;
        }
        depth = depth.max(stack.len());
    }
    let total = root.value;
    let height = TOP + (depth as f64 + 1.0) * ROW + PAD;
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="monospace" font-size="12"><rect width="100%" height="100%" fill="rgb(250,250,247)"/><text x="{PAD}" y="22" font-size="15" fill="rgb(30,30,30)">{}</text>"#,
        esc(title)
    );
    let scale = (WIDTH - 2.0 * PAD) / (total.max(1) as f64);
    // (node, x, level); level 0 is the "all" row at the bottom.
    let mut todo: Vec<(&Node, Option<u32>, f64, usize)> = vec![(&root, None, PAD, 0)];
    while let Some((n, frame, x, level)) = todo.pop() {
        let w = n.value as f64 * scale;
        if w < MIN_PX {
            continue;
        }
        let y = height - PAD - (level as f64 + 1.0) * ROW;
        let label = frame.map_or_else(|| "all".to_owned(), |f| frame_text(&p.frames[f as usize]));
        let name = frame.map_or("all", |f| p.frames[f as usize].name.as_str());
        let (r, g, b) = if frame.is_some() { color(name) } else { (200, 200, 200) };
        let tip = format!("{label} — {} {} ({:.2}%)", n.value, p.unit.as_str(), pct(n.value, total));
        let _ = write!(
            out,
            r#"<g><title>{}</title><rect x="{x:.2}" y="{y:.1}" width="{w:.2}" height="{:.1}" rx="2" fill="rgb({r},{g},{b})"/>"#,
            esc(&tip),
            ROW - 1.0
        );
        // About 7 px per character at 12 px monospace.
        let fit = ((w - 6.0) / 7.0).floor();
        if fit >= 3.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let fit = fit as usize;
            let shown: String = if name.chars().count() > fit {
                name.chars().take(fit.saturating_sub(2)).chain("..".chars()).collect()
            } else {
                name.to_owned()
            };
            let _ = write!(out, r#"<text x="{:.2}" y="{:.1}" fill="rgb(20,20,20)">{}</text>"#, x + 3.0, y + ROW - 5.0, esc(&shown));
        }
        out.push_str("</g>");
        let mut cx = x;
        for (&f, c) in &n.children {
            todo.push((c, Some(f), cx, level + 1));
            cx += c.value as f64 * scale;
        }
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::{Unit, parse};

    #[test]
    fn writes_a_static_svg() {
        let p = parse::collapsed("main (src/a.rs:1);fib<&'a> (src/a.rs:9) 75\nmain (src/a.rs:1);work 25\n");
        let s = svg(&p, "kor perf <cargo test>");
        assert!(s.starts_with("<svg") && s.ends_with("</svg>\n"));
        assert!(!s.contains("<script") && !s.contains("href") && !s.contains("onload"));
        assert!(s.contains("kor perf &lt;cargo test&gt;"));
        assert!(s.contains("fib&lt;&amp;&#39;a&gt; (src/a.rs:9) — 75 samples (75.00%)"), "{s}");
        // all + main + fib + work
        assert_eq!(s.matches("<rect x=").count(), 4);
        // fib starts at the left edge of main and is three times as wide as work.
        let width = |label: &str| {
            let i = s.find(label).unwrap();
            let rest = &s[i..];
            let w = rest.split("width=\"").nth(1).unwrap().split('"').next().unwrap();
            w.parse::<f64>().unwrap()
        };
        let (fib, work) = (width("fib&lt;"), width("work — "));
        assert!((fib / work - 3.0).abs() < 0.01, "{fib} {work}");
        let empty = svg(&Profile::new(Unit::Bytes), "x");
        assert!(empty.contains("</svg>"));
    }
}
