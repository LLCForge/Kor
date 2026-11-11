//! `catalog.*`: model zekâ/hız/fiyat verisi (`kor catalog`, §4.4) ve kancalar (`kor hooks list`,
//! §15.1). İş mantığı `kor_providers::model_catalog` ve `kor_agent::hooks` içinde; burası yalnızca
//! JSON'a çevirir.
//!
//! - `catalog.get(refresh?: bool, query?: string)` → `{ origin, fetchedAt, fromCache, notices, models }`.
//!   Artificial Analysis puanları yalnızca merkezi katalogdan gelir (CI anahtarı); uygulama AA'ya
//!   istek atmaz, kullanıcıdan AA anahtarı istenmez (ADR-0003).
//! - `catalog.hooks()` → `{ hooks: [{ event, korEvents, matcher, command, source }], warnings }`

use kor_providers::model_catalog::{self as mc, Cached, Endpoints, Options};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root};
use crate::state::AppState;

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "get" => get(state, b(args, 0), opt_s(args, 1)).await,
        "hooks" => hooks(state),
        _ => return None,
    })
}

fn privacy(state: &AppState) -> bool {
    root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .is_some_and(|c| c.config.privacy.mode)
}

/// `kor catalog` ile aynı kaynak ve önbellek. `api.rs` model seçicisi de bunu kullanabilir.
pub(crate) async fn data(
    state: &AppState,
    refresh: bool,
) -> Result<(Cached, bool, Vec<String>), String> {
    let cache = kor_platform::cache_dir().map(|d| d.join("model-catalog.json"));
    // `[auto] catalog_refresh = false`: a stale cache is used as is until refreshed by hand (§28.14).
    let no_auto = !refresh
        && crate::bridge::load_config(&root(state)?).is_ok_and(|c| !c.config.auto.catalog_refresh)
        && cache.as_deref().is_some_and(std::path::Path::is_file);
    let opts = Options {
        offline: privacy(state) || no_auto,
        refresh,
    };
    mc::load(cache.as_deref(), &Endpoints::default(), opts)
        .await
        .map(|l| (l.data, l.from_cache, l.notices))
        .map_err(|e| format!("Model kataloğu alınamadı: {e}"))
}

async fn get(state: &AppState, refresh: bool, query: Option<&str>) -> Res {
    let (d, from_cache, notices) = data(state, refresh).await?;
    let models: Vec<&mc::ModelData> = match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) if q.len() <= 200 => mc::search(&d.catalog.models, q),
        Some(_) => return Err("Arama metni çok uzun".into()),
        None => d.catalog.models.iter().collect(),
    };
    Ok(json!({
        "origin": d.origin,
        "fetchedAt": d.fetched_at,
        "fromCache": from_cache,
        "notices": notices,
        "models": models,
    }))
}

fn hooks(state: &AppState) -> Res {
    let c = kor_runtime::load_compat(&root(state)?);
    let hooks: Vec<Value> = c
        .hooks
        .iter()
        .map(|h| {
            json!({
                "event": h.event,
                "korEvents": kor_agent::hooks::kor_events(&h.event, None, &Value::Null),
                "matcher": h.matcher,
                "command": h.command,
                "source": { "eco": h.source.eco, "scope": h.source.scope, "path": h.source.path.to_string_lossy() },
            })
        })
        .collect();
    let warnings: Vec<&String> = c.warnings.iter().filter(|w| w.contains("hooks")).collect();
    Ok(json!({ "hooks": hooks, "warnings": warnings, "events": kor_agent::hooks::KOR_EVENTS }))
}
