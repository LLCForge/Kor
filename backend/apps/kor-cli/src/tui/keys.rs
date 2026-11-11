//! Key maps of the full-screen TUI (`ui.tui_keys`): `default`, `vim` and `emacs`. A key press
//! becomes an [`Action`]; the reducer decides what the action means for the focused panel.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
pub use kor_config::TuiKeys as Keys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Char(char),
    Backspace,
    Submit,
    FocusNext,
    FocusPrev,
    Up,
    Down,
    PageUp,
    PageDown,
    /// Keep the selected file change.
    Accept,
    /// Revert the selected file change from the undo journal.
    Reject,
    Refresh,
    /// Back to the input line.
    ToInput,
    /// vim `:`: the input line with `/` (`:q` → `/q`).
    Command,
    /// Ctrl+C: stop the running turn, or quit when idle.
    Interrupt,
    Quit,
    /// vim `v`: start selecting agent output lines (§28.10).
    Visual,
    /// vim `y` in visual mode: quote the selected lines into the input line.
    Yank,
    /// Esc in visual mode.
    VisualEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyState {
    pub keys: Keys,
    /// vim insert mode (the TUI starts in it: it is a chat composer first).
    pub insert: bool,
    /// vim visual mode (selecting output lines).
    pub visual: bool,
    /// emacs `C-x` prefix pressed.
    chord: bool,
}

impl KeyState {
    pub fn new(keys: Keys) -> Self {
        Self {
            keys,
            insert: true,
            visual: false,
            chord: false,
        }
    }

    /// Whether printable keys type into the input line.
    pub fn typing(self, input_focused: bool) -> bool {
        input_focused && (self.keys != Keys::Vim || self.insert)
    }

    /// Map one key event. `input_focused`: the input line has focus (no panel selected).
    #[allow(clippy::too_many_lines)] // one table per layout
    pub fn map(&mut self, input_focused: bool, key: KeyEvent) -> Option<Action> {
        // Windows (ConPTY) also reports releases; only presses and repeats act.
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let m = key.modifiers;
        // AltGr arrives as Ctrl+Alt on Windows: that is a printable character, not a shortcut.
        let ctrl = m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT);
        let alt = m.contains(KeyModifiers::ALT) && !m.contains(KeyModifiers::CONTROL);
        if self.keys == Keys::Emacs && self.chord {
            self.chord = false;
            return match key.code {
                KeyCode::Char('o') => Some(Action::FocusNext),
                KeyCode::Char('c') if ctrl => Some(Action::Quit),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('c') if ctrl => return Some(Action::Interrupt),
            KeyCode::Char('q') if ctrl => return Some(Action::Quit),
            KeyCode::F(5) => return Some(Action::Refresh),
            KeyCode::Char('r') if ctrl => return Some(Action::Refresh),
            KeyCode::Tab => return Some(Action::FocusNext),
            KeyCode::BackTab => return Some(Action::FocusPrev),
            KeyCode::Up => return Some(Action::Up),
            KeyCode::Down => return Some(Action::Down),
            KeyCode::PageUp => return Some(Action::PageUp),
            KeyCode::PageDown => return Some(Action::PageDown),
            _ => {}
        }
        let typing = self.typing(input_focused);
        match self.keys {
            Keys::Default => match key.code {
                KeyCode::Char(c) if typing && !ctrl && !alt => Some(Action::Char(c)),
                KeyCode::Backspace if typing => Some(Action::Backspace),
                KeyCode::Enter if typing => Some(Action::Submit),
                KeyCode::Esc if typing => Some(Action::Quit),
                _ if typing => None,
                KeyCode::Char('j') => Some(Action::Down),
                KeyCode::Char('k') => Some(Action::Up),
                KeyCode::Char('a') | KeyCode::Enter => Some(Action::Accept),
                KeyCode::Char('r') | KeyCode::Delete => Some(Action::Reject),
                KeyCode::Char('i') | KeyCode::Esc => Some(Action::ToInput),
                _ => None,
            },
            Keys::Vim if self.visual => match key.code {
                KeyCode::Esc => {
                    self.visual = false;
                    Some(Action::VisualEnd)
                }
                KeyCode::Char('y') => {
                    self.visual = false;
                    self.insert = true;
                    Some(Action::Yank)
                }
                KeyCode::Char('j') => Some(Action::Down),
                KeyCode::Char('k') => Some(Action::Up),
                _ => None,
            },
            Keys::Vim => match key.code {
                KeyCode::Char('v') if !typing => {
                    self.visual = true;
                    Some(Action::Visual)
                }
                KeyCode::Esc if typing => {
                    self.insert = false;
                    None
                }
                KeyCode::Char(c) if typing && !ctrl && !alt => Some(Action::Char(c)),
                KeyCode::Backspace if typing => Some(Action::Backspace),
                KeyCode::Enter if typing => Some(Action::Submit),
                _ if typing => None,
                KeyCode::Char('d') if ctrl => Some(Action::PageDown),
                KeyCode::Char('u') if ctrl => Some(Action::PageUp),
                KeyCode::Char('j') => Some(Action::Down),
                KeyCode::Char('k') => Some(Action::Up),
                KeyCode::Char('l') => Some(Action::FocusNext),
                KeyCode::Char('h') => Some(Action::FocusPrev),
                KeyCode::Char('i') => {
                    self.insert = true;
                    Some(Action::ToInput)
                }
                KeyCode::Char(':') => {
                    self.insert = true;
                    Some(Action::Command)
                }
                KeyCode::Char('a') | KeyCode::Enter => Some(Action::Accept),
                KeyCode::Char('r' | 'x') => Some(Action::Reject),
                _ => None,
            },
            Keys::Emacs => match key.code {
                KeyCode::Char('x') if ctrl => {
                    self.chord = true;
                    None
                }
                KeyCode::Char('n') if ctrl => Some(Action::Down),
                KeyCode::Char('p') if ctrl => Some(Action::Up),
                KeyCode::Char('v') if ctrl => Some(Action::PageDown),
                KeyCode::Char('v') if alt => Some(Action::PageUp),
                KeyCode::Char('g') if ctrl => Some(Action::ToInput),
                KeyCode::Char('h') if ctrl && typing => Some(Action::Backspace),
                KeyCode::Char(c) if typing && !ctrl && !alt => Some(Action::Char(c)),
                KeyCode::Backspace if typing => Some(Action::Backspace),
                KeyCode::Enter if typing => Some(Action::Submit),
                _ if typing => None,
                KeyCode::Char('a') | KeyCode::Enter => Some(Action::Accept),
                KeyCode::Char('r') => Some(Action::Reject),
                _ => None,
            },
        }
    }

    /// A user binding from `keymap.toml` (§28.10) for this key, before the layout's own map.
    /// Plain characters still type while the input line takes text.
    pub fn map_override(&mut self, input_focused: bool, key: KeyEvent, overrides: &[(String, Action)]) -> Option<Action> {
        if key.kind == KeyEventKind::Release || overrides.is_empty() {
            return None;
        }
        let plain_char = matches!(key.code, KeyCode::Char(_))
            && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        if plain_char && self.typing(input_focused) {
            return None;
        }
        let name = key_name(key)?;
        overrides.iter().find(|(k, _)| *k == name).map(|(_, a)| *a)
    }

    /// Localization key of the one-line key help.
    pub fn help_key(self) -> &'static str {
        if self.visual {
            return "tui-help-visual";
        }
        match self.keys {
            Keys::Default => "tui-help-default",
            Keys::Vim => "tui-help-vim",
            Keys::Emacs => "tui-help-emacs",
        }
    }
}

/// `KeyEvent` as a `keymap.toml` key (`ctrl+g`, `shift+tab`, `f5`), same form as
/// `kor_runtime::prefs::normalize_key`.
pub fn key_name(key: KeyEvent) -> Option<String> {
    let m = key.modifiers;
    let mut shift = m.contains(KeyModifiers::SHIFT);
    let base = match key.code {
        KeyCode::Char(' ') => "space".to_owned(),
        KeyCode::Char(c) => {
            if c.is_uppercase() {
                shift = true;
            }
            c.to_lowercase().collect()
        }
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "escape".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            shift = true;
            "tab".into()
        }
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::F(n) => format!("f{n}"),
        _ => return None,
    };
    let mut parts = Vec::new();
    if m.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if m.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    if shift {
        parts.push("shift");
    }
    if m.contains(KeyModifiers::SUPER) || m.contains(KeyModifiers::META) {
        parts.push("meta");
    }
    parts.push(&base);
    Some(parts.join("+"))
}

/// A `keymap.toml` action name (`kor_runtime::prefs::TUI_ACTIONS`).
pub fn action_named(name: &str) -> Option<Action> {
    Some(match name {
        "submit" => Action::Submit,
        "backspace" => Action::Backspace,
        "focus_next" => Action::FocusNext,
        "focus_prev" => Action::FocusPrev,
        "up" => Action::Up,
        "down" => Action::Down,
        "page_up" => Action::PageUp,
        "page_down" => Action::PageDown,
        "accept" => Action::Accept,
        "reject" => Action::Reject,
        "refresh" => Action::Refresh,
        "to_input" => Action::ToInput,
        "command" => Action::Command,
        "interrupt" => Action::Interrupt,
        "quit" => Action::Quit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn default_types_in_input_and_navigates_panels() {
        let mut s = KeyState::new(Keys::Default);
        assert_eq!(s.map(true, k(KeyCode::Char('j'))), Some(Action::Char('j')));
        assert_eq!(s.map(true, k(KeyCode::Enter)), Some(Action::Submit));
        assert_eq!(s.map(true, k(KeyCode::Tab)), Some(Action::FocusNext));
        assert_eq!(s.map(false, k(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(s.map(false, k(KeyCode::Char('a'))), Some(Action::Accept));
        assert_eq!(s.map(false, k(KeyCode::Char('r'))), Some(Action::Reject));
        assert_eq!(s.map(false, k(KeyCode::Esc)), Some(Action::ToInput));
        assert_eq!(s.map(true, ctrl('c')), Some(Action::Interrupt));
        assert_eq!(s.map(true, k(KeyCode::F(5))), Some(Action::Refresh));
    }

    #[test]
    fn releases_are_ignored_and_altgr_types() {
        let mut s = KeyState::new(Keys::Default);
        let mut release = k(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        assert_eq!(s.map(true, release), None);
        let altgr = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(s.map(true, altgr), Some(Action::Char('@')));
    }

    #[test]
    fn vim_modes_and_colon_commands() {
        let mut s = KeyState::new(Keys::Vim);
        assert_eq!(s.map(true, k(KeyCode::Char('j'))), Some(Action::Char('j')));
        assert_eq!(s.map(true, k(KeyCode::Esc)), None);
        assert!(!s.insert);
        assert_eq!(s.map(true, k(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(s.map(true, ctrl('d')), Some(Action::PageDown));
        assert_eq!(s.map(true, k(KeyCode::Char('l'))), Some(Action::FocusNext));
        assert_eq!(s.map(false, k(KeyCode::Char('x'))), Some(Action::Reject));
        assert_eq!(s.map(true, k(KeyCode::Char(':'))), Some(Action::Command));
        assert!(s.insert);
        assert_eq!(s.map(true, k(KeyCode::Char('q'))), Some(Action::Char('q')));
        s.insert = false;
        assert_eq!(s.map(false, k(KeyCode::Char('i'))), Some(Action::ToInput));
        assert!(s.insert);
    }

    #[test]
    fn vim_visual_mode() {
        let mut s = KeyState::new(Keys::Vim);
        s.insert = false;
        assert_eq!(s.map(true, k(KeyCode::Char('v'))), Some(Action::Visual));
        assert_eq!(s.map(true, k(KeyCode::Char('k'))), Some(Action::Up));
        assert_eq!(s.map(true, k(KeyCode::Char('y'))), Some(Action::Yank));
        assert!(!s.visual && s.insert);
        s.insert = false;
        s.map(true, k(KeyCode::Char('v')));
        assert_eq!(s.map(true, k(KeyCode::Esc)), Some(Action::VisualEnd));
        assert!(!s.visual);
    }

    #[test]
    fn keymap_overrides_and_names() {
        assert_eq!(key_name(ctrl('g')).as_deref(), Some("ctrl+g"));
        assert_eq!(key_name(k(KeyCode::BackTab)).as_deref(), Some("shift+tab"));
        assert_eq!(key_name(k(KeyCode::Char('G'))).as_deref(), Some("shift+g"));
        assert_eq!(key_name(k(KeyCode::F(2))).as_deref(), Some("f2"));
        let names: Vec<&str> = vec!["submit", "backspace", "focus_next", "focus_prev", "up", "down", "page_up", "page_down", "accept", "reject", "refresh", "to_input", "command", "interrupt", "quit"];
        assert!(names.iter().all(|n| action_named(n).is_some()));
        assert_eq!(action_named("fly"), None);
        let ov = vec![("ctrl+g".to_owned(), Action::Quit), ("j".to_owned(), Action::Refresh)];
        let mut s = KeyState::new(Keys::Default);
        assert_eq!(s.map_override(true, ctrl('g'), &ov), Some(Action::Quit));
        assert_eq!(s.map_override(true, k(KeyCode::Char('j')), &ov), None, "typing wins in the input line");
        assert_eq!(s.map_override(false, k(KeyCode::Char('j')), &ov), Some(Action::Refresh));
    }

    #[test]
    fn emacs_chords() {
        let mut s = KeyState::new(Keys::Emacs);
        assert_eq!(s.map(true, ctrl('n')), Some(Action::Down));
        assert_eq!(s.map(true, ctrl('x')), None);
        assert_eq!(s.map(true, k(KeyCode::Char('o'))), Some(Action::FocusNext));
        assert_eq!(s.map(true, k(KeyCode::Char('o'))), Some(Action::Char('o')));
        assert_eq!(s.map(true, ctrl('x')), None);
        assert_eq!(s.map(true, ctrl('c')), Some(Action::Quit));
        assert_eq!(s.map(false, ctrl('g')), Some(Action::ToInput));
        assert_eq!(s.map(false, k(KeyCode::Char('r'))), Some(Action::Reject));
        assert_eq!(s.help_key(), "tui-help-emacs");
    }
}
