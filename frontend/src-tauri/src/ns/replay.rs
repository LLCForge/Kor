//! `replay.*`: `kor replay list|<id>|export` için ince köprü (§15.9, ADR 0043). İş mantığı
//! `kor_runtime::replay` içinde; kayıtlar zaten maskelidir.
//!
//! - `replay.list()` → `Summary[]` (yeniden eskiye)
//! - `replay.load(id)` → `{ id, header, steps: Step[], events: Value[] }`
//! - `replay.export(id)` → kaydetme penceresi, sonra `{ path }` (iptalde `{ path: null }`)

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `replay::call`'a yönlendirir).
pub const NS: &str = "replay";

/// Masaüstü yalnızca projenin kendi kayıtlarını açar; dosya yolu kabul edilmez.
fn id_arg<'a>(args: &'a [Value], method: &str) -> Result<&'a str, String> {
    let id = s(args, 0, method)?;
    if kor_tools::record::valid_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz kayıt kimliği".into())
    }
}

fn load(args: &[Value], state: &AppState) -> Res {
    let id = id_arg(args, "replay.load")?;
    let r = kor_runtime::replay::load(&root(state)?, id)?;
    let steps = kor_runtime::replay::Timeline::new(&r.events).steps;
    Ok(json!({ "id": r.id, "header": r.header, "steps": steps, "events": r.events }))
}

async fn export(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let id = id_arg(args, "replay.export")?.to_owned();
    let root = root(state)?;
    let Some(path) =
        crate::api::save_dialog(app, Some(root.as_path()), &format!("kor-replay-{id}.json")).await
    else {
        return Ok(json!({ "path": null }));
    };
    let out = kor_runtime::replay::export(&root, &id, Some(&path))
        .map_err(|e| format!("Dışa aktarılamadı: {e}"))?;
    Ok(json!({ "path": out.to_string_lossy() }))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "list" => root(state).and_then(|r| {
            serde_json::to_value(kor_runtime::replay::list(&r)).map_err(|e| e.to_string())
        }),
        "load" => load(args, state),
        "export" => export(args, app, state).await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_recording_ids_are_accepted() {
        assert_eq!(
            id_arg(&[json!("20261110-1-2")], "replay.load"),
            Ok("20261110-1-2")
        );
        for bad in [json!("../x"), json!("C:\\x.json"), json!(3)] {
            assert!(id_arg(&[bad], "replay.load").is_err());
        }
    }
}
