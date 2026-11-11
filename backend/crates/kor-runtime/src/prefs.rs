//! Settings shared by the CLI, TUI and desktop (Master Build Prompt §28.9, 28.10, 28.14, 26.1;
//! ADR 0050): the `[auto]` inventory, `keymap.toml`, readability values, config change logging and
//! the safe housekeeping fixes of `kor doctor --fix`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kor_config::{AUTO_KEYS, Layer, Loaded, TuiKeys};
use serde::Serialize;
use serde_json::{Value, json};

// ------------------------------------------------------------------------------------------------
// [auto]
// ------------------------------------------------------------------------------------------------

/// One `[auto]` switch for `kor config explain auto` and the desktop "Automations" tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRow {
    /// `compact` (the config key is `auto.compact`).
    pub key: String,
    pub value: bool,
    /// `default`, `env`, `cli` or the file that set it.
    pub source: String,
    pub from_default: bool,
    /// Older keys that must be on too, with their effective values.
    pub related: Vec<(String, String)>,
}

fn setting_value(loaded: &Loaded, key: &str) -> String {
    loaded
        .settings
        .get(key)
        .map_or_else(|| "-".to_owned(), |s| s.value.to_string())
}

pub fn auto_rows(loaded: &Loaded) -> Vec<AutoRow> {
    AUTO_KEYS
        .iter()
        .map(|(k, related)| {
            let full = format!("auto.{k}");
            let s = loaded.settings.get(&full);
            AutoRow {
                key: (*k).to_owned(),
                value: loaded.config.auto.get(k).unwrap_or(true),
                source: s.map_or_else(|| "default".into(), |s| match s.layer {
                    Layer::Default => "default".into(),
                    Layer::Env | Layer::Cli | Layer::System | Layer::User | Layer::Project => s.origin.clone(),
                }),
                from_default: s.is_none_or(|s| s.layer == Layer::Default),
                related: related.iter().map(|r| ((*r).to_owned(), setting_value(loaded, r))).collect(),
            }
        })
        .collect()
}

/// §28.14 "every change is logged": a config change goes to the hash-chained audit log.
pub fn log_config_change(key: &str, value: &str, file: &Path) {
    if let Some(home) = crate::kor_home() {
        let _ = kor_agent::audit::append(
            &home.join(kor_agent::audit::FILE),
            "config.set",
            json!({ "key": key, "value": kor_agent::audit::clip(value), "file": file.display().to_string() }),
        );
    }
}

// ------------------------------------------------------------------------------------------------
// keymap.toml (§28.10)
// ------------------------------------------------------------------------------------------------

/// Actions a TUI key can be bound to (`[tui]` table).
pub const TUI_ACTIONS: &[&str] = &[
    "submit", "backspace", "focus_next", "focus_prev", "up", "down", "page_up", "page_down", "accept",
    "reject", "refresh", "to_input", "command", "interrupt", "quit",
];

/// Actions a desktop composer key can be bound to (`[composer]` table).
pub const COMPOSER_ACTIONS: &[&str] = &[
    "submit", "submit_now", "newline", "cancel", "mode_cycle", "keys_cycle", "line_start", "line_end",
    "char_back", "char_forward", "word_back", "word_forward", "line_up", "line_down", "kill_line",
    "delete_char", "delete_word_back", "yank",
];

/// Built-in TUI keys that work in every layout; rebinding one is reported.
const TUI_BUILTIN: &[(&str, &str)] = &[
    ("ctrl+c", "interrupt"),
    ("ctrl+q", "quit"),
    ("ctrl+r", "refresh"),
    ("f5", "refresh"),
    ("tab", "focus_next"),
    ("shift+tab", "focus_prev"),
    ("up", "up"),
    ("down", "down"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
];

const COMPOSER_BUILTIN: &[(&str, &str)] = &[
    ("enter", "submit"),
    ("ctrl+enter", "submit_now"),
    ("shift+enter", "newline"),
    ("shift+tab", "mode_cycle"),
    ("escape", "cancel"),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Keymap {
    /// `mode` from the files (project over user); `None` = config `keys.mode` / `ui.tui_keys`.
    pub mode: Option<TuiKeys>,
    /// Normalized key (`ctrl+k`) → action.
    pub tui: BTreeMap<String, String>,
    pub composer: BTreeMap<String, String>,
    /// Files read, user first.
    pub files: Vec<PathBuf>,
    /// Invalid keys/actions, project-over-user overrides, and rebound built-ins.
    pub warnings: Vec<String>,
}

const MODS: &[&str] = &["ctrl", "alt", "shift", "meta"];
const NAMED: &[&str] = &[
    "enter", "escape", "esc", "tab", "backspace", "delete", "up", "down", "left", "right", "home", "end",
    "pageup", "pagedown", "space", "insert",
];

/// `Ctrl+Shift+K` → `ctrl+shift+k`; `None` when it is not a key.
pub fn normalize_key(spec: &str) -> Option<String> {
    let parts: Vec<String> = spec.split('+').map(|p| p.trim().to_lowercase()).collect();
    let (key, mods) = parts.split_last()?;
    let mut ms: Vec<&str> = Vec::new();
    for m in mods {
        let m = match m.as_str() {
            "control" | "ctl" => "ctrl",
            "option" => "alt",
            "cmd" | "super" | "win" => "meta",
            other => other,
        };
        if !MODS.contains(&m) || ms.contains(&m) {
            return None;
        }
        ms.push(m);
    }
    ms.sort_by_key(|m| MODS.iter().position(|x| x == m));
    let key = if key == "esc" { "escape".to_owned() } else { key.clone() };
    let ok = key.chars().count() == 1
        || NAMED.contains(&key.as_str())
        || (key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
    ok.then(|| ms.iter().map(|m| (*m).to_owned()).chain(std::iter::once(key)).collect::<Vec<_>>().join("+"))
}

/// User `keymap.toml` (next to `config.toml`) then the project's `.kor/keymap.toml`.
pub fn keymap_paths(root: &Path) -> Vec<PathBuf> {
    kor_config::user_dir()
        .map(|d| d.join("keymap.toml"))
        .into_iter()
        .chain(std::iter::once(root.join(".kor").join("keymap.toml")))
        .collect()
}

pub fn load_keymap(root: &Path) -> Keymap {
    load_keymap_from(&keymap_paths(root))
}

pub fn load_keymap_from(files: &[PathBuf]) -> Keymap {
    let mut km = Keymap::default();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        km.files.push(f.clone());
        let origin = f.display().to_string();
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                km.warnings.push(format!("{origin}: {e}"));
                continue;
            }
        };
        for (k, v) in &table {
            match (k.as_str(), v) {
                ("mode", toml::Value::String(m)) => match m.as_str() {
                    "default" => km.mode = Some(TuiKeys::Default),
                    "vim" | "vi" => km.mode = Some(TuiKeys::Vim),
                    "emacs" => km.mode = Some(TuiKeys::Emacs),
                    _ => km.warnings.push(format!("{origin}: mode `{m}` (default, vim, emacs)")),
                },
                ("tui" | "composer", toml::Value::Table(t)) => {
                    let (actions, builtin, map) = if k == "tui" {
                        (TUI_ACTIONS, TUI_BUILTIN, &mut km.tui)
                    } else {
                        (COMPOSER_ACTIONS, COMPOSER_BUILTIN, &mut km.composer)
                    };
                    for (key, action) in t {
                        let Some(action) = action.as_str() else {
                            km.warnings.push(format!("{origin}: [{k}] `{key}` must be an action name"));
                            continue;
                        };
                        let Some(nk) = normalize_key(key) else {
                            km.warnings.push(format!("{origin}: [{k}] `{key}` is not a key"));
                            continue;
                        };
                        if !actions.contains(&action) {
                            km.warnings.push(format!("{origin}: [{k}] unknown action `{action}` ({})", actions.join(", ")));
                            continue;
                        }
                        if let Some(prev) = map.get(&nk)
                            && prev != action
                        {
                            km.warnings.push(format!("{origin}: [{k}] `{nk}` was `{prev}`, now `{action}`"));
                        }
                        if let Some((_, b)) = builtin.iter().find(|(bk, _)| *bk == nk)
                            && *b != action
                        {
                            km.warnings.push(format!("{origin}: [{k}] `{nk}` replaces the built-in `{b}`"));
                        }
                        map.insert(nk, action.to_owned());
                    }
                }
                _ => km.warnings.push(format!("{origin}: unknown key `{k}` (mode, [tui], [composer])")),
            }
        }
    }
    km
}

// ------------------------------------------------------------------------------------------------
// Readability (§28.9)
// ------------------------------------------------------------------------------------------------

/// `ui.*` readability values as the desktop reads them (CSS variables are made from these).
pub fn ui_json(c: &kor_config::Config) -> Value {
    let u = &c.ui;
    let width = match u.max_width {
        kor_config::LineWidth::Narrow => "narrow",
        kor_config::LineWidth::Medium => "medium",
        kor_config::LineWidth::Wide => "wide",
        kor_config::LineWidth::Full => "full",
    };
    json!({
        "wrap": u.wrap,
        "wrapWidth": u.wrap_width,
        "maxWidth": width,
        "density": match u.density { kor_config::Density::Compact => "compact", kor_config::Density::Comfortable => "comfortable" },
        "fontSize": u.font_size.clamp(12, 24),
        "lineHeight": f64::from(u.line_height.clamp(110, 220)) / 100.0,
        "codeFont": u.code_font,
        "diffView": match u.diff_view { kor_config::DiffView::Inline => "inline", kor_config::DiffView::Split => "split" },
        "toolOutput": match u.tool_output { kor_config::Verbosity::Quiet => "quiet", kor_config::Verbosity::Normal => "normal", kor_config::Verbosity::Verbose => "verbose" },
        "screenReader": u.screen_reader,
        "highContrast": u.high_contrast,
        "reducedMotion": u.reduced_motion,
        "keys": match c.key_mode() { TuiKeys::Default => "default", TuiKeys::Vim => "vim", TuiKeys::Emacs => "emacs" },
    })
}

/// Readability keys the desktop may write (`prefs.setUi`), with the config key they map to.
pub const UI_KEYS: &[(&str, &str)] = &[
    ("wrap", "ui.wrap"),
    ("wrapWidth", "ui.wrap_width"),
    ("maxWidth", "ui.max_width"),
    ("density", "ui.density"),
    ("fontSize", "ui.font_size"),
    ("lineHeight", "ui.line_height"),
    ("codeFont", "ui.code_font"),
    ("diffView", "ui.diff_view"),
    ("toolOutput", "ui.tool_output"),
    ("screenReader", "ui.screen_reader"),
    ("highContrast", "ui.high_contrast"),
    ("reducedMotion", "ui.reduced_motion"),
    ("keys", "keys.mode"),
];

/// Wrap `text` at `width` columns (words kept whole when they fit), for the terminal.
pub fn wrap(text: &str, width: usize) -> String {
    if width < 10 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut col = 0;
        for word in line.split_inclusive(' ') {
            let n = word.chars().count();
            if col > 0 && col + n.saturating_sub(usize::from(word.ends_with(' '))) > width {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
                col = 0;
            }
            if n > width {
                // A word longer than the line is cut.
                for (j, c) in word.chars().enumerate() {
                    if j > 0 && (col + 1) > width {
                        out.push('\n');
                        col = 0;
                    }
                    out.push(c);
                    col += 1;
                }
            } else {
                out.push_str(word);
                col += n;
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Housekeeping (§26.1 `kor doctor --fix`)
// ------------------------------------------------------------------------------------------------

/// Something `kor doctor` found that it can fix safely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Issue {
    /// A folder Kor writes to does not exist.
    MissingDir { path: PathBuf },
    /// A lock file nobody holds (left by a crashed process).
    StaleLock { path: PathBuf },
    /// An interrupted atomic write (`*.tmp` older than an hour).
    StaleTemp { path: PathBuf },
}

impl Issue {
    pub fn path(&self) -> &Path {
        match self {
            Self::MissingDir { path } | Self::StaleLock { path } | Self::StaleTemp { path } => path,
        }
    }
}

/// Kor's own folders: user config, Kor home, sessions.
pub fn own_dirs() -> Vec<PathBuf> {
    [kor_config::user_dir(), crate::kor_home(), crate::sessions_dir()]
        .into_iter()
        .flatten()
        .collect()
}

/// Issues in `dirs` (missing ones, and stale locks / temp files directly inside them).
pub fn find_issues(dirs: &[PathBuf], max_tmp_age: std::time::Duration) -> Vec<Issue> {
    let mut out = Vec::new();
    for d in dirs {
        if !d.is_dir() {
            out.push(Issue::MissingDir { path: d.clone() });
            continue;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = Path::new(&name).extension().map(std::ffi::OsStr::to_ascii_lowercase);
            if ext.as_deref() == Some(std::ffi::OsStr::new("lock")) {
                // We can take it: no process holds it. (Release at once; the file is removed by `fix`.)
                if matches!(kor_platform::try_lock_file(&p), Ok(Some(_))) {
                    out.push(Issue::StaleLock { path: p });
                }
            } else if ext.as_deref() == Some(std::ffi::OsStr::new("tmp"))
                && e.metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > max_tmp_age)
            {
                out.push(Issue::StaleTemp { path: p });
            }
        }
    }
    out
}

/// Fix one issue (the caller asked the user first).
///
/// # Errors
/// The I/O error of the fix.
pub fn fix(issue: &Issue) -> std::io::Result<()> {
    match issue {
        Issue::MissingDir { path } => std::fs::create_dir_all(path),
        Issue::StaleLock { path } => {
            // Still nobody holds it (checked again right before removing).
            let free = kor_platform::try_lock_file(path)?.is_some();
            if free { std::fs::remove_file(path) } else { Ok(()) }
        }
        Issue::StaleTemp { path } => std::fs::remove_file(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_merges_validates_and_reports_conflicts() {
        let t = tempfile::tempdir().unwrap();
        let user = t.path().join("user.toml");
        let project = t.path().join("project.toml");
        std::fs::write(&user, "mode = \"emacs\"\n[tui]\n\"Ctrl+G\" = \"to_input\"\n\"ctrl+x\" = \"fly\"\n[composer]\n\"ctrl+j\" = \"submit\"\n").unwrap();
        std::fs::write(&project, "mode = \"vi\"\n[tui]\n\"control+g\" = \"quit\"\n\"ctrl+c\" = \"quit\"\n\"not-a-key+\" = \"up\"\n").unwrap();
        let km = load_keymap_from(&[user, project, t.path().join("missing.toml")]);
        assert_eq!(km.mode, Some(TuiKeys::Vim));
        assert_eq!(km.tui.get("ctrl+g").map(String::as_str), Some("quit"));
        assert_eq!(km.composer.get("ctrl+j").map(String::as_str), Some("submit"));
        assert_eq!(km.files.len(), 2);
        let w = km.warnings.join("\n");
        assert!(w.contains("unknown action `fly`"), "{w}");
        assert!(w.contains("`ctrl+g` was `to_input`, now `quit`"), "{w}");
        assert!(w.contains("replaces the built-in `interrupt`"), "{w}");
        assert!(w.contains("is not a key"), "{w}");
        assert_eq!(normalize_key("Shift+Ctrl+K").as_deref(), Some("ctrl+shift+k"));
        assert_eq!(normalize_key("F12").as_deref(), Some("f12"));
        assert_eq!(normalize_key("hyper+k"), None);
        assert_eq!(normalize_key("ctrl+ctrl+k"), None);
    }

    #[test]
    fn auto_rows_list_every_key_with_its_source() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("config.toml");
        std::fs::write(&f, "[auto]\ncompact = false\n").unwrap();
        let paths = kor_config::Paths { system: None, user: Some(f.clone()), project: None };
        let loaded = kor_config::load(&paths, std::iter::empty(), &[]).unwrap();
        let rows = auto_rows(&loaded);
        assert_eq!(rows.len(), AUTO_KEYS.len());
        let c = rows.iter().find(|r| r.key == "compact").unwrap();
        assert!(!c.value && !c.from_default && c.source.contains("config.toml"));
        let g = rows.iter().find(|r| r.key == "graph_watch").unwrap();
        assert!(g.value && g.from_default && g.related.iter().any(|(k, _)| k == "graph.watch"));
        let ui = ui_json(&loaded.config);
        assert_eq!(ui["maxWidth"], "medium");
        assert!(UI_KEYS.iter().all(|(k, _)| ui.get(*k).is_some()), "{ui}");
    }

    #[test]
    fn wrap_keeps_words_and_cuts_long_ones() {
        assert_eq!(wrap("aaaa bbbb cccc", 10), "aaaa bbbb\ncccc");
        assert_eq!(wrap("abcdefghijkl", 10), "abcdefghij\nkl");
        assert_eq!(wrap("short\nline", 40), "short\nline");
        assert_eq!(wrap("x y", 3), "x y", "below 10 columns nothing is wrapped");
    }

    #[test]
    fn housekeeping_finds_and_fixes_safely() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let missing = t.path().join("missing");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("audit.lock"), "").unwrap();
        std::fs::write(home.join("x.json.tmp"), "").unwrap();
        std::fs::write(home.join("keep.json"), "{}").unwrap();
        let held = kor_platform::lock_file(&home.join("busy.lock")).unwrap();
        let issues = find_issues(&[home.clone(), missing.clone()], std::time::Duration::ZERO);
        assert!(issues.contains(&Issue::MissingDir { path: missing.clone() }));
        assert!(issues.contains(&Issue::StaleLock { path: home.join("audit.lock") }));
        assert!(issues.iter().any(|i| matches!(i, Issue::StaleTemp { .. })));
        assert!(!issues.iter().any(|i| i.path().ends_with("busy.lock")), "a held lock is not stale");
        for i in &issues {
            fix(i).unwrap();
        }
        assert!(missing.is_dir() && !home.join("audit.lock").exists() && home.join("keep.json").exists());
        drop(held);
        let fresh = find_issues(&[home], std::time::Duration::from_secs(3600));
        assert!(fresh.iter().all(|i| !matches!(i, Issue::StaleTemp { .. })));
    }
}
