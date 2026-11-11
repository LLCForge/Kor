//! `kor layout save|restore` on the real desktop (§27.7, ADR 0059): monitors from screen capture,
//! windows from the per-OS window scripts ([`WindowControl`]), the matching and placement math
//! from `kor_computer::layout`. Shared by the CLI and the desktop app.

use std::path::Path;

use kor_computer::layout::{self, Layout, Monitor, Plan, Store, Window};
use serde_json::Value;

use crate::WindowControl;

/// Monitors now (empty in builds without desktop control).
pub fn monitors() -> Vec<Monitor> {
    #[cfg(feature = "computer")]
    {
        kor_computer::live::displays()
            .into_iter()
            .map(|d| Monitor {
                name: d.name,
                x: d.x,
                y: d.y,
                width: d.width,
                height: d.height,
                primary: d.primary,
            })
            .collect()
    }
    #[cfg(not(feature = "computer"))]
    {
        Vec::new()
    }
}

fn window(v: &Value) -> Option<Window> {
    let i = |k: &str| v[k].as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(0);
    Some(Window {
        hwnd: v["hwnd"].as_i64()?,
        title: v["title"].as_str().unwrap_or_default().to_owned(),
        process: v["process"].as_str().unwrap_or_default().to_owned(),
        x: i("x"),
        y: i("y"),
        w: i("w"),
        h: i("h"),
        state: v["state"].as_str().unwrap_or("normal").to_owned(),
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Save the current arrangement as `name`; with `project`, link it to that project.
pub async fn save(store: &Store, name: &str, root: &Path, project: Option<&Path>) -> Result<Layout, String> {
    if !layout::valid_name(name) {
        return Err(format!("invalid layout name `{name}` (letters, digits, - _ . only)"));
    }
    let wc = WindowControl::new(root)?;
    let windows: Vec<Window> = wc.list().await?.iter().filter_map(window).collect();
    let mons = monitors();
    if mons.is_empty() {
        return Err("no monitor found (screen capture is unavailable)".into());
    }
    let l = layout::capture(name, &mons, &windows, now());
    store.save(&l)?;
    if let Some(p) = project {
        layout::link(p, Some(name))?;
    }
    Ok(l)
}

/// Bring layout `name` back. `dry_run` only plans. Windows that fail to move are reported in
/// `missing` with the reason.
pub async fn restore(store: &Store, name: &str, root: &Path, dry_run: bool) -> Result<Plan, String> {
    let l = store.load(name)?;
    let wc = WindowControl::new(root)?;
    let windows: Vec<Window> = wc.list().await?.iter().filter_map(window).collect();
    let mut plan = layout::plan(&l, &monitors(), &windows);
    if !dry_run {
        let mut failed = Vec::new();
        for m in &plan.moves {
            if let Err(e) = wc.place(m.hwnd, (m.x, m.y, m.w, m.h), &m.state).await {
                failed.push(format!("{} — {}: {e}", m.process, m.title));
            }
        }
        plan.missing.extend(failed);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn script_rows_become_windows() {
        let w = window(&json!({"hwnd": 42, "title": "a", "process": "code", "x": -8, "y": 0, "w": 900, "h": 700, "state": "maximized"})).unwrap();
        assert_eq!((w.hwnd, w.x, w.w, w.state.as_str()), (42, -8, 900, "maximized"));
        assert!(window(&json!({"title": "no handle"})).is_none());
    }
}
