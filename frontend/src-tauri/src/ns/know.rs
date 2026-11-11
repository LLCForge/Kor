//! `know.*`: kişisel bilgi tabanı ekleri (§27.15, ADR 0063). İş mantığı `kor_memory::digest`.
//!
//! - `know.search(query, here)` → `[{ hit, stale }]` (eskimiş notlar işaretli)
//! - `know.digest(week?)` → `{ week, markdown, path }` (haftalık özet; dosyaya da yazılır)
//! - `know.digests()` → `[{ week, path }]` (yazılmış özetler, yeniden eskiye)
//! - `know.stale(here)` → `Stale[]`

use kor_memory::digest;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "know";

fn home() -> Result<std::path::PathBuf, String> {
    kor_runtime::kor_home().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn here(state: &AppState) -> Option<String> {
    root(state).ok().map(|r| kor_memory::project_key(&r))
}

/// Pazartesi 00:00 UTC (`2026-W45`).
fn parse_week(w: &str) -> Option<i64> {
    let (y, n) = w.split_once("-W")?;
    let d =
        chrono::NaiveDate::from_isoywd_opt(y.parse().ok()?, n.parse().ok()?, chrono::Weekday::Mon)?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

fn digests(home: &std::path::Path) -> Value {
    let mut v: Vec<(String, String)> = std::fs::read_dir(digest::digest_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| {
            Some((
                p.file_stem()?.to_string_lossy().into_owned(),
                p.to_string_lossy().into_owned(),
            ))
        })
        .collect();
    v.sort();
    v.reverse();
    json!(
        v.into_iter()
            .map(|(week, path)| json!({ "week": week, "path": path }))
            .collect::<Vec<_>>()
    )
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(name, "search" | "digest" | "digests" | "stale") {
        return None;
    }
    let here = here(state);
    let args = args.to_vec();
    let name = name.to_owned();
    Some(
        tokio::task::spawn_blocking(move || -> Res {
            let home = home()?;
            if name == "digests" {
                return Ok(digests(&home));
            }
            let kb = kor_memory::open_knowledge(&home).map_err(|e| e.to_string())?;
            match name.as_str() {
                "search" => {
                    let q = s(&args, 0, "know.search")?;
                    let only = b(&args, 1).then_some(()).and(here.as_deref());
                    let hits = kb.recall(q, only, here.as_deref(), 20).map_err(|e| e.to_string())?;
                    let _ = digest::record_uses(&home, &hits.iter().map(|h| h.record.id).collect::<Vec<_>>());
                    Ok(json!(hits.iter().map(|h| json!({ "hit": h, "stale": digest::staleness(&h.record) })).collect::<Vec<_>>()))
                }
                "digest" => {
                    let from = match opt_s(&args, 0) {
                        Some(w) => parse_week(w).ok_or("Hafta 2026-W45 biçiminde olmalı")?,
                        None => digest::week_start(chrono::Utc::now().timestamp()) - 7 * 86_400,
                    };
                    let d = digest::digest(&kb, &home, from).map_err(|e| e.to_string())?;
                    let path = digest::write_week(&kb, &home, from).map_err(|e| e.to_string())?;
                    Ok(json!({ "week": d.week, "markdown": digest::markdown(&d), "path": path.to_string_lossy() }))
                }
                _ => {
                    let only = b(&args, 0).then_some(()).and(here.as_deref());
                    serde_json::to_value(digest::stale(&kb, only, 500).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
                }
            }
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_and_digest_listing() {
        let m = parse_week("2026-W45").unwrap();
        assert_eq!(digest::week_start(m + 100), m);
        assert!(parse_week("x").is_none());
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(digest::digest_dir(d.path())).unwrap();
        std::fs::write(digest::digest_dir(d.path()).join("2026-W44.md"), "").unwrap();
        std::fs::write(digest::digest_dir(d.path()).join("2026-W45.md"), "").unwrap();
        assert_eq!(digests(d.path())[0]["week"], "2026-W45");
    }
}
