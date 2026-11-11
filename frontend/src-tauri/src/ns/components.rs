//! `components.*`: `kor component list|add|remove` (parity.toml). İş mantığı `kor-components`'ta;
//! indirme onayı ve ilerleme (`kor:runtimeProgress`) `runtimes::component` içinde.
//!
//! - `components.list()` → `[{ name, version, about, size, installed, path, sha256 }]`
//! - `components.add(name, from?)` → güncel liste; `from` verilirse internetsiz kurulum (aynı SHA-256)
//! - `components.remove(name)` → güncel liste
//! - `components.installAll()` / `components.update()` → toplam boyut için tek onay, sonra güncel liste
//! - `components.verify()` → `[{ name, state, detail }]`; bozuk kopya silinir, ilk kullanımda yeniden iner

use std::path::Path;

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

fn list() -> Value {
    let manifest = kor_components::Manifest::builtin();
    let target = kor_components::target();
    Value::Array(
        kor_components::list()
            .into_iter()
            .map(|c| {
                // Kurulum yalnızca bu özet eşleşince yerine konur; kuruluysa doğrulanmıştır.
                let sha256 = manifest
                    .get(&c.name)
                    .and_then(|m| m.artifact(&target))
                    .map(|a| a.sha256.clone());
                json!({
                    "sha256": sha256,
                    "name": c.name,
                    "version": c.version,
                    "about": c.about,
                    "size": c.size,
                    "installed": c.installed.is_some(),
                    "path": c.installed.map(|p| p.to_string_lossy().into_owned()),
                })
            })
            .collect(),
    )
}

fn known(name: &str) -> Result<&str, String> {
    // `browser` → `chromium` (spec §15.6: `kor component add browser`).
    let name = kor_components::canonical(name);
    if kor_components::info(name).is_some() {
        Ok(name)
    } else {
        Err(format!("Bilinmeyen bileşen: {name}"))
    }
}

/// `names` için toplam boyutla tek onay, sonra sırayla indir (`kor component install --all` / `update`).
async fn install_many(app: &AppHandle, names: Vec<String>, what: &str) -> Res {
    let all = kor_components::list();
    let todo: Vec<_> = all.into_iter().filter(|c| names.contains(&c.name) && c.size.is_some()).collect();
    if todo.is_empty() {
        return Ok(list());
    }
    let total: u64 = todo.iter().filter_map(|c| c.size).sum();
    let msg = format!(
        "{} bileşen ({}) indirilip Kor'un kendi klasörüne kurulsun mu? Her biri SHA-256 ile doğrulanır; yönetici izni gerekmez.",
        todo.len(),
        crate::runtimes::mb(total)
    );
    if !crate::api::confirm(app, what, &msg, "İndir").await {
        return Err("İndirilmedi".into());
    }
    let mut failed = Vec::new();
    for c in &todo {
        if let Err(e) = crate::runtimes::component(app, &c.name, false).await {
            failed.push(format!("{}: {e}", c.name));
        }
    }
    if failed.is_empty() { Ok(list()) } else { Err(failed.join("\n")) }
}

fn verify_all() -> Value {
    Value::Array(
        kor_components::list()
            .into_iter()
            .filter(|c| c.installed.is_some())
            .map(|c| {
                let (state, detail) = match kor_components::verify(&c.name) {
                    Ok(kor_components::Health::Ok) => ("ok", String::new()),
                    Ok(kor_components::Health::Missing) => ("missing", String::new()),
                    Ok(kor_components::Health::Outdated { installed }) => ("outdated", installed),
                    Ok(kor_components::Health::Unrecorded) => ("unrecorded", String::new()),
                    Ok(kor_components::Health::Broken(files)) => ("broken", files.join(", ")),
                    Err(e) => ("broken", e),
                };
                json!({ "name": c.name, "state": state, "detail": detail })
            })
            .collect(),
    )
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    _state: &AppState,
) -> Option<Res> {
    let m = format!("components.{name}");
    let m = m.as_str();
    Some(match name {
        "list" => Ok(list()),
        "add" => match s(args, 0, m).and_then(known) {
            Err(e) => Err(e),
            Ok(id) => match opt_s(args, 1).filter(|f| !f.is_empty()) {
                Some(file) => {
                    let (id, file) = (id.to_owned(), file.to_owned());
                    tauri::async_runtime::spawn_blocking(move || {
                        kor_components::add_from(&id, Path::new(&file))
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .map(|_| list())
                }
                // Kullanıcının açık isteği onaydır; yine de boyutu gösteren onay penceresi bir kez sorulur.
                None => crate::runtimes::component(app, id, true)
                    .await
                    .map(|_| list()),
            },
        },
        "installAll" => {
            let names = kor_components::list().into_iter().filter(|c| c.installed.is_none()).map(|c| c.name).collect();
            install_many(app, names, "Bütün bileşenleri kur").await
        }
        "update" => install_many(app, kor_components::outdated(), "Bileşenleri güncelle").await,
        "verify" => tauri::async_runtime::spawn_blocking(verify_all).await.map_err(|e| e.to_string()),
        "remove" => s(args, 0, m)
            .and_then(known)
            .and_then(|id| kor_components::remove(id).map_err(|e| format!("Silinemedi: {e}")))
            .map(|_| list()),
        _ => return None,
    })
}
