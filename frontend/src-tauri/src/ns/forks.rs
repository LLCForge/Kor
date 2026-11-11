//! `fork.*`: konuşma dallandırma köprüsü (§27.3, ADR 0063). İş mantığı `kor_runtime::fork`.
//!
//! - `fork.messages(id)` → `[{ n, role, text }]` ("buradan dallan" seçicisi)
//! - `fork.create(id, at, model?, noWorktree?)` → `Fork` (`worktree` varsa arayüz o klasörü açar)
//! - `fork.tree(id?)` → `Node[]`
//! - `fork.compare(a, b)` → `Comparison`
//! - `fork.mergePlan(id)` → `MergePlan` (dosya bazında inceleme için before/after)
//! - `fork.mergeApply(id, [{ path, content|null }], note)` → `{ written, note }`
//! - `fork.remove(id, worktree)` → `null`

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, opt_s, s};
use crate::state::AppState;

pub const NS: &str = "fork";

fn paths() -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let sessions = kor_runtime::sessions_dir().ok_or("Veri klasörü bulunamadı")?;
    let store = kor_runtime::fork::store_path().ok_or("Veri klasörü bulunamadı")?;
    Ok((sessions, store))
}

fn id_arg<'a>(args: &'a [Value], i: usize, method: &str) -> Result<&'a str, String> {
    let id = s(args, i, method)?;
    if kor_runtime::session_tree::valid_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz oturum kimliği".into())
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn messages(args: &[Value]) -> Res {
    let (sessions, _) = paths()?;
    let id = id_arg(args, 0, "fork.messages")?;
    let msgs = kor_agent::session::load(&sessions.join(format!("{id}.jsonl")))
        .map_err(|e| format!("Oturum okunamadı: {e}"))?;
    Ok(json!(
        msgs.iter()
            .enumerate()
            .map(|(i, m)| {
                let text: String = m.plain_text().chars().take(200).collect();
                json!({ "n": i + 1, "role": format!("{:?}", m.role).to_lowercase(), "text": text })
            })
            .collect::<Vec<_>>()
    ))
}

/// Kabul edilen dosyalar: `[{ path, content }]`, `content: null` silme demek.
fn accepted(v: Option<&Value>) -> Result<Vec<(String, Option<String>)>, String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|f| {
                    let p = f["path"].as_str().ok_or("Dosya yolu eksik")?.to_owned();
                    Ok((p, f["content"].as_str().map(str::to_owned)))
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let r = match name {
        "messages" => messages(args),
        "tree" => paths().and_then(|(sd, st)| {
            let meta = kor_runtime::session_tree::meta_path();
            kor_runtime::fork::tree(&sd, &st, meta.as_deref(), opt_s(args, 0))
                .map_err(|e| e.to_string())
                .and_then(|t| to_json(&t))
        }),
        "create" => (|| {
            let (sd, st) = paths()?;
            let id = id_arg(args, 0, "fork.create")?;
            let at = args
                .get(1)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("Mesaj numarası eksik")?;
            let root = crate::api::root(state).ok();
            let undo = kor_runtime::undo_dir();
            let opts = kor_runtime::fork::Options {
                model: opt_s(args, 2).map(str::to_owned),
                root: root.as_deref(),
                no_worktree: b(args, 3),
                undo_base: undo.as_deref(),
            };
            let f = kor_runtime::fork::fork(&sd, &st, id, at, &opts).map_err(|e| e.to_string())?;
            let _ = app.emit("kor:sessionsChanged", ());
            to_json(&f)
        })(),
        "compare" => (|| {
            let (sd, st) = paths()?;
            let c = kor_runtime::fork::compare(
                &sd,
                &st,
                id_arg(args, 0, "fork.compare")?,
                id_arg(args, 1, "fork.compare")?,
            )
            .map_err(|e| e.to_string())?;
            to_json(&c)
        })(),
        "mergePlan" => (|| {
            let (sd, st) = paths()?;
            to_json(
                &kor_runtime::fork::merge_plan(&sd, &st, id_arg(args, 0, "fork.mergePlan")?)
                    .map_err(|e| e.to_string())?,
            )
        })(),
        "mergeApply" => (|| {
            let (sd, st) = paths()?;
            let id = id_arg(args, 0, "fork.mergeApply")?;
            let files = accepted(args.get(1))?;
            let plan = kor_runtime::fork::merge_plan(&sd, &st, id).map_err(|e| e.to_string())?;
            // Yalnızca planda önerilen yollar yazılabilir.
            if let Some((p, _)) = files
                .iter()
                .find(|(p, _)| !plan.files.iter().any(|f| &f.path == p))
            {
                return Err(format!("`{p}` birleştirme planında yok"));
            }
            let note = b(args, 2).then_some(plan.summary.as_str());
            let undo = kor_runtime::undo_dir();
            let r = kor_runtime::fork::merge_apply(&sd, &st, undo.as_deref(), id, &files, note)
                .map_err(|e| e.to_string())?;
            let _ = app.emit("kor:sessionsChanged", ());
            to_json(&r)
        })(),
        "remove" => (|| {
            let (_, st) = paths()?;
            kor_runtime::fork::remove(&st, id_arg(args, 0, "fork.remove")?, b(args, 1))
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        })(),
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_files_and_ids() {
        let v = json!([{ "path": "a.rs", "content": "x" }, { "path": "b.rs", "content": null }]);
        assert_eq!(
            accepted(Some(&v)).unwrap(),
            vec![("a.rs".into(), Some("x".into())), ("b.rs".into(), None)]
        );
        assert!(accepted(Some(&json!([{ "content": "x" }]))).is_err());
        assert!(accepted(None).unwrap().is_empty());
        assert!(id_arg(&[json!("../x")], 0, "fork.tree").is_err());
        assert!(id_arg(&[json!("20260101-000000-1")], 0, "fork.tree").is_ok());
    }
}
