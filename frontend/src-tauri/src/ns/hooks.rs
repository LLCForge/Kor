//! `hooks.*`: görsel hook editörünün köprüsü (§28.12, ADR 0063). İş mantığı
//! `kor_runtime::hooks_edit`; Kor `hooks.toml` (kullanıcı/proje) ve Claude Code `settings.json`
//! (kullanıcı/proje/yerel) dosyaları değişmeden çalışır.
//!
//! - `hooks.events()` → `{ kor: string[], claude: string[] }` (olay seçicisi)
//! - `hooks.files()` → `HookFile[]`
//! - `hooks.list()` → `{ items: Item[], warnings: string[] }`
//! - `hooks.save(fileIndex, item, replace?)` → `null` (`replace = { event, id }` düzenler)
//! - `hooks.remove(fileIndex, event, id)` → `null`
//! - `hooks.test(fileIndex?, item, sample, execute)` → `DryRun`
//! - `hooks.log(limit)` → son çalışmalar

use kor_runtime::hooks_edit::{self as he, HookFile, Item, Sample};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root};
use crate::state::AppState;

pub const NS: &str = "hooks";

fn files(root: &std::path::Path) -> Vec<HookFile> {
    he::files(
        root,
        dirs::home_dir().as_deref(),
        kor_config::user_dir().as_deref(),
    )
}

fn file_at(files: &[HookFile], args: &[Value], i: usize) -> Result<HookFile, String> {
    args.get(i)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| files.get(n).cloned())
        .ok_or_else(|| "Hook dosyası seçilmedi".into())
}

fn item(v: Option<&Value>) -> Result<Item, String> {
    serde_json::from_value(v.cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("Hook tanımı okunamadı: {e}"))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(
        name,
        "events" | "files" | "list" | "save" | "remove" | "test" | "log"
    ) {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let fs = files(&root);
    Some(match name {
        "events" => Ok(json!({ "kor": kor_agent::hooks::KOR_EVENTS, "claude": he::CLAUDE_EVENTS })),
        "files" => serde_json::to_value(&fs).map_err(|e| e.to_string()),
        "list" => {
            let (items, warnings) = he::list(&fs);
            Ok(json!({ "items": items, "warnings": warnings }))
        }
        "save" => (|| {
            let f = file_at(&fs, args, 0)?;
            let it = item(args.get(1))?;
            let replace = args.get(2).filter(|v| !v.is_null());
            let rep = replace.map(|r| {
                (
                    r["event"].as_str().unwrap_or_default().to_owned(),
                    r["id"].as_str().unwrap_or_default().to_owned(),
                )
            });
            he::save(&f, &it, rep.as_ref().map(|(e, i)| (e.as_str(), i.as_str())))?;
            Ok(Value::Null)
        })(),
        "remove" => (|| {
            let f = file_at(&fs, args, 0)?;
            let event = args.get(1).and_then(Value::as_str).ok_or("Olay eksik")?;
            let id = args
                .get(2)
                .and_then(Value::as_str)
                .ok_or("Hook kimliği eksik")?;
            he::remove(&f, event, id)?;
            Ok(Value::Null)
        })(),
        "test" => match (
            item(args.get(1)),
            serde_json::from_value::<Sample>(args.get(2).cloned().unwrap_or(json!({}))),
        ) {
            (Ok(mut it), Ok(sample)) => {
                if let Ok(f) = file_at(&fs, args, 0) {
                    it.file = f.path.to_string_lossy().into_owned();
                    it.format = Some(f.format);
                }
                let d =
                    he::dry_run(&root, kor_tools::Shell::detect(), &it, &sample, b(args, 3)).await;
                serde_json::to_value(d).map_err(|e| e.to_string())
            }
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(format!("Örnek okunamadı: {e}")),
        },
        _ => Ok(json!(he::log(
            args.first()
                .and_then(Value::as_u64)
                .map_or(50, |n| usize::try_from(n).unwrap_or(50))
        ))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_index_and_item_shape() {
        let fs = files(std::path::Path::new("/p"));
        assert!(file_at(&fs, &[json!(1)], 0).is_ok());
        assert!(file_at(&fs, &[json!(99)], 0).is_err());
        let it = item(Some(
            &json!({ "event": "pre-edit", "command": "x", "files": ["src/**"] }),
        ))
        .unwrap();
        assert_eq!(it.files, vec!["src/**".to_owned()]);
        assert!(item(Some(&json!({ "event": 3 }))).is_err());
    }
}
