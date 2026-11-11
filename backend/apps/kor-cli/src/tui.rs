//! Full-screen terminal UI (Master Build Prompt §15.18). [`Screen`] is the state, [`reduce`] the
//! headless reducer that turns agent events, key actions and mouse input into panel state, and
//! [`view::draw`] paints it with ratatui. File I/O (undo journal, diff, revert) lives in the small
//! helpers below the reducer and runs only when the reducer asks for it.

pub mod changes;
// §16.4: panels drawn from real core events (ADR 0061).
#[cfg(test)]
mod core_events;
pub mod data;
pub mod graph;
pub mod keys;
pub mod review;
pub mod term;
pub mod view;

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use kor_agent::PermissionReply;
use kor_i18n::{Localizer, args};
use ratatui::layout::Rect;
use ratatui::style::Color;

pub use view::draw;

use changes::Change;
use keys::{Action, KeyState, Keys};

/// Lines kept in the agent stream.
const MAX_LINES: usize = 5000;
const MAX_NOTICES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Stream,
    Tasks,
    Changes,
    Cost,
    Context,
    Health,
    Notices,
    /// Project graph summary (§15.18, Faz 3): selected node card, hotspots, search.
    Graph,
}

pub const PANELS: [Panel; 8] = [
    Panel::Stream,
    Panel::Tasks,
    Panel::Changes,
    Panel::Cost,
    Panel::Context,
    Panel::Health,
    Panel::Notices,
    Panel::Graph,
];

impl Panel {
    pub fn key(self) -> &'static str {
        match self {
            Self::Stream => "tui-panel-stream",
            Self::Tasks => "tui-panel-tasks",
            Self::Changes => "tui-panel-changes",
            Self::Cost => "tui-panel-cost",
            Self::Context => "tui-panel-context",
            Self::Health => "tui-panel-health",
            Self::Notices => "tui-panel-notices",
            Self::Graph => "tui-panel-graph",
        }
    }

    fn idx(self) -> usize {
        PANELS.iter().position(|p| *p == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // UI state flags
pub struct Screen {
    /// The agent stream.
    pub lines: Vec<String>,
    /// The last stream line is model text still being streamed into.
    open: bool,
    pub input: String,
    pub mode: String,
    pub model: String,
    pub theme: String,
    pub perm: Option<String>,
    /// Files of the open request (§12.3) and which the user toggled off by number.
    pub perm_files: Vec<String>,
    pub perm_rejected: Vec<bool>,
    /// Content typed in `$EDITOR` for file n of a reviewed write (`e` then the number).
    pub perm_edited: Vec<Option<String>>,
    /// The proposed content of each reviewed file (what the editor opens with).
    pub perm_texts: Vec<String>,
    /// `e` was pressed: the next digit opens that file in the editor.
    pub perm_edit_next: bool,
    /// "Destructive: …" of the open request; the prompt turns red.
    pub perm_danger: Option<String>,
    /// §28.13 options of an open question (`AskUserQuestion`), the selected one and "always".
    pub perm_choices: Vec<String>,
    pub perm_choice: usize,
    pub perm_remember: bool,
    /// §28.1 the open request is the compaction preview: numbers mark items to keep.
    pub perm_compact: bool,
    pub plan_dialog: bool,
    pub tip_shown: bool,
    /// `None`: the input line.
    pub focus: Option<Panel>,
    /// Side panel shown in the compact (80×24) layout.
    pub dock: Panel,
    /// Per panel: lines up from the bottom (stream, notices) or down from the top (others).
    pub scroll: [usize; PANELS.len()],
    pub keys: KeyState,
    /// `keymap.toml` `[tui]` bindings (§28.10), checked before the layout's own keys.
    pub keymap: Vec<(String, Action)>,
    /// vim visual mode: the stream scroll position where the selection started.
    pub visual: Option<usize>,
    /// The message the agent is working on.
    pub busy: Option<String>,
    /// The core's latest `agent.status` while busy (§16.2): the TUI draws its verb, never its own.
    pub status: Option<kor_agent::status::AgentStatus>,
    pub tool: Option<String>,
    /// Background jobs: (running, total).
    pub jobs: (usize, usize),
    /// Changed files not yet kept or reverted here.
    pub changes: Vec<Change>,
    /// Files kept or reverted here, with their modification time then: a later change shows again.
    pub hidden: Vec<(PathBuf, Option<SystemTime>)>,
    pub selected: usize,
    pub diff: Vec<String>,
    pub cost: Vec<String>,
    pub quota: Vec<String>,
    pub health: Vec<String>,
    pub context: Vec<String>,
    pub notices: Vec<String>,
    /// Tokens this session: (input, output).
    pub tokens: (u64, u64),
    /// Graph panel lines (`tui::graph::fill`), its selected list row and the `/gfind` search.
    pub graph: Vec<String>,
    pub graph_sel: usize,
    pub graph_items: usize,
    pub graph_query: String,
}

impl Screen {
    pub fn new(mode: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            lines: Vec::new(),
            open: false,
            input: String::new(),
            mode: mode.into(),
            model: model.into(),
            theme: "dark".into(),
            perm: None,
            perm_files: Vec::new(),
            perm_rejected: Vec::new(),
            perm_edited: Vec::new(),
            perm_texts: Vec::new(),
            perm_edit_next: false,
            perm_danger: None,
            perm_choices: Vec::new(),
            perm_choice: 0,
            perm_remember: false,
            perm_compact: false,
            plan_dialog: false,
            tip_shown: false,
            focus: None,
            dock: Panel::Tasks,
            scroll: [0; PANELS.len()],
            keys: KeyState::new(Keys::Default),
            keymap: Vec::new(),
            visual: None,
            busy: None,
            status: None,
            tool: None,
            jobs: (0, 0),
            changes: Vec::new(),
            hidden: Vec::new(),
            selected: 0,
            diff: Vec::new(),
            cost: Vec::new(),
            quota: Vec::new(),
            health: Vec::new(),
            context: Vec::new(),
            notices: Vec::new(),
            tokens: (0, 0),
            graph: Vec::new(),
            graph_sel: 0,
            graph_items: 0,
            graph_query: String::new(),
        }
    }

    #[must_use]
    pub fn with_keys(mut self, keys: Keys) -> Self {
        self.keys = KeyState::new(keys);
        self
    }

    /// `keymap.toml` bindings; unknown action names were already reported by the loader.
    #[must_use]
    pub fn with_keymap(mut self, map: &std::collections::BTreeMap<String, String>) -> Self {
        self.keymap = map
            .iter()
            .filter_map(|(k, a)| Some((k.clone(), keys::action_named(a)?)))
            .collect();
        self
    }

    pub fn selected_change(&self) -> Option<&Change> {
        self.changes.get(self.selected)
    }

    fn push(&mut self, line: String) {
        self.lines.push(line);
        self.open = false;
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - MAX_LINES);
        }
    }

    pub fn notice(&mut self, n: String) {
        self.notices.push(n);
        if self.notices.len() > MAX_NOTICES {
            self.notices.remove(0);
        }
    }

    fn scroll_by(&mut self, panel: Panel, up: bool, step: usize) {
        let s = &mut self.scroll[panel.idx()];
        // The stream and notices count from the bottom, the others from the top.
        let toward_start = matches!(panel, Panel::Stream | Panel::Notices) == up;
        *s = if toward_start {
            s.saturating_add(step)
        } else {
            s.saturating_sub(step)
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ev {
    Char(char),
    Backspace,
    Enter,
    /// Model text (streamed in pieces) or thinking.
    Text(String),
    /// A tool started: `name summary`.
    Tool(String),
    ToolEnd {
        name: String,
        is_error: bool,
    },
    /// A line a long-running tool reported (`AgentEvent::ToolProgress` message).
    Progress(String),
    Notice(String),
    Context(kor_agent::context::Breakdown),
    /// Tokens of one model call: (input, output).
    Usage(u64, u64),
    /// A turn started (the message) or ended.
    Busy(Option<String>),
    /// `agent.status` from the core: verb, elapsed, tokens (§16.2).
    Status(kor_agent::status::AgentStatus),
    Ask(String),
    AskPlan(String),
    Act(Action),
    /// Mouse: click a panel (and a row inside it).
    Click(Panel, Option<usize>),
    /// Mouse wheel over a panel.
    Wheel(Panel, bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    None,
    Submit(String),
    Decision(PermissionReply),
    Quit,
    Interrupt,
    /// Re-read everything, quotas included (network only as `kor quota` does).
    Refresh,
    /// Re-read local state: undo journal, jobs, cost, provider health.
    Sync,
    /// Re-read the diff of the selected change.
    Diff,
    /// Keep the selected change (hide it).
    Keep(Change),
    /// Revert the selected change from the undo journal.
    Revert(Change),
    /// Re-read the graph panel (selection or search changed).
    Graph,
    /// Open file n (0-based) of a reviewed write in `$EDITOR`.
    EditFile(usize),
}

#[allow(clippy::too_many_lines)]
pub fn reduce(screen: &mut Screen, l: &Localizer, ev: Ev) -> Out {
    // §28.13: a question: digits pick, `!` = always (a rule), Enter/a = this one, d = recommended.
    if let Some(prompt) = screen.perm.clone()
        && !screen.perm_choices.is_empty()
    {
        let n = screen.perm_choices.len();
        let decision = match ev {
            Ev::Char(c)
                if c.to_digit(10)
                    .is_some_and(|d| (1..=n).contains(&(d as usize))) =>
            {
                screen.perm_choice = c.to_digit(10).map_or(0, |d| d as usize - 1);
                return Out::None;
            }
            Ev::Char('!') => {
                screen.perm_remember = !screen.perm_remember;
                return Out::None;
            }
            Ev::Enter | Ev::Char('a' | 'y') => PermissionReply::Allow,
            Ev::Char('d' | 'n') => PermissionReply::Deny,
            _ => return Out::None,
        };
        let picked = screen
            .perm_choices
            .get(screen.perm_choice)
            .cloned()
            .unwrap_or_default();
        screen.push(format!("{prompt} → {picked}"));
        screen.perm = None;
        return Out::Decision(decision);
    }
    // §28.1: Enter in the compaction preview summarizes (keeping the marked items).
    let ev = if screen.perm.is_some() && screen.perm_compact && ev == Ev::Enter {
        Ev::Char('a')
    } else {
        ev
    };
    if let Some(prompt) = screen.perm.clone()
        && let Ev::Char(ch) = ev
    {
        if screen.perm_edit_next {
            screen.perm_edit_next = false;
            if let Some(i) = ch.to_digit(10).and_then(|d| (d as usize).checked_sub(1))
                && i < screen.perm_texts.len()
            {
                return Out::EditFile(i);
            }
        }
        if ch == 'e' && !screen.plan_dialog && !screen.perm_texts.is_empty() {
            if screen.perm_texts.len() == 1 {
                return Out::EditFile(0);
            }
            screen.perm_edit_next = true;
            return Out::None;
        }
        if let Some(i) = ch.to_digit(10).and_then(|d| (d as usize).checked_sub(1))
            && !screen.plan_dialog
            && let Some(off) = screen.perm_rejected.get_mut(i)
        {
            *off = !*off;
            return Out::None;
        }
        let decision = match ch {
            'y' if screen.plan_dialog => Some(PermissionReply::Allow),
            'n' | 'N' if screen.plan_dialog => Some(PermissionReply::Deny),
            'a' => Some(PermissionReply::Allow),
            'A' => Some(PermissionReply::AllowAlways),
            'd' | 'D' => Some(PermissionReply::Deny),
            _ => None,
        };
        if let Some(decision) = decision {
            if screen.plan_dialog && decision == PermissionReply::Allow {
                screen.mode = "plan".into();
            }
            let word = l.t(match decision {
                PermissionReply::Allow => "tui-allowed",
                PermissionReply::AllowAlways => "tui-allowed-always",
                PermissionReply::Deny => "tui-denied",
            });
            screen.push(format!("{prompt} → {word}"));
            screen.perm = None;
            screen.plan_dialog = false;
            return Out::Decision(decision);
        }
        return Out::None;
    }
    match ev {
        Ev::Char(ch) => {
            screen.input.push(ch);
            Out::None
        }
        Ev::Backspace => {
            screen.input.pop();
            Out::None
        }
        Ev::Enter => {
            let line = std::mem::take(&mut screen.input);
            if line.trim().is_empty() || line == "/" {
                if !screen.tip_shown {
                    screen.tip_shown = true;
                    screen.push(l.t("tui-tip"));
                }
                return Out::None;
            }
            if line == "/q" {
                return Out::Quit;
            }
            // `/gfind <text>` searches the graph panel; `/gfind` alone goes back to hotspots.
            if let Some(q) = line
                .strip_prefix("/gfind")
                .filter(|r| r.is_empty() || r.starts_with(' '))
            {
                q.trim().clone_into(&mut screen.graph_query);
                screen.graph_sel = 0;
                focus(screen, Some(Panel::Graph));
                return Out::Graph;
            }
            screen.push(format!("> {line}"));
            screen.scroll[Panel::Stream.idx()] = 0;
            if let Some(q) = line.strip_prefix("/find ") {
                let n = search(&screen.lines, q).len();
                screen.push(l.t_args("tui-find", &args!("count" => n, "text" => q.to_owned())));
            }
            Out::Submit(line)
        }
        Ev::Text(text) => {
            if text.contains("```mermaid") {
                screen.push(mermaid_outline(&text));
                return Out::None;
            }
            let mut parts = text.split('\n');
            if let Some(first) = parts.next() {
                match screen.lines.last_mut() {
                    Some(last) if screen.open => last.push_str(first),
                    _ => screen.lines.push(first.to_owned()),
                }
            }
            for p in parts {
                screen.lines.push(p.to_owned());
            }
            screen.open = true;
            Out::None
        }
        Ev::Tool(text) => {
            screen.push(format!("⏺ {text}"));
            screen.tool = Some(text);
            Out::None
        }
        Ev::ToolEnd { name, is_error } => {
            screen.push(format!("  ⎿ {name}{}", if is_error { " ✗" } else { "" }));
            screen.tool = None;
            Out::Sync
        }
        Ev::Progress(m) => {
            screen.push(format!("  … {m}"));
            Out::None
        }
        Ev::Notice(n) => {
            screen.notice(n);
            Out::None
        }
        Ev::Context(b) => {
            screen.context = kor_runtime::context::render(l, &b)
                .lines()
                .map(str::to_owned)
                .collect();
            if let Some(w) = kor_runtime::context::warning(l, &b) {
                screen.notice(w);
            }
            Out::None
        }
        Ev::Usage(input, output) => {
            screen.tokens.0 += input;
            screen.tokens.1 += output;
            Out::Sync
        }
        Ev::Busy(task) => {
            screen.busy = task;
            screen.status = None;
            screen.tool = None;
            Out::None
        }
        Ev::Status(s) => {
            screen.status = Some(s);
            Out::None
        }
        Ev::Ask(text) => {
            screen.perm = Some(text);
            screen.plan_dialog = false;
            Out::None
        }
        Ev::AskPlan(text) => {
            screen.perm = Some(text);
            screen.perm_files.clear();
            screen.perm_rejected.clear();
            screen.perm_texts.clear();
            screen.perm_edited.clear();
            screen.perm_danger = None;
            screen.plan_dialog = true;
            Out::None
        }
        Ev::Click(panel, row) => {
            focus(screen, Some(panel));
            if panel == Panel::Changes
                && let Some(r) = row
                && r < screen.changes.len()
                && r != screen.selected
            {
                screen.selected = r;
                return Out::Diff;
            }
            Out::None
        }
        Ev::Wheel(panel, up) => {
            screen.scroll_by(panel, up, 3);
            Out::None
        }
        Ev::Act(a) => act(screen, l, a),
    }
}

fn focus(screen: &mut Screen, to: Option<Panel>) {
    screen.focus = to;
    if let Some(p) = to
        && p != Panel::Stream
    {
        screen.dock = p;
    }
}

fn act(screen: &mut Screen, l: &Localizer, a: Action) -> Out {
    let target = screen.focus.unwrap_or(Panel::Stream);
    match a {
        Action::Char(c) => {
            screen.input.push(c);
            Out::None
        }
        Action::Backspace => {
            screen.input.pop();
            Out::None
        }
        Action::Submit => reduce(screen, l, Ev::Enter),
        Action::FocusNext | Action::FocusPrev => {
            // Order: input, then every panel.
            let n = PANELS.len() + 1;
            let cur = screen.focus.map_or(0, |p| p.idx() + 1);
            let next = if a == Action::FocusNext {
                (cur + 1) % n
            } else {
                (cur + n - 1) % n
            };
            focus(screen, next.checked_sub(1).map(|i| PANELS[i]));
            Out::None
        }
        Action::Up | Action::Down if target == Panel::Graph => {
            screen.graph_sel = if a == Action::Up {
                screen.graph_sel.saturating_sub(1)
            } else {
                (screen.graph_sel + 1).min(screen.graph_items.saturating_sub(1))
            };
            Out::Graph
        }
        Action::Up | Action::Down if target == Panel::Changes => {
            let before = screen.selected;
            screen.selected = if a == Action::Up {
                before.saturating_sub(1)
            } else {
                (before + 1).min(screen.changes.len().saturating_sub(1))
            };
            if screen.selected == before {
                Out::None
            } else {
                Out::Diff
            }
        }
        Action::Up | Action::Down => {
            screen.scroll_by(target, a == Action::Up, 1);
            Out::None
        }
        Action::PageUp | Action::PageDown => {
            screen.scroll_by(target, a == Action::PageUp, 10);
            Out::None
        }
        Action::Accept | Action::Reject if target == Panel::Changes => {
            let Some(c) = screen.selected_change().cloned() else {
                return Out::None;
            };
            if a == Action::Accept {
                Out::Keep(c)
            } else if screen.busy.is_some() {
                screen.notice(l.t("tui-revert-busy"));
                Out::None
            } else {
                Out::Revert(c)
            }
        }
        Action::Accept | Action::Reject => Out::None,
        Action::Refresh => Out::Refresh,
        Action::ToInput => {
            focus(screen, None);
            Out::None
        }
        Action::Command => {
            focus(screen, None);
            screen.input = "/".into();
            Out::None
        }
        Action::Interrupt => Out::Interrupt,
        Action::Quit => Out::Quit,
        Action::Visual => {
            focus(screen, Some(Panel::Stream));
            screen.visual = Some(screen.scroll[Panel::Stream.idx()]);
            Out::None
        }
        Action::VisualEnd => {
            screen.visual = None;
            Out::None
        }
        Action::Yank => {
            if let Some(anchor) = screen.visual.take() {
                screen.input = visual_quote(screen, anchor);
            }
            focus(screen, None);
            Out::None
        }
    }
}

/// The stream lines between the visual anchor and the current position (both counted from the
/// bottom), quoted for the input line.
pub fn visual_quote(screen: &Screen, anchor: usize) -> String {
    let n = screen.lines.len();
    let cur = screen.scroll[Panel::Stream.idx()];
    let (lo, hi) = (anchor.min(cur), anchor.max(cur));
    let from = n.saturating_sub(1 + hi);
    let to = n.saturating_sub(lo);
    screen.lines[from..to]
        .iter()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A terminal event as a reducer event: keys through the configured key map (raw characters
/// while a permission question waits), mouse clicks and wheel over the panel under the pointer.
pub fn input_ev(screen: &mut Screen, l: &Localizer, area: Rect, ev: &Event) -> Option<Ev> {
    match ev {
        Event::Key(key) => {
            if screen.perm.is_some()
                && key.kind != KeyEventKind::Release
                && let KeyCode::Char(c) = key.code
            {
                return Some(Ev::Char(c));
            }
            let input_focused = screen.focus.is_none();
            let keymap = std::mem::take(&mut screen.keymap);
            let over = screen.keys.map_override(input_focused, *key, &keymap);
            screen.keymap = keymap;
            let action = match over {
                Some(a) => a,
                None => screen.keys.map(input_focused, *key)?,
            };
            Some(match action {
                Action::Char(c) => Ev::Char(c),
                Action::Backspace => Ev::Backspace,
                Action::Submit => Ev::Enter,
                a => Ev::Act(a),
            })
        }
        Event::Mouse(m) => {
            let areas = view::layout(area, screen.dock);
            let (panel, rect) = match areas.hit(m.column, m.row) {
                Some((p, r)) => (p, Some(r)),
                None => (view::tab_at(l, &areas, m.column, m.row)?, None),
            };
            match m.kind {
                MouseEventKind::ScrollUp => Some(Ev::Wheel(panel, true)),
                MouseEventKind::ScrollDown => Some(Ev::Wheel(panel, false)),
                MouseEventKind::Down(MouseButton::Left) => {
                    let row = rect
                        .filter(|r| m.row > r.y)
                        .map(|r| usize::from(m.row - r.y - 1) + view::list_start(screen, r));
                    Some(Ev::Click(panel, row))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

// -------------------------------------------------------------------------------------------
// I/O the reducer asks for (`Out::Diff`, `Out::Keep`, `Out::Revert`, `Out::Sync`)
// -------------------------------------------------------------------------------------------

fn stamp(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Show the journal's changes, minus those kept or reverted here and unchanged since.
pub fn set_changes(screen: &mut Screen, l: &Localizer, all: Vec<Change>) {
    let hidden = &screen.hidden;
    let shown: Vec<Change> = all
        .into_iter()
        .filter(|c| {
            !hidden
                .iter()
                .any(|(p, t)| *p == c.path && *t == stamp(&c.path))
        })
        .collect();
    let prev = screen.selected_change().map(|c| c.path.clone());
    screen.changes = shown;
    screen.selected = prev
        .and_then(|p| screen.changes.iter().position(|c| c.path == p))
        .unwrap_or(0);
    fill_diff(screen, l);
}

/// Load the diff of the selected change.
pub fn fill_diff(screen: &mut Screen, l: &Localizer) {
    screen.diff = match screen.selected_change() {
        None => Vec::new(),
        Some(c) => changes::diff(c).unwrap_or_else(|k| vec![l.t(&format!("tui-diff-{k}"))]),
    };
    screen.scroll[Panel::Changes.idx()] = 0;
}

fn hide(screen: &mut Screen, l: &Localizer, c: &Change) {
    screen.hidden.retain(|(p, _)| *p != c.path);
    screen.hidden.push((c.path.clone(), stamp(&c.path)));
    screen.changes.retain(|x| x.path != c.path);
    screen.selected = screen.selected.min(screen.changes.len().saturating_sub(1));
    fill_diff(screen, l);
}

/// Keep a change: nothing is written, it only leaves the list.
pub fn keep(screen: &mut Screen, l: &Localizer, c: &Change) {
    hide(screen, l, c);
    let path = kor_tools::display_path(&c.path);
    screen.notice(l.t_args("tui-kept", &args!("path" => path)));
}

/// Revert one file to its original from the undo journal (the user pressed the key).
pub fn revert(screen: &mut Screen, l: &Localizer, c: &Change) {
    let path = kor_tools::display_path(&c.path);
    match changes::revert(c) {
        Ok(()) => {
            hide(screen, l, c);
            screen.notice(l.t_args("tui-reverted", &args!("path" => path)));
        }
        Err(e) => screen.notice(l.t_args(
            "tui-revert-failed",
            &args!("path" => path, "error" => e.to_string()),
        )),
    }
}

// -------------------------------------------------------------------------------------------

pub const THEMES: &[&str] = &[
    "dark",
    "light",
    "dark-daltonized",
    "light-daltonized",
    "dark-ansi",
    "light-ansi",
];

pub fn theme_color(name: &str) -> Option<Color> {
    match name {
        "dark" | "dark-daltonized" | "dark-ansi" => Some(Color::White),
        "light" | "light-daltonized" | "light-ansi" => Some(Color::Black),
        _ => None,
    }
}

/// §23.2 in the terminal: the provider mark left of the model name. Terminals cannot draw the SVG logos, so
/// the TUI uses the desktop's fallback: the provider's initial in a circle (Ⓓ), `⌂` for local runtimes;
/// models routed through `OpenRouter` show their real provider.
pub fn provider_glyph(model: &str) -> char {
    let mut parts = model.split('/');
    let first = parts.next().unwrap_or_default();
    let provider = match (first, parts.next()) {
        ("openrouter", Some(real)) if model.matches('/').count() >= 2 => real,
        (p, Some(_)) => p,
        (_, None) => "venice",
    };
    if provider == "local" || provider == "ollama" || provider.starts_with("local-") {
        return '\u{2302}';
    }
    provider
        .chars()
        .find(char::is_ascii_alphabetic)
        .and_then(|c| char::from_u32(0x24B6 + u32::from(c.to_ascii_uppercase()) - u32::from('A')))
        .unwrap_or('\u{25CF}')
}

/// `✶ Közlüyor… (12s)`: the verb and seconds are the core's; the frame turns every 100 ms on the
/// screen's own clock (the core reports once a second, which showed only every eighth frame).
pub fn status_line(s: &kor_agent::status::AgentStatus) -> String {
    status_line_at(s, anim_tick())
}

fn status_line_at(s: &kor_agent::status::AgentStatus, tick: usize) -> String {
    let frames = crate::brand::spinner_frames();
    format!(
        "{} {}… ({}s)",
        frames[tick % frames.len()],
        s.verb,
        s.elapsed_ms / 1000
    )
}

/// 100 ms animation ticks since the first frame of this process.
pub fn anim_tick() -> usize {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        / 100;
    usize::try_from(ms).unwrap_or(0)
}

/// Status glyphs: pause, play, running, done, fast.
pub fn sticker(kind: &str) -> Option<&'static str> {
    match kind {
        "pause" => Some("\u{23f8}"),
        "play" => Some("\u{25b6}"),
        "running" => Some("\u{25c7}"),
        "done" => Some("\u{25c6}"),
        "fast" => Some("\u{21af}"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub start: usize,
}

pub fn search(lines: &[String], needle: &str) -> Vec<Hit> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        let mut from = 0;
        while let Some(rel) = text[from..].find(needle) {
            hits.push(Hit {
                line,
                start: from + rel,
            });
            from += rel + needle.len();
            if from >= text.len() {
                break;
            }
        }
    }
    hits
}

/// Outline of a mermaid fence: node ids and `A --> B` edges.
pub fn mermaid_outline(fence: &str) -> String {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for raw in fence.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("flowchart") || line.starts_with("graph") {
            continue;
        }
        if let Some((left, right)) = line.split_once("-->") {
            let a = left.split('[').next().unwrap_or(left).trim();
            let b = right.split('[').next().unwrap_or(right).trim();
            if !nodes.contains(&a.to_owned()) {
                nodes.push(a.to_owned());
            }
            if !nodes.contains(&b.to_owned()) {
                nodes.push(b.to_owned());
            }
            edges.push(format!("{a} -> {b}"));
        }
    }
    format!("nodes: {}\n{}", nodes.join(", "), edges.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_i18n::Lang;

    fn en() -> Localizer {
        Localizer::new(Lang::En)
    }

    #[test]
    fn transcript_submit_and_permission_keys() {
        let l = en();
        let mut screen = Screen::new("default", "anthropic/claude");
        assert_eq!(reduce(&mut screen, &l, Ev::Char('h')), Out::None);
        assert_eq!(reduce(&mut screen, &l, Ev::Char('i')), Out::None);
        assert_eq!(reduce(&mut screen, &l, Ev::Enter), Out::Submit("hi".into()));
        assert!(screen.lines[0].contains("hi"));
        reduce(&mut screen, &l, Ev::Text("hello".into()));
        reduce(&mut screen, &l, Ev::Tool("Read a.rs".into()));
        assert_eq!(screen.tool.as_deref(), Some("Read a.rs"));
        reduce(&mut screen, &l, Ev::Ask("Bash cargo test".into()));
        assert!(screen.perm.is_some());
        assert_eq!(
            reduce(&mut screen, &l, Ev::Char('A')),
            Out::Decision(PermissionReply::AllowAlways)
        );
        assert!(screen.perm.is_none());
        reduce(&mut screen, &l, Ev::Char('/'));
        reduce(&mut screen, &l, Ev::Char('c'));
        assert_eq!(reduce(&mut screen, &l, Ev::Enter), Out::Submit("/c".into()));
        for c in "/q".chars() {
            reduce(&mut screen, &l, Ev::Char(c));
        }
        assert_eq!(reduce(&mut screen, &l, Ev::Enter), Out::Quit);
    }

    #[test]
    fn number_keys_toggle_files_of_a_reviewed_write() {
        let l = en();
        let mut screen = Screen::new("default", "m");
        screen.perm_files = vec!["a.rs".into(), "b.rs".into()];
        screen.perm_rejected = vec![false, false];
        reduce(&mut screen, &l, Ev::Ask("apply_patch".into()));
        assert_eq!(reduce(&mut screen, &l, Ev::Char('2')), Out::None);
        assert_eq!(reduce(&mut screen, &l, Ev::Char('9')), Out::None);
        assert_eq!(screen.perm_rejected, [false, true]);
        assert_eq!(
            reduce(&mut screen, &l, Ev::Char('a')),
            Out::Decision(PermissionReply::Allow)
        );
        let r = review::from_marks(
            PermissionReply::Allow,
            &screen.perm_files,
            &screen.perm_rejected,
        );
        assert_eq!(r.files.get("b.rs"), Some(&kor_agent::FileDecision::Reject));
    }

    #[test]
    fn e_then_a_number_opens_that_file_in_the_editor() {
        let l = en();
        let mut screen = Screen::new("default", "m");
        screen.perm_files = vec!["a.rs".into(), "b.rs".into()];
        screen.perm_rejected = vec![false, false];
        screen.perm_texts = vec!["a".into(), "b".into()];
        reduce(&mut screen, &l, Ev::Ask("apply_patch".into()));
        assert_eq!(reduce(&mut screen, &l, Ev::Char('e')), Out::None);
        assert_eq!(reduce(&mut screen, &l, Ev::Char('2')), Out::EditFile(1));
        assert_eq!(
            screen.perm_rejected,
            [false, false],
            "the digit after e does not toggle"
        );
        assert_eq!(reduce(&mut screen, &l, Ev::Char('2')), Out::None);
        assert_eq!(screen.perm_rejected, [false, true]);
    }

    #[test]
    fn search_themes_stickers_and_mermaid() {
        let lines = vec!["alpha beta".into(), "beta gamma".into()];
        let hits = search(&lines, "beta");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[1].line, 1);
        for name in THEMES {
            assert!(theme_color(name).is_some(), "{name}");
        }
        assert!(theme_color("nope").is_none());
        assert_eq!(sticker("pause"), Some("\u{23f8}"));
        assert_eq!(sticker("running"), Some("\u{25c7}"));
        let outline = mermaid_outline("flowchart LR\nA --> B\nB --> C\n");
        assert!(outline.contains("A -> B"), "{outline}");
        assert!(outline.contains("B -> C"), "{outline}");
        assert!(outline.contains("A, B, C"), "{outline}");
    }

    #[test]
    fn plan_dialog_yes_sets_plan_and_empty_enter_tips_once() {
        let l = en();
        let mut screen = Screen::new("default", "m");
        reduce(&mut screen, &l, Ev::AskPlan("Enter plan mode?".into()));
        assert!(screen.plan_dialog);
        assert_eq!(
            reduce(&mut screen, &l, Ev::Char('y')),
            Out::Decision(PermissionReply::Allow)
        );
        assert_eq!(screen.mode, "plan");
        assert_eq!(reduce(&mut screen, &l, Ev::Enter), Out::None);
        assert_eq!(screen.lines.last(), Some(&l.t("tui-tip")));
        let n = screen.lines.len();
        assert_eq!(reduce(&mut screen, &l, Ev::Enter), Out::None);
        assert_eq!(screen.lines.len(), n);
    }

    #[test]
    fn agent_events_fill_their_panels() {
        let l = en();
        let mut s = Screen::new("default", "m");
        reduce(&mut s, &l, Ev::Busy(Some("fix it".into())));
        reduce(&mut s, &l, Ev::Text("Hel".into()));
        reduce(&mut s, &l, Ev::Text("lo\nworld".into()));
        assert_eq!(s.lines, ["Hello", "world"]);
        reduce(&mut s, &l, Ev::Tool("Bash cargo build".into()));
        reduce(&mut s, &l, Ev::Progress("Compiling kor".into()));
        assert!(s.lines.last().is_some_and(|x| x.contains("Compiling kor")));
        assert_eq!(
            reduce(
                &mut s,
                &l,
                Ev::ToolEnd {
                    name: "Bash".into(),
                    is_error: true
                }
            ),
            Out::Sync
        );
        assert!(s.tool.is_none());
        assert_eq!(reduce(&mut s, &l, Ev::Usage(100, 20)), Out::Sync);
        reduce(&mut s, &l, Ev::Usage(5, 1));
        assert_eq!(s.tokens, (105, 21));
        reduce(&mut s, &l, Ev::Notice("budget 80%".into()));
        let b = kor_agent::context::Breakdown {
            window: 1000,
            history: 600,
            warning: Some("history".into()),
            ..Default::default()
        };
        reduce(&mut s, &l, Ev::Context(b));
        assert!(!s.context.is_empty());
        assert_eq!(s.notices.len(), 2);
        assert_eq!(s.notices[0], "budget 80%");
        let st = kor_agent::status::status_at(0, std::time::Duration::from_millis(4_500), 3);
        reduce(&mut s, &l, Ev::Status(st.clone()));
        assert_eq!(s.status.as_ref(), Some(&st));
        assert!(status_line(&st).contains(&format!("{}… (4s)", st.verb)));
        // The frame turns with the screen's clock, not the core's once-a-second report.
        let frames: Vec<String> = (0..8)
            .map(|t| status_line_at(&st, t).chars().next().unwrap().to_string())
            .collect();
        assert_eq!(frames, kor_core::theme::frames::EMBER);
        reduce(&mut s, &l, Ev::Busy(None));
        assert!(s.status.is_none());
        assert_eq!(provider_glyph("openai/gpt-5"), 'Ⓞ');
        assert_eq!(provider_glyph("deepseek-v4"), 'Ⓥ');
        assert_eq!(provider_glyph("openrouter/deepseek/deepseek-v4"), 'Ⓓ');
        assert_eq!(provider_glyph("local-lmstudio/qwen3"), '⌂');
        assert_eq!(provider_glyph("9x/m"), 'Ⓧ');
        assert!(s.busy.is_none());
    }

    #[test]
    fn focus_cycles_and_changes_keep_or_revert_only_on_request() {
        let l = en();
        let mut s = Screen::new("default", "m");
        let c = |p: &str| Change {
            path: PathBuf::from(p),
            blob: None,
            skipped: false,
        };
        s.changes = vec![c("a"), c("b")];
        reduce(&mut s, &l, Ev::Act(Action::FocusNext));
        assert_eq!(s.focus, Some(Panel::Stream));
        reduce(&mut s, &l, Ev::Act(Action::FocusNext));
        reduce(&mut s, &l, Ev::Act(Action::FocusNext));
        assert_eq!(s.focus, Some(Panel::Changes));
        assert_eq!(s.dock, Panel::Changes);
        assert_eq!(reduce(&mut s, &l, Ev::Act(Action::Down)), Out::Diff);
        assert_eq!(s.selected, 1);
        assert_eq!(reduce(&mut s, &l, Ev::Act(Action::Down)), Out::None);
        assert_eq!(
            reduce(&mut s, &l, Ev::Act(Action::Accept)),
            Out::Keep(c("b"))
        );
        s.busy = Some("x".into());
        assert_eq!(reduce(&mut s, &l, Ev::Act(Action::Reject)), Out::None);
        assert_eq!(s.notices.len(), 1);
        s.busy = None;
        assert_eq!(
            reduce(&mut s, &l, Ev::Act(Action::Reject)),
            Out::Revert(c("b"))
        );
        reduce(&mut s, &l, Ev::Act(Action::FocusPrev));
        assert_eq!(s.focus, Some(Panel::Tasks));
        // Accept/Reject outside the changes panel do nothing.
        assert_eq!(reduce(&mut s, &l, Ev::Act(Action::Reject)), Out::None);
        reduce(&mut s, &l, Ev::Act(Action::Command));
        assert_eq!((s.focus, s.input.as_str()), (None, "/"));
        assert_eq!(
            reduce(&mut s, &l, Ev::Click(Panel::Changes, Some(0))),
            Out::Diff
        );
        assert_eq!(s.selected, 0);
        reduce(&mut s, &l, Ev::Wheel(Panel::Stream, true));
        assert_eq!(s.scroll[0], 3);
    }

    #[test]
    fn keep_and_revert_hide_until_the_file_changes_again() {
        let l = en();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "old\n").unwrap();
        let base = tmp.path().join("undo");
        let mut journal =
            kor_tools::undo::Journal::open(&base, "s", std::slice::from_ref(&root)).unwrap();
        journal.record(&root, "a.txt").unwrap();
        journal.record(&root, "b.txt").unwrap();
        std::fs::write(root.join("a.txt"), "new\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        let dir = base.join("s");

        let mut s = Screen::new("default", "m");
        set_changes(&mut s, &l, changes::load(&dir));
        assert_eq!(s.changes.len(), 2);
        assert!(s.diff.iter().any(|d| d.contains("+ new")), "{:?}", s.diff);
        let first = s.changes[0].clone();
        revert(&mut s, &l, &first);
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "old\n"
        );
        let second = s.changes[0].clone();
        keep(&mut s, &l, &second);
        assert!(root.join("b.txt").exists());
        assert!(s.changes.is_empty());
        set_changes(&mut s, &l, changes::load(&dir));
        assert!(s.changes.is_empty());
        assert_eq!(s.notices.len(), 2);
    }
}
