//! `batch.*`: toplu "Kor ile düzenle" için ince köprü (§27.6, ADR 0052). İş mantığı
//! `kor_runtime::batch`: model işlem listesini önerir, Kor denetler, arayüz önizleme tablosunu gösterir,
//! kullanıcı onaylayınca `kor undo` günlüğüyle uygulanır.
//!
//! - `batch.plan(files: string[], instruction: string, model?: string)` → `{ id, rows, warnings }`
//! - `batch.apply(id)` → `{ applied, failed: [{ row, error }], undo }` (arayüzdeki onay düğmesi onaydır)
//! - `batch.cancel(id)` → `null`

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use kor_runtime::batch::{self, Plan, ProviderPlanner};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `batch::call`'a yönlendirir).
pub const NS: &str = "batch";

/// Onay bekleyen planlar (en çok birkaç tane; eskisi yenisiyle düşer).
static PLANS: LazyLock<Mutex<HashMap<String, Plan>>> = LazyLock::new(Mutex::default);
const MAX_PENDING: usize = 8;

fn plans() -> std::sync::MutexGuard<'static, HashMap<String, Plan>> {
    PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "plan" => plan(args, state).await,
        "apply" => match s(args, 0, "batch.apply") {
            Ok(id) => apply(id.to_owned()).await,
            Err(e) => Err(e),
        },
        "cancel" => match s(args, 0, "batch.cancel") {
            Ok(id) => {
                plans().remove(id);
                Ok(Value::Null)
            }
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

/// Dosya listesi: mutlak yollar, var olan dosyalar, en çok `batch::MAX_FILES`.
fn files(v: Option<&Value>) -> Result<Vec<PathBuf>, String> {
    let list: Vec<PathBuf> = v
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    if list.is_empty() || list.len() > batch::MAX_FILES {
        return Err(format!("1-{} dosya seçin.", batch::MAX_FILES));
    }
    if let Some(bad) = list.iter().find(|p| !p.is_absolute() || !p.is_file()) {
        return Err(format!("Dosya bulunamadı: {}", bad.display()));
    }
    Ok(list)
}

async fn plan(args: &[Value], state: &AppState) -> Res {
    let files = files(args.first())?;
    let instruction = s(args, 1, "batch.plan")?.to_owned();
    let model = opt_s(args, 2)
        .filter(|m| !m.trim().is_empty())
        .map(str::to_owned);
    let root = crate::api::root(state)
        .ok()
        .or_else(|| files[0].parent().map(PathBuf::from))
        .ok_or("Klasör yok")?;
    let loaded = crate::bridge::load_config(&root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    // Gizlilik modunda yerel olmayan sağlayıcıyı oneshot_provider reddeder.
    let (provider, model) = kor_runtime::oneshot_provider(&l, &loaded, model.as_deref())?;
    let planner = ProviderPlanner { provider, model };
    let p = batch::plan(&planner, &files, &instruction).await?;
    let id = crate::meta::new_uuid();
    let out = json!({ "id": id, "rows": p.rows, "warnings": p.warnings });
    let mut all = plans();
    if all.len() >= MAX_PENDING {
        all.clear();
    }
    all.insert(id, p);
    Ok(out)
}

async fn apply(id: String) -> Res {
    let p = plans()
        .remove(&id)
        .ok_or("Plan bulunamadı ya da zaten uygulandı.")?;
    let out = tauri::async_runtime::spawn_blocking(move || {
        batch::apply(&p, kor_runtime::undo_dir().as_deref())
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "applied": out.done.len(),
        "failed": out.failed.iter().map(|(row, error)| json!({ "row": row, "error": error })).collect::<Vec<_>>(),
        "undo": out.undo,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn file_lists_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, "").unwrap();
        assert_eq!(
            super::files(Some(&json!([f.to_string_lossy()]))).unwrap(),
            vec![f.clone()]
        );
        assert!(super::files(Some(&json!([]))).is_err());
        assert!(super::files(Some(&json!(["göreli.txt"]))).is_err());
        assert!(super::files(Some(&json!([d.path().join("yok").to_string_lossy()]))).is_err());
        assert!(super::files(None).is_err());
    }
}
