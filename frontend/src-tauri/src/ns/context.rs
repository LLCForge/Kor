//! `context.*`: Context Budget Manager (§5.3) için ince köprü (`kor context` karşılığı). Dökümler
//! çekirdeğin yerel günlüğünden okunur (`kor_runtime::context_log_path`); rakamların hepsi tahmindir (≈).

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::Res;
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `context::call`'a yönlendirir).
pub const NS: &str = "context";

/// Grafik için en fazla bu kadar istek döner.
const HISTORY_MAX: usize = 2000;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let index = || {
        args.get(1)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| "Parça numarası gerekli".to_owned())
    };
    let id = || crate::api::s(args, 0, name).map(str::to_owned);
    Some(match name {
        // §28.1 döküm satırları: bağlamdan çıkar / sabitle (ADR 0050).
        "items" => match id() {
            Ok(id) => crate::chat::context_items(app, &id).await,
            Err(e) => Err(e),
        },
        "drop" => match (id(), index()) {
            (Ok(id), Ok(i)) => crate::chat::context_change(app, &id, i, None).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "pin" => match (id(), index()) {
            (Ok(id), Ok(i)) => {
                let on = args.get(2).and_then(Value::as_bool).unwrap_or(true);
                crate::chat::context_change(app, &id, i, Some(on)).await
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "last" => last(state),
        // args: limit? (varsayılan 200)
        "history" => Ok(history(args)),
        _ => return None,
    })
}

fn loaded(state: &AppState) -> Option<kor_config::Loaded> {
    let cwd = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    crate::bridge::load_config(&cwd).ok()
}

/// Son döküm, kullanıcı dilinde metni ve uyarısı, `[context]` eşikleri.
fn last(state: &AppState) -> Res {
    let loaded = loaded(state);
    let lang = loaded
        .as_ref()
        .map_or(kor_i18n::Lang::En, |l| l.config.ui.language.resolve());
    let l = kor_i18n::Localizer::new(lang);
    let cfg = loaded.map(|l| l.config.context).unwrap_or_default();
    let b = kor_runtime::context_log_path().and_then(|p| kor_agent::context::read_log(&p, 1).pop());
    Ok(json!({
        "breakdown": b,
        "total": b.as_ref().map(kor_agent::context::Breakdown::total),
        "text": b.as_ref().map(|b| kor_runtime::context::render(&l, b)),
        "warning": b.as_ref().and_then(|b| kor_runtime::context::warning(&l, b)),
        "thresholds": cfg,
    }))
}

/// Zaman içindeki kullanım: eskiden yeniye dökümler (`ts`, `window`, kategoriler).
fn history(args: &[Value]) -> Value {
    let limit = args
        .first()
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(200)
        .clamp(1, HISTORY_MAX);
    let items = kor_runtime::context_log_path()
        .map(|p| kor_agent::context::read_log(&p, limit))
        .unwrap_or_default();
    json!(items)
}
