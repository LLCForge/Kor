//! `tools.*`: bilgisayardaki komut satırı araçları ve tipli araç tanımları için ince köprü
//! (§26.2, ADR 0056). Katalog, yardım özeti ve tanımlar `kor_agent::clitools` içinde.
//! Sözleşme: `frontend/src/desktop/ipc-intent.ts` (`ToolsApi`).
//!
//! - `tools.list(refresh?)` → `{ catalog, typed, errors, trusted }`
//! - `tools.describe(name, refresh?, model?)` → `Help` (yardım metni korumalı alanda, ağ kapalı)
//! - `tools.search(query)` → `{ hits: string[] }`
//! - `tools.add({ name, command, safety, description, user })` → `{ path }`
//! - `tools.share()` → `{ count }` (takım sunucusuna gönderim öncesi yerel onay)
//! - `tools.pull()` → `{ names, rejected }`

use std::path::Path;

use kor_agent::clitools::{Catalog, defs, help, search};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

pub const NS: &str = "tools";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "list" => list(&root, b(args, 0)).await,
        "describe" => describe(&root, args).await,
        "search" => search_tools(&root, args).await,
        "add" => add(&root, args),
        "share" => share(&root, app).await,
        "pull" => pull(&root).await,
        _ => return None,
    })
}

fn home() -> Result<std::path::PathBuf, String> {
    kor_runtime::clitools::tools_home().ok_or_else(|| "Kor klasörü bulunamadı".to_owned())
}

fn cfg(root: &Path) -> Result<kor_config::Loaded, String> {
    crate::bridge::load_config(root)
}

async fn list(root: &Path, refresh: bool) -> Res {
    let loaded = cfg(root)?;
    let mut cat = Catalog::load(&home()?, refresh).await;
    let t = loaded.config.tools.clone();
    cat.filter(|n| t.permits(n));
    let trusted = kor_runtime::is_trusted(root);
    let (typed, errors) = defs::load(&kor_runtime::clitools::dirs(root, trusted));
    let typed: Vec<Value> = typed
        .iter()
        .map(|x| json!({ "name": x.def.name, "origin": x.origin, "safety": x.def.safety, "command": x.def.command, "description": x.def.description }))
        .collect();
    Ok(json!({ "catalog": cat, "typed": typed, "errors": errors, "trusted": trusted }))
}

async fn describe(root: &Path, args: &[Value]) -> Res {
    let name = s(args, 0, "tools.describe")?;
    let loaded = cfg(root)?;
    if !loaded.config.tools.permits(name) {
        return Err(format!("{name}, [tools] ayarıyla izinli değil"));
    }
    let home = home()?;
    let ctx = kor_tools::ToolCtx::new(root).map_err(|e| e.to_string())?;
    let mut h = help::describe(&home.join("help"), name, b(args, 1), &ctx).await?;
    if b(args, 2) {
        let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
        let (p, m) = kor_runtime::oneshot_provider(&l, &loaded, None)?;
        h.model_summary = Some(help::model_summary(&p, &m, &h).await?);
        let _ = help::store(&home.join("help"), &h);
    }
    serde_json::to_value(&h).map_err(|e| e.to_string())
}

async fn search_tools(root: &Path, args: &[Value]) -> Res {
    let q = s(args, 0, "tools.search")?.trim();
    if q.is_empty() || q.len() > 500 {
        return Err("Arama 1-500 karakter olmalı".into());
    }
    let loaded = cfg(root)?;
    let mut cat = Catalog::load(&home()?, false).await;
    let t = loaded.config.tools.clone();
    cat.filter(|n| t.permits(n));
    let (typed, _) = defs::load(&kor_runtime::clitools::dirs(
        root,
        kor_runtime::is_trusted(root),
    ));
    let hits: Vec<String> = search::keyword_hits(&cat, &typed, q, kor_runtime::privacy_mode())
        .into_iter()
        .map(|h| h.line)
        .collect();
    Ok(json!({ "hits": hits }))
}

fn add(root: &Path, args: &[Value]) -> Res {
    let v = args.first().ok_or("`tools.add`: alanlar gerekli")?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let safety: defs::Safety = serde_json::from_value(Value::String(field("safety")))
        .map_err(|_| "Güvenlik sınıfı: read, write, network ya da destructive")?;
    let user = v.get("user").and_then(Value::as_bool) == Some(true);
    let file = kor_runtime::clitools::add(
        root,
        &field("name"),
        &field("command"),
        safety,
        &field("description"),
        user,
    )?;
    Ok(json!({ "path": file.display().to_string() }))
}

async fn share(root: &Path, app: &AppHandle) -> Res {
    let loaded = cfg(root)?;
    let server = loaded.config.team.server.trim().to_owned();
    // Ağa çıkan gönderim: onay arka uçta.
    if !crate::api::confirm(
        app,
        "Araç tanımları paylaşılsın mı?",
        &format!("Tanımlarınız {server} adresine gönderilecek."),
        "Gönder",
    )
    .await
    {
        return Err("Vazgeçildi".into());
    }
    let n =
        kor_runtime::clitools::share(&loaded.config, root, kor_runtime::is_trusted(root)).await?;
    Ok(json!({ "count": n }))
}

async fn pull(root: &Path) -> Res {
    let loaded = cfg(root)?;
    let (names, rejected) = kor_runtime::clitools::pull(&loaded.config).await?;
    Ok(json!({ "names": names, "rejected": rejected }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let bad = add(
            dir.path(),
            &[json!({"name": "X", "command": "git status", "safety": "write"})],
        );
        assert!(bad.is_err());
        let bad = add(
            dir.path(),
            &[json!({"name": "gs", "command": "git status", "safety": "root"})],
        );
        assert!(bad.is_err());
        let ok = add(
            dir.path(),
            &[json!({"name": "gs", "command": "git status", "safety": "read"})],
        )
        .unwrap();
        assert!(ok["path"].as_str().unwrap().ends_with("gs.toml"));
        assert!(
            add(
                dir.path(),
                &[json!({"name": "gs", "command": "git status", "safety": "read"})]
            )
            .is_err()
        );
    }
}
