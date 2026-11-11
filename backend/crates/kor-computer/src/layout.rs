//! Saved window arrangements (§27.7): `kor layout save|restore|list`. A layout records every
//! visible top-level window (app, title, rectangle, state) with the monitor it was on, relative to
//! that monitor. Restoring matches windows by app and title and maps each rectangle onto the same
//! monitor (by name) or, when that monitor is gone, onto the primary one — scaled to its size and
//! never off-screen. A layout can be linked to a project (`.kor/layout.json`) so opening the
//! project brings its arrangement back.
//!
//! This module is pure: windows come in and moves go out; `kor-tools` lists and moves the real
//! windows with the per-OS window tools (Win32 `SetWindowPos`, macOS Accessibility, X11 EWMH).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A monitor, physical pixels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

impl Monitor {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}

/// A live window as the window tools report it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    pub hwnd: i64,
    pub title: String,
    pub process: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// `normal`, `maximized`, `minimized`.
    pub state: String,
}

/// One window in a saved layout; the rectangle is relative to its monitor, in 1/10000ths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub process: String,
    pub title: String,
    pub monitor: String,
    pub rx: i32,
    pub ry: i32,
    pub rw: i32,
    pub rh: i32,
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    /// Unix seconds.
    pub saved_at: u64,
    pub monitors: Vec<Monitor>,
    pub windows: Vec<Saved>,
}

/// One window to move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub hwnd: i64,
    pub process: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub state: String,
}

/// What a restore will do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// Saved windows with no open match (`app — title`).
    pub missing: Vec<String>,
}

const SCALE: i64 = 10_000;

fn monitor_of<'a>(monitors: &'a [Monitor], w: &Window) -> Option<&'a Monitor> {
    let cx = w.x.saturating_add(w.w / 2);
    let cy = w.y.saturating_add(w.h / 2);
    monitors
        .iter()
        .find(|m| m.contains(cx, cy))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

fn rel(v: i32, origin: i32, size: u32) -> i32 {
    if size == 0 {
        return 0;
    }
    i32::try_from((i64::from(v) - i64::from(origin)) * SCALE / i64::from(size)).unwrap_or(0)
}

fn abs(r: i32, origin: i32, size: u32) -> i32 {
    i32::try_from(i64::from(origin) + i64::from(r) * i64::from(size) / SCALE).unwrap_or(origin)
}

fn worth_saving(w: &Window) -> bool {
    !w.title.trim().is_empty() && w.w >= 100 && w.h >= 50 && w.state != "minimized"
}

/// Capture `windows` on `monitors` as a layout.
pub fn capture(name: &str, monitors: &[Monitor], windows: &[Window], saved_at: u64) -> Layout {
    let windows = windows
        .iter()
        .filter(|w| worth_saving(w))
        .filter_map(|w| {
            let m = monitor_of(monitors, w)?;
            Some(Saved {
                process: kor_platform::desktop_probe::app_name(&w.process),
                title: w.title.clone(),
                monitor: m.name.clone(),
                rx: rel(w.x, m.x, m.width),
                ry: rel(w.y, m.y, m.height),
                rw: rel(w.x + w.w, m.x, m.width) - rel(w.x, m.x, m.width),
                rh: rel(w.y + w.h, m.y, m.height) - rel(w.y, m.y, m.height),
                state: if w.state == "maximized" { "maximized".into() } else { "normal".into() },
            })
        })
        .collect();
    Layout {
        name: name.to_owned(),
        saved_at,
        monitors: monitors.to_vec(),
        windows,
    }
}

/// Moves that bring `layout` back on today's `monitors` with today's `windows`.
pub fn plan(layout: &Layout, monitors: &[Monitor], windows: &[Window]) -> Plan {
    let mut used = vec![false; windows.len()];
    let mut out = Plan::default();
    let fallback = monitors.iter().find(|m| m.primary).or_else(|| monitors.first());
    for s in &layout.windows {
        let same_app = |w: &Window| kor_platform::desktop_probe::app_name(&w.process) == s.process;
        // Exact title first, then any unused window of the same app.
        let pick = windows
            .iter()
            .enumerate()
            .find(|(i, w)| !used[*i] && same_app(w) && w.title == s.title)
            .or_else(|| windows.iter().enumerate().find(|(i, w)| !used[*i] && same_app(w)));
        let Some((i, w)) = pick else {
            out.missing.push(format!("{} — {}", s.process, s.title));
            continue;
        };
        let Some(m) = monitors.iter().find(|m| m.name == s.monitor).or(fallback) else {
            out.missing.push(format!("{} — {}", s.process, s.title));
            continue;
        };
        used[i] = true;
        // Keep at least 100x50 and the whole window on the monitor.
        let width = i32::try_from(m.width).unwrap_or(i32::MAX);
        let height = i32::try_from(m.height).unwrap_or(i32::MAX);
        let ww = abs(s.rw, 0, m.width).clamp(100.min(width), width);
        let hh = abs(s.rh, 0, m.height).clamp(50.min(height), height);
        let x = abs(s.rx, m.x, m.width).clamp(m.x, m.x + width - ww);
        let y = abs(s.ry, m.y, m.height).clamp(m.y, m.y + height - hh);
        out.moves.push(Move {
            hwnd: w.hwnd,
            process: s.process.clone(),
            title: w.title.clone(),
            x,
            y,
            w: ww,
            h: hh,
            state: s.state.clone(),
        });
    }
    out
}

/// Layout names are file names: letters, digits, `-`, `_`, `.`, 1–64 characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Saved layouts in `dir` (`<kor home>/layouts`).
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn at(dir: &Path) -> Self {
        Self { dir: dir.to_owned() }
    }

    fn file(&self, name: &str) -> Result<PathBuf, String> {
        if !valid_name(name) {
            return Err(format!("invalid layout name `{name}` (letters, digits, - _ . only)"));
        }
        Ok(self.dir.join(format!("{name}.json")))
    }

    pub fn save(&self, l: &Layout) -> Result<PathBuf, String> {
        let f = self.file(&l.name)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let tmp = f.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(l).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &f).map_err(|e| e.to_string())?;
        Ok(f)
    }

    pub fn load(&self, name: &str) -> Result<Layout, String> {
        let f = self.file(name)?;
        let text = std::fs::read_to_string(&f).map_err(|_| format!("no layout named `{name}`"))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))
    }

    pub fn delete(&self, name: &str) -> Result<(), String> {
        std::fs::remove_file(self.file(name)?).map_err(|_| format!("no layout named `{name}`"))
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Layout> {
        let mut v: Vec<Layout> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
            .collect();
        v.sort_by(|a: &Layout, b| b.saved_at.cmp(&a.saved_at).then(a.name.cmp(&b.name)));
        v
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Link {
    layout: String,
}

fn link_file(root: &Path) -> PathBuf {
    root.join(".kor").join("layout.json")
}

/// Link layout `name` to the project at `root` (`.kor/layout.json`); `None` removes the link.
pub fn link(root: &Path, name: Option<&str>) -> Result<(), String> {
    let f = link_file(root);
    match name {
        Some(n) => {
            if !valid_name(n) {
                return Err(format!("invalid layout name `{n}`"));
            }
            std::fs::create_dir_all(root.join(".kor")).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&Link { layout: n.to_owned() }).map_err(|e| e.to_string())?;
            std::fs::write(f, text).map_err(|e| e.to_string())
        }
        None => match std::fs::remove_file(f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    }
}

/// The layout linked to the project at `root`.
pub fn linked(root: &Path) -> Option<String> {
    let l: Link = serde_json::from_str(&std::fs::read_to_string(link_file(root)).ok()?).ok()?;
    valid_name(&l.layout).then_some(l.layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, x: i32, w: u32, h: u32, primary: bool) -> Monitor {
        Monitor { name: name.into(), x, y: 0, width: w, height: h, primary }
    }

    fn win(hwnd: i64, process: &str, title: &str, x: i32, y: i32, w: i32, h: i32) -> Window {
        Window { hwnd, title: title.into(), process: process.into(), x, y, w, h, state: "normal".into() }
    }

    #[test]
    fn save_and_restore_across_monitors() {
        let mons = [mon("DISPLAY1", 0, 1920, 1080, true), mon("DISPLAY2", 1920, 2560, 1440, false)];
        let wins = [
            win(1, "Code.exe", "main.rs - kor", 0, 0, 960, 1080),
            win(2, "chrome.exe", "Docs", 1920, 0, 1280, 1440),
            win(3, "x.exe", "", 0, 0, 500, 500),
            win(4, "tiny.exe", "tiny", 0, 0, 20, 20),
        ];
        let l = capture("kodlama", &mons, &wins, 7);
        assert_eq!(l.windows.len(), 2, "untitled and tiny windows are skipped");
        assert_eq!(l.windows[1].monitor, "DISPLAY2");
        assert_eq!((l.windows[0].rw, l.windows[0].rh), (5000, 10000));

        // Same monitors, windows reopened elsewhere with new handles.
        let now = [win(11, "chrome.exe", "Docs", 5, 5, 300, 300), win(12, "code.exe", "other - kor", 9, 9, 300, 300)];
        let p = plan(&l, &mons, &now);
        assert!(p.missing.is_empty());
        let code = p.moves.iter().find(|m| m.hwnd == 12).unwrap();
        assert_eq!((code.x, code.y, code.w, code.h), (0, 0, 960, 1080));
        let chrome = p.moves.iter().find(|m| m.hwnd == 11).unwrap();
        assert_eq!((chrome.x, chrome.w, chrome.h), (1920, 1280, 1440));

        // The second monitor is gone: chrome lands on the primary, scaled, on screen.
        let p = plan(&l, &mons[..1], &now);
        let chrome = p.moves.iter().find(|m| m.hwnd == 11).unwrap();
        assert_eq!((chrome.x, chrome.y, chrome.w, chrome.h), (0, 0, 960, 1080));

        // An app that is not open is reported.
        let p = plan(&l, &mons, &now[..1]);
        assert_eq!(p.missing, ["code — main.rs - kor"]);
    }

    #[test]
    fn store_and_project_link() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(&d.path().join("layouts"));
        assert!(s.save(&Layout { name: "../x".into(), ..Layout::default() }).is_err());
        s.save(&Layout { name: "a".into(), saved_at: 1, ..Layout::default() }).unwrap();
        s.save(&Layout { name: "b".into(), saved_at: 2, ..Layout::default() }).unwrap();
        assert_eq!(s.list().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(s.load("a").unwrap().saved_at, 1);
        s.delete("a").unwrap();
        assert!(s.load("a").is_err());

        let root = d.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(linked(&root), None);
        link(&root, Some("b")).unwrap();
        assert_eq!(linked(&root).as_deref(), Some("b"));
        link(&root, None).unwrap();
        assert_eq!(linked(&root), None);
        assert!(!valid_name(".hidden") && valid_name("kodlama-2"));
    }
}
