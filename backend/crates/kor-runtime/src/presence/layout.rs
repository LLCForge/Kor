//! Window positions per monitor configuration (§27.9): the main window reopens where it was last
//! closed on this set of monitors; a position on a monitor that is gone is never restored
//! off-screen. The status window (`PiP`) appears on the monitor under the cursor.
//! All coordinates are physical pixels (what the window system reports).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A restored window must show at least this much of itself on one monitor.
const MIN_VISIBLE: (i64, i64) = (120, 48);
const MAX_CONFIGS: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Stable key of a monitor set: names, positions and sizes, order-independent.
pub fn key(monitors: &[Monitor]) -> String {
    let mut parts: Vec<String> = monitors
        .iter()
        .map(|m| format!("{}@{},{},{}x{}", m.name, m.x, m.y, m.w, m.h))
        .collect();
    parts.sort();
    parts.join("|")
}

fn overlap(a: Rect, m: &Monitor) -> (i64, i64) {
    let (ax0, ay0) = (i64::from(a.x), i64::from(a.y));
    let (ax1, ay1) = (ax0 + i64::from(a.w), ay0 + i64::from(a.h));
    let (mx0, my0) = (i64::from(m.x), i64::from(m.y));
    let (mx1, my1) = (mx0 + i64::from(m.w), my0 + i64::from(m.h));
    (
        (ax1.min(mx1) - ax0.max(mx0)).max(0),
        (ay1.min(my1) - ay0.max(my0)).max(0),
    )
}

/// `saved` when enough of it is visible on one of `monitors`, its size clamped to that monitor.
pub fn restore(saved: Rect, monitors: &[Monitor]) -> Option<Rect> {
    let m = monitors.iter().max_by_key(|m| {
        let (w, h) = overlap(saved, m);
        w * h
    })?;
    let (w, h) = overlap(saved, m);
    if w < MIN_VISIBLE.0 || h < MIN_VISIBLE.1 {
        return None;
    }
    Some(Rect {
        w: saved.w.min(m.w),
        h: saved.h.min(m.h),
        ..saved
    })
}

/// Monitor containing the point, else the first.
pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|m| {
            let (x, y) = (i64::from(x), i64::from(y));
            x >= i64::from(m.x)
                && x < i64::from(m.x) + i64::from(m.w)
                && y >= i64::from(m.y)
                && y < i64::from(m.y) + i64::from(m.h)
        })
        .or_else(|| monitors.first())
}

/// Status window position: the remembered one when it lies on the cursor's monitor, else that
/// monitor's bottom-right corner with a margin.
pub fn pip_position(
    saved: Option<Rect>,
    monitors: &[Monitor],
    cursor: (i32, i32),
    size: (u32, u32),
) -> Option<(i32, i32)> {
    let m = monitor_at(monitors, cursor.0, cursor.1)?;
    if let Some(r) = saved
        && let Some(r) = restore(r, std::slice::from_ref(m))
    {
        return Some((r.x, r.y));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // margins of a few pixels
    let margin = (24.0 * m.scale.max(1.0)).round() as u32;
    let x = i64::from(m.x) + i64::from(m.w) - i64::from(size.0) - i64::from(margin);
    let y = i64::from(m.y) + i64::from(m.h) - i64::from(size.1) - i64::from(margin) * 3;
    Some((
        i32::try_from(x.max(i64::from(m.x))).unwrap_or(m.x),
        i32::try_from(y.max(i64::from(m.y))).unwrap_or(m.y),
    ))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    /// Monitor-set key → window label → rect; `order` keeps the most recent configs.
    configs: BTreeMap<String, BTreeMap<String, Rect>>,
    order: Vec<String>,
}

/// `<presence>/windows.json`.
#[derive(Debug, Clone)]
pub struct Layouts {
    path: PathBuf,
}

impl Layouts {
    pub fn standard() -> Option<Self> {
        super::dir().map(|d| Self::at(&d.join("windows.json")))
    }

    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    pub fn get(&self, monitors: &[Monitor], label: &str) -> Option<Rect> {
        let f: File = super::read_json(&self.path);
        f.configs.get(&key(monitors))?.get(label).copied()
    }

    /// # Errors
    /// When the file cannot be written.
    pub fn save(&self, monitors: &[Monitor], label: &str, r: Rect) -> Result<(), String> {
        let k = key(monitors);
        super::edit_json(&self.path, |f: &mut File| {
            f.configs
                .entry(k.clone())
                .or_default()
                .insert(label.to_owned(), r);
            f.order.retain(|x| x != &k);
            f.order.push(k);
            while f.order.len() > MAX_CONFIGS {
                let old = f.order.remove(0);
                f.configs.remove(&old);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, x: i32, w: u32) -> Monitor {
        Monitor {
            name: name.into(),
            x,
            y: 0,
            w,
            h: 1080,
            scale: 1.0,
        }
    }

    #[test]
    fn key_ignores_order() {
        let a = [mon("A", 0, 1920), mon("B", 1920, 2560)];
        let b = [mon("B", 1920, 2560), mon("A", 0, 1920)];
        assert_eq!(key(&a), key(&b));
        assert_ne!(key(&a), key(&a[..1]));
    }

    #[test]
    fn restore_on_a_missing_monitor_is_refused() {
        let two = [mon("A", 0, 1920), mon("B", 1920, 2560)];
        let on_b = Rect {
            x: 2200,
            y: 100,
            w: 1280,
            h: 820,
        };
        assert_eq!(restore(on_b, &two), Some(on_b));
        assert_eq!(restore(on_b, &two[..1]), None, "B unplugged");
        let barely = Rect { x: 1900, ..on_b };
        assert_eq!(restore(barely, &two[..1]), None, "only 20 px visible");
    }

    #[test]
    fn restore_clamps_size_to_the_monitor() {
        let small = [mon("A", 0, 1280)];
        let big = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert_eq!(
            restore(big, &small),
            Some(Rect {
                w: 1280,
                h: 1080,
                ..big
            })
        );
    }

    #[test]
    fn pip_goes_to_the_cursor_monitor() {
        let two = [mon("A", 0, 1920), mon("B", 1920, 2560)];
        let (x, y) = pip_position(None, &two, (2000, 500), (240, 72)).unwrap();
        assert_eq!((x, y), (1920 + 2560 - 240 - 24, 1080 - 72 - 72));
        let saved = Rect {
            x: 100,
            y: 100,
            w: 240,
            h: 72,
        };
        assert_eq!(
            pip_position(Some(saved), &two, (10, 10), (240, 72)),
            Some((100, 100))
        );
        assert_ne!(
            pip_position(Some(saved), &two, (2000, 10), (240, 72)),
            Some((100, 100)),
            "saved spot is on the other monitor"
        );
        assert!(pip_position(None, &[], (0, 0), (240, 72)).is_none());
    }

    #[test]
    fn layouts_round_trip_per_config() {
        let d = tempfile::tempdir().unwrap();
        let l = Layouts::at(&d.path().join("w.json"));
        let one = [mon("A", 0, 1920)];
        let two = [mon("A", 0, 1920), mon("B", 1920, 2560)];
        let r1 = Rect {
            x: 10,
            y: 10,
            w: 800,
            h: 600,
        };
        let r2 = Rect { x: 2000, ..r1 };
        l.save(&one, "main", r1).unwrap();
        l.save(&two, "main", r2).unwrap();
        assert_eq!(l.get(&one, "main"), Some(r1));
        assert_eq!(l.get(&two, "main"), Some(r2));
        assert_eq!(l.get(&two, "pip"), None);
    }
}
