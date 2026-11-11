//! `diagram.*`: canlı diyagram köprüsü (§27.14, ADR 0063). İş mantığı `kor_runtime::diagram_sync`.
//!
//! - `diagram.list()` → `Change[]` (durum, sayımlar, ASCII özet, yeniden üretilmiş metin)
//! - `diagram.sync(force)` → `{ written, changes }`
//! - `diagram.add(file, kind, format, params)` → `{ written, changes }`

use std::collections::BTreeMap;

use kor_runtime::diagram_sync::{self as ds, Format, Kind};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

pub const NS: &str = "diagram";

fn plan(root: &std::path::Path) -> Result<(Vec<ds::Block>, Vec<ds::Change>), String> {
    let mut g = kor_graph::Graph::open(root).map_err(|e| e.to_string())?;
    g.update().map_err(|e| e.to_string())?;
    let (blocks, bad) = ds::scan(root);
    let mut changes = ds::plan_in(root, &g, &blocks);
    changes.extend(bad);
    Ok((blocks, changes))
}

fn sync(root: &std::path::Path, force: bool) -> Res {
    let (blocks, changes) = plan(root)?;
    let written = ds::apply(root, &blocks, &changes, force)?;
    let (_, after) = plan(root)?;
    Ok(
        json!({ "written": written, "changes": after, "summary": changes.iter().filter(|c| c.status == ds::Status::Stale).map(ds::summary_line).collect::<Vec<_>>() }),
    )
}

fn params(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .filter_map(|(k, v)| {
                    Some((
                        k.to_ascii_lowercase(),
                        v.as_str()?.replace(char::is_whitespace, ""),
                    ))
                })
                .filter(|(_, v)| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(name, "list" | "sync" | "add") {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let force = b(args, 0);
    let args = args.to_vec();
    // Graf güncellemesi diske dokunur: arayüz iş parçacığını tutmasın.
    let name = name.to_owned();
    Some(
        tokio::task::spawn_blocking(move || match name.as_str() {
            "list" => {
                plan(&root).and_then(|(_, c)| serde_json::to_value(c).map_err(|e| e.to_string()))
            }
            "sync" => sync(&root, force),
            _ => {
                let file = s(&args, 0, "diagram.add")?;
                let kind =
                    Kind::parse(s(&args, 1, "diagram.add")?).ok_or("Bilinmeyen diyagram türü")?;
                let format = Format::parse(s(&args, 2, "diagram.add")?)
                    .ok_or("Bilinmeyen diyagram biçimi")?;
                ds::add(&root, file, kind, format, &params(args.get(3)))?;
                sync(&root, false)
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
    fn params_are_sanitized() {
        let p = params(Some(
            &json!({ "depth": "2", "from": " main ", "bad key": "x", "scope": "" }),
        ));
        assert_eq!(p.get("depth").map(String::as_str), Some("2"));
        assert_eq!(p.get("from").map(String::as_str), Some("main"));
        assert_eq!(p.len(), 2);
    }
}
