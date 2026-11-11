//! Layout and painting. Two layouts: wide (≥120×30) shows every panel beside the stream;
//! compact (80×24 and up) shows the stream, one docked side panel and a tab row to switch it.

use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::keys::Keys;
use super::{PANELS, Panel, Screen, sticker, theme_color};

const EMBER: Color = {
    let (r, g, b) = kor_core::theme::ember::MID;
    Color::Rgb(r, g, b)
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Areas {
    pub header: Rect,
    pub input: Rect,
    /// Compact layout only: the row of panel names.
    pub tabs: Option<Rect>,
    /// The stream first, then the visible side panels.
    pub panels: Vec<(Panel, Rect)>,
}

impl Areas {
    /// The panel under a terminal cell.
    pub fn hit(&self, col: u16, row: u16) -> Option<(Panel, Rect)> {
        self.panels
            .iter()
            .copied()
            .find(|(_, r)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
    }
}

pub fn wide(area: Rect) -> bool {
    area.width >= 120 && area.height >= 30
}

pub fn layout(area: Rect, dock: Panel) -> Areas {
    if wide(area) {
        let [header, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
                .areas(body);
        let [stream, input] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(left);
        let side = Layout::vertical([
            Constraint::Length(6),
            Constraint::Fill(3),
            Constraint::Fill(2),
            Constraint::Fill(2),
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Fill(3),
        ])
        .split(right);
        let mut panels = vec![(Panel::Stream, stream)];
        panels.extend(PANELS[1..].iter().copied().zip(side.iter().copied()));
        return Areas {
            header,
            input,
            tabs: None,
            panels,
        };
    }
    let dock_h = match area.height {
        h if h >= 40 => 16,
        h if h >= 30 => 12,
        h if h >= 20 => 8,
        _ => 5,
    };
    let [header, stream, docked, tabs, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(dock_h),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(area);
    let dock = if dock == Panel::Stream {
        Panel::Tasks
    } else {
        dock
    };
    Areas {
        header,
        input,
        tabs: Some(tabs),
        panels: vec![(Panel::Stream, stream), (dock, docked)],
    }
}

/// Tab labels and their column ranges in the compact layout.
fn tabs(l: &Localizer, area: Rect) -> Vec<(Panel, String, u16, u16)> {
    let mut x = area.x;
    PANELS[1..]
        .iter()
        .map(|p| {
            let label = format!(" {} ", l.t(p.key()));
            let w = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
            let start = x;
            x = x.saturating_add(w);
            (*p, label, start, x)
        })
        .collect()
}

/// The side panel whose tab is under a terminal cell.
pub fn tab_at(l: &Localizer, areas: &Areas, col: u16, row: u16) -> Option<Panel> {
    let t = areas.tabs?;
    if row != t.y {
        return None;
    }
    tabs(l, t)
        .into_iter()
        .find(|(_, _, a, b)| col >= *a && col < *b)
        .map(|(p, ..)| p)
}

/// First change shown in the changes list (it scrolls to keep the selection visible).
pub fn list_start(screen: &Screen, rect: Rect) -> usize {
    let room = usize::from(rect.height.saturating_sub(2) / 2).max(1);
    screen.selected.saturating_sub(room - 1)
}

/// Hard-wrap to `width` characters (terminal cells for the scripts Kor ships in).
fn wrap(lines: &[String], width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            out.push(String::new());
        }
        for chunk in chars.chunks(width) {
            out.push(chunk.iter().collect());
        }
    }
    out
}

/// The visible window of `lines`: `scroll` lines up from the bottom (`bottom`) or down from the top.
fn window(lines: Vec<String>, height: usize, scroll: usize, bottom: bool) -> Vec<String> {
    let n = lines.len();
    let (start, end) = if bottom {
        let end = n.saturating_sub(scroll).max(height.min(n));
        (end.saturating_sub(height), end)
    } else {
        let start = scroll.min(n.saturating_sub(1));
        (start, (start + height).min(n))
    };
    lines.into_iter().skip(start).take(end - start).collect()
}

fn plain(lines: Vec<String>) -> Vec<Line<'static>> {
    lines.into_iter().map(Line::from).collect()
}

fn diff_line(s: String) -> Line<'static> {
    let color = if s.starts_with("+ ") {
        Some(Color::Green)
    } else if s.starts_with("- ") {
        Some(Color::Red)
    } else {
        None
    };
    match color {
        Some(c) => Line::styled(s, Style::default().fg(c)),
        None => Line::from(s),
    }
}

fn or_empty(lines: &[String], l: &Localizer, key: &str) -> Vec<String> {
    if lines.is_empty() {
        vec![l.t(key)]
    } else {
        lines.to_vec()
    }
}

#[allow(clippy::too_many_lines)] // one arm per panel
fn body(screen: &Screen, l: &Localizer, panel: Panel, inner: Rect) -> Vec<Line<'static>> {
    let h = usize::from(inner.height);
    let w = usize::from(inner.width);
    let scroll = screen.scroll[panel.idx()];
    match panel {
        // §16.1: an empty session shows the Kor banner in the gradient, then the hint.
        Panel::Stream if screen.lines.is_empty() && screen.busy.is_none() && h >= 6 => {
            let g = &kor_core::theme::GRADIENT;
            let mut out: Vec<Line<'static>> = crate::brand::banner_lines(inner.width)
                .into_iter()
                .enumerate()
                .map(|(i, s)| {
                    let (r, gr, b) = g[i % g.len()];
                    Line::styled(s.to_owned(), Style::default().fg(Color::Rgb(r, gr, b)))
                })
                .collect();
            out.push(Line::from(""));
            out.push(Line::styled(
                l.t("repl-hint"),
                Style::default().add_modifier(Modifier::DIM),
            ));
            out.truncate(h);
            out
        }
        Panel::Stream => {
            // Each source line wraps to at least one row: the tail is enough.
            let tail = screen.lines.len().saturating_sub(h + scroll);
            // §28.9: `ui.wrap` word-wraps at `ui.wrap_width` (or the panel); off cuts long lines.
            let ww = super::term::wrap_width();
            let shaped: Vec<String> = screen.lines[tail..]
                .iter()
                .flat_map(|line| {
                    let s = if ww == 0 {
                        line.chars().take(w.max(1)).collect()
                    } else {
                        kor_runtime::prefs::wrap(line, w.min(ww).max(1))
                    };
                    s.split('\n').map(str::to_owned).collect::<Vec<_>>()
                })
                .collect();
            plain(window(wrap(&shaped, w), h, scroll, true))
        }
        Panel::Tasks => {
            let status = match (&screen.busy, &screen.tool) {
                (Some(_), Some(t)) => l.t_args("tui-tool", &args!("tool" => t.clone())),
                (Some(b), None) => l.t_args("tui-running", &args!("task" => b.clone())),
                (None, _) => l.t("tui-idle"),
            };
            let mut lines = vec![status];
            // §16.2: the same verb the desktop shows, from the core's `agent.status`.
            if let (Some(_), Some(s)) = (&screen.busy, &screen.status) {
                lines.push(super::status_line(s));
            }
            lines.extend([
                l.t_args(
                    "tui-jobs",
                    &args!("running" => screen.jobs.0, "total" => screen.jobs.1),
                ),
                l.t_args("tui-journal", &args!("count" => screen.changes.len())),
                l.t_args(
                    "tui-tokens",
                    &args!("input" => screen.tokens.0, "output" => screen.tokens.1),
                ),
            ]);
            plain(window(wrap(&lines, w), h, scroll, false))
        }
        Panel::Changes => {
            if screen.changes.is_empty() {
                return plain(vec![l.t("tui-no-changes")]);
            }
            let rect = Rect {
                height: inner.height + 2,
                ..inner
            };
            let start = list_start(screen, rect);
            let room = usize::from(inner.height / 2).max(1);
            let mut out: Vec<Line<'static>> = screen
                .changes
                .iter()
                .enumerate()
                .skip(start)
                .take(room)
                .map(|(i, c)| {
                    let name = kor_tools::display_path(&c.path);
                    if i == screen.selected {
                        Line::styled(
                            format!("▸ {name}"),
                            Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Line::from(format!("  {name}"))
                    }
                })
                .collect();
            out.push(Line::styled(
                l.t("tui-changes-help"),
                Style::default().add_modifier(Modifier::DIM),
            ));
            let diff_rows = h.saturating_sub(out.len());
            out.extend(
                window(wrap(&screen.diff, w), diff_rows, scroll, false)
                    .into_iter()
                    .map(diff_line),
            );
            out
        }
        Panel::Cost => {
            let mut lines = screen.cost.clone();
            lines.extend(screen.quota.iter().cloned());
            plain(window(wrap(&lines, w), h, scroll, false))
        }
        Panel::Context => plain(window(
            wrap(&or_empty(&screen.context, l, "tui-no-context"), w),
            h,
            scroll,
            false,
        )),
        Panel::Health => plain(window(
            wrap(&or_empty(&screen.health, l, "tui-health-none"), w),
            h,
            scroll,
            false,
        )),
        Panel::Notices => plain(window(
            wrap(&or_empty(&screen.notices, l, "tui-no-notices"), w),
            h,
            scroll,
            true,
        )),
        Panel::Graph => window(
            wrap(&or_empty(&screen.graph, l, "tui-graph-off"), w),
            h,
            scroll,
            false,
        )
        .into_iter()
        .map(|s| {
            // `tui::graph` marks the selected row with ▸ and files the agent touched with ●.
            if s.starts_with('▸') {
                Line::styled(s, Style::default().fg(EMBER).add_modifier(Modifier::BOLD))
            } else if s.contains('●') {
                Line::styled(s, Style::default().fg(Color::Cyan))
            } else {
                Line::from(s)
            }
        })
        .collect(),
    }
}

#[allow(clippy::too_many_lines)] // the whole screen in one place
pub fn draw(frame: &mut Frame<'_>, screen: &Screen, l: &Localizer) {
    let area = frame.area();
    let areas = layout(area, screen.dock);
    let fg = theme_color(&screen.theme).unwrap_or(Color::White);

    let mark = match (&screen.perm, &screen.busy) {
        (Some(_), _) => sticker("pause"),
        (None, Some(_)) => sticker("running"),
        (None, None) => sticker("play"),
    }
    .unwrap_or("");
    let vim = match (screen.keys.keys, screen.keys.insert) {
        (Keys::Vim, true) => format!("  {}", l.t("tui-vim-insert")),
        (Keys::Vim, false) => format!("  {}", l.t("tui-vim-normal")),
        _ => String::new(),
    };
    let header = format!(
        " {mark} kor  {}  {} {}  ↑{} ↓{}{vim}",
        screen.mode,
        super::provider_glyph(&screen.model),
        screen.model,
        screen.tokens.0,
        screen.tokens.1
    );
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(EMBER)),
        areas.header,
    );

    for (panel, rect) in &areas.panels {
        let focused = screen.focus == Some(*panel);
        let block = Block::bordered()
            .title(format!(" {} ", l.t(panel.key())))
            .border_style(if focused {
                Style::default().fg(EMBER)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let inner = block.inner(*rect);
        let lines = body(screen, l, *panel, inner);
        frame.render_widget(
            Paragraph::new(lines)
                .style(Style::default().fg(fg))
                .block(block),
            *rect,
        );
    }

    if let Some(t) = areas.tabs {
        let docked = areas.panels.get(1).map(|(p, _)| *p);
        let spans: Vec<Span> = tabs(l, t)
            .into_iter()
            .map(|(p, label, ..)| {
                if Some(p) == docked {
                    Span::styled(label, Style::default().fg(Color::Black).bg(EMBER))
                } else {
                    Span::styled(label, Style::default().fg(Color::DarkGray))
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(spans)), t);
    }

    let typing = screen.keys.typing(screen.focus.is_none());
    // §12.3: a destructive request turns the prompt red with its reason; files toggle by number.
    let danger = screen.perm.as_ref().and(screen.perm_danger.as_ref());
    // The reason text stays without colour (NO_COLOR); red is only the extra cue.
    let red = danger.is_some() && std::env::var_os("NO_COLOR").is_none();
    let title = match (danger, screen.perm_files.len() > 1) {
        (Some(d), _) => d.clone(),
        (None, true) if screen.perm.is_some() => l.t("perm-file-marks"),
        _ => l.t(screen.keys.help_key()),
    };
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(if red {
            Style::default().fg(Color::Red)
        } else if typing {
            Style::default().fg(EMBER)
        } else {
            Style::default().fg(Color::DarkGray)
        });
    let prompt = match &screen.perm {
        Some(ask) if screen.plan_dialog => l.t_args("tui-plan", &args!("ask" => ask.clone())),
        Some(ask) if !screen.perm_choices.is_empty() => {
            let mut opts = String::new();
            for (i, o) in screen.perm_choices.iter().enumerate() {
                let sel = if i == screen.perm_choice { "▸" } else { " " };
                let _ = write!(opts, " {sel}{} {o}", i + 1);
            }
            let always = if screen.perm_remember { " [!]" } else { "" };
            l.t_args(
                "tui-choose",
                &args!("ask" => format!("{ask}{opts}{always}")),
            )
        }
        Some(ask) if screen.perm_compact => {
            let mut items = String::new();
            for (i, (p, keep)) in screen
                .perm_files
                .iter()
                .zip(&screen.perm_rejected)
                .enumerate()
            {
                let _ = write!(items, " · {} {p}{}", i + 1, if *keep { " [+]" } else { "" });
            }
            l.t_args("tui-compact", &args!("ask" => format!("{ask}{items}")))
        }
        Some(ask) => {
            let ask = format!(
                "{ask}{}",
                super::review::marks(&screen.perm_files, &screen.perm_rejected)
            );
            l.t_args("tui-perm", &args!("ask" => ask))
        }
        None => format!("> {}", screen.input),
    };
    let inner = block.inner(areas.input);
    // Keep the end of a long input visible.
    let width = usize::from(inner.width.max(1));
    let shown: String = {
        let n = prompt.chars().count();
        prompt.chars().skip(n.saturating_sub(width - 1)).collect()
    };
    let cursor_x = inner.x + u16::try_from(shown.chars().count()).unwrap_or(0);
    let text_style = if red {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    frame.render_widget(
        Paragraph::new(shown).style(text_style).block(block),
        areas.input,
    );
    if typing && screen.perm.is_none() {
        frame.set_cursor_position((
            cursor_x.min(inner.x + inner.width.saturating_sub(1)),
            inner.y,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{Ev, reduce};
    use kor_i18n::Lang;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn text(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn screen(l: &Localizer) -> Screen {
        let mut s = Screen::new("default", "anthropic/claude-sonnet");
        reduce(&mut s, l, Ev::Text("Merhaba dünya".into()));
        reduce(&mut s, l, Ev::Notice("budget alert".into()));
        s
    }

    #[test]
    fn empty_session_shows_the_banner_and_hint() {
        let l = Localizer::new(Lang::En);
        let s = Screen::new("default", "anthropic/claude-sonnet");
        let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
        t.draw(|f| draw(f, &s, &l)).unwrap();
        let out = text(&t);
        assert!(out.contains("██╗  ██╗"), "{out}");
        assert!(out.contains("/help"), "{out}");
    }

    #[test]
    fn renders_compact_at_80x24() {
        let l = Localizer::new(Lang::En);
        let mut s = screen(&l);
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, &s, &l)).unwrap();
        let out = text(&t);
        // The provider badge (ADR 0053) sits between the mode and the model.
        assert!(out.contains("kor  default  "), "{out}");
        assert!(out.contains("anthropic/claude-sonnet"), "{out}");
        assert!(out.contains("Merhaba dünya"), "{out}");
        assert!(out.contains(&l.t("tui-panel-tasks")), "{out}");
        assert!(out.contains(&l.t("tui-idle")), "{out}");
        assert!(out.contains("> "), "{out}");
        // The tab row switches the docked panel.
        let areas = layout(Rect::new(0, 0, 80, 24), s.dock);
        assert!(areas.tabs.is_some());
        let tab = tabs(&l, areas.tabs.unwrap())
            .into_iter()
            .find(|(p, ..)| *p == Panel::Notices)
            .unwrap();
        if tab.2 < 80 {
            assert_eq!(
                tab_at(&l, &areas, tab.2, areas.tabs.unwrap().y),
                Some(Panel::Notices)
            );
        }
        s.dock = Panel::Notices;
        t.draw(|f| draw(f, &s, &l)).unwrap();
        assert!(text(&t).contains("budget alert"));
    }

    #[test]
    fn renders_every_panel_when_wide_in_turkish() {
        let l = Localizer::new(Lang::Tr);
        let s = screen(&l);
        let mut t = Terminal::new(TestBackend::new(160, 48)).unwrap();
        t.draw(|f| draw(f, &s, &l)).unwrap();
        let out = text(&t);
        for p in PANELS {
            assert!(out.contains(&l.t(p.key())), "{} missing:\n{out}", p.key());
        }
        assert!(out.contains("budget alert"), "{out}");
        let areas = layout(Rect::new(0, 0, 160, 48), s.dock);
        assert_eq!(areas.panels.len(), PANELS.len());
        let (_, r) = areas.panels[2];
        assert_eq!(
            areas.hit(r.x + 1, r.y + 1).map(|h| h.0),
            Some(Panel::Changes)
        );
    }

    #[test]
    fn windows_scroll_from_the_right_end() {
        let lines: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert_eq!(window(lines.clone(), 3, 0, true), ["7", "8", "9"]);
        assert_eq!(window(lines.clone(), 3, 2, true), ["5", "6", "7"]);
        assert_eq!(window(lines.clone(), 3, 99, true), ["0", "1", "2"]);
        assert_eq!(window(lines.clone(), 3, 1, false), ["1", "2", "3"]);
        assert_eq!(wrap(&["abcde".into()], 2), ["ab", "cd", "e"]);
    }
}
