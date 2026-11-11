//! `provider.*`: `kor provider test|status|add|remove|discover|keys|add-key|remove-key` için ince
//! köprü (parity.toml). İş mantığı `kor_providers::{probe, custom, health, pool}` içinde; burası
//! yalnızca JSON'a çevirir. Anahtarlar hiçbir zaman açık dönmez (son 4 hane maskeli).

use std::sync::Arc;

use kor_core::ProviderError;
use kor_providers::custom::{self, AuthMethod};
use kor_providers::{Provider, ProviderManifest, Registry, health, probe};
use kor_secrets::{
    ExposeSecret, KeychainStore, MAX_POOL_KEYS, SecretStore, SecretString, mask, pool_account,
};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `provider::call`'a yönlendirir).
pub const NS: &str = "provider";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    _state: &AppState,
) -> Option<Res> {
    let m = format!("{NS}.{name}");
    let m = m.as_str();
    Some(match name {
        "test" => match s(args, 0, m) {
            Ok(id) => test(id, opt_s(args, 1)).await,
            Err(e) => Err(e),
        },
        "status" => status(opt_s(args, 0)).await,
        // args: id, url, anthropic, noKey, proxy
        "add" => match (s(args, 0, m), s(args, 1, m)) {
            (Ok(id), Ok(url)) => add(id, url, b(args, 2), b(args, 3), b(args, 4)).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "remove" => s(args, 0, m).and_then(remove),
        "discover" => discover().await,
        "keys" => s(args, 0, m).and_then(keys),
        "addKey" => match (s(args, 0, m), s(args, 1, m)) {
            (Ok(id), Ok(key)) => add_key(id, key.trim()),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "removeKey" => match (s(args, 0, m), args.get(1).and_then(Value::as_u64)) {
            (Ok(id), Some(slot)) => remove_key(id, usize::try_from(slot).unwrap_or(0)),
            (Err(e), _) => Err(e),
            (_, None) => Err(format!("{m}: yuva numarası gerekli")),
        },
        _ => return None,
    })
}

fn registry() -> Result<Registry, String> {
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    kor_runtime::registry(&l, &mut Vec::new())
}

fn manifest(reg: &Registry, id: &str) -> Result<ProviderManifest, String> {
    reg.get(id)
        .cloned()
        .ok_or_else(|| format!("Bilinmeyen sağlayıcı: {id}"))
}

fn health_file() {
    if let Some(d) = kor_platform::cache_dir() {
        health::set_store(d.join("provider-health.json"));
    }
}

fn privacy_blocks(m: &ProviderManifest) -> Result<(), String> {
    if kor_runtime::privacy_mode() && !kor_runtime::is_local_provider(m) {
        return Err(format!("Gizlilik modu: {} yerel değil", m.id));
    }
    Ok(())
}

fn pooled(m: &ProviderManifest) -> Result<Arc<dyn Provider>, String> {
    let keys = kor_providers::pool::keys_for(m, &KeychainStore).map_err(|e| e.to_string())?;
    kor_providers::build_pool(m, keys).map_err(|e| e.message)
}

pub(crate) fn error_json(e: &ProviderError) -> Value {
    json!({
        "kind": serde_json::to_value(e.kind).unwrap_or(Value::Null),
        "status": e.http_status,
        "message": e.message,
    })
}

pub(crate) fn check_json(c: &probe::Check) -> Value {
    json!({
        "provider": c.provider,
        "ok": c.ok,
        "latencyMs": c.latency_ms,
        "models": c.models,
        "error": c.error.as_ref().map(error_json),
    })
}

async fn test(id: &str, model: Option<&str>) -> Res {
    health_file();
    let m = manifest(&registry()?, id)?;
    privacy_blocks(&m)?;
    let p = pooled(&m)?;
    let r = probe::test(p.as_ref(), model).await;
    Ok(json!({
        "check": check_json(&r.check),
        "model": r.model,
        "stream": r.stream.as_ref().map(|s| json!({
            "ok": s.ok,
            "firstTokenMs": s.first_token_ms,
            "totalMs": s.total_ms,
            "text": s.text,
            "error": s.error.as_ref().map(error_json),
        })),
    }))
}

async fn status(only: Option<&str>) -> Res {
    health_file();
    let reg = registry()?;
    let mut targets = Vec::new();
    for m in reg.iter().filter(|m| only.is_none_or(|id| id == m.id)) {
        if privacy_blocks(m).is_err() || (only.is_none() && !m.needs_key()) {
            continue;
        }
        let keys = kor_providers::pool::keys_for(m, &KeychainStore).map_err(|e| e.to_string())?;
        if m.needs_key() && keys.is_empty() {
            continue;
        }
        if let Ok(p) = kor_providers::build_pool(m, keys) {
            targets.push(p);
        }
    }
    futures::future::join_all(targets.iter().map(|p| probe::check(p.as_ref()))).await;
    let rows: Vec<Value> = health::snapshot()
        .into_iter()
        .filter(|(id, _)| only.is_none_or(|o| id == o || id.starts_with(&format!("{o}#"))))
        .map(|(id, st)| {
            json!({
                "provider": id,
                "latencyMs": st.latency_ms(),
                "errorRate": st.error_rate(),
                "calls": st.recent.len(),
                "remainingRequests": st.quota.remaining_requests,
                "remainingTokens": st.quota.remaining_tokens,
                "limitRequests": st.quota.limit_requests,
                "limitTokens": st.quota.limit_tokens,
                "reset": st.quota.reset,
                "lastError": st.last_error,
                "updatedAt": st.updated_at,
            })
        })
        .collect();
    Ok(Value::Array(rows))
}

fn user_dir() -> Result<std::path::PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("providers"))
        .ok_or_else(|| "Yapılandırma klasörü bulunamadı".to_owned())
}

fn loopback(raw: &str) -> bool {
    tauri::Url::parse(raw).is_ok_and(|u| {
        matches!(u.host_str(), Some("localhost" | "[::1]"))
            || u.host_str().is_some_and(|h| h.starts_with("127."))
    })
}

async fn add(id: &str, url: &str, anthropic: bool, no_key: bool, proxy: bool) -> Res {
    if Registry::builtin().get(id).is_some() {
        return Err(format!(
            "“{id}” yerleşik bir sağlayıcı; başka bir kimlik seçin"
        ));
    }
    let proxy = proxy || id == "cliproxy";
    let method = if proxy {
        AuthMethod::SubscriptionProxy {
            endpoint: url.to_owned(),
        }
    } else if loopback(url) && no_key && !anthropic {
        AuthMethod::Local {
            base_url: url.to_owned(),
        }
    } else {
        AuthMethod::CustomEndpoint {
            base_url: url.to_owned(),
            anthropic,
            key: !no_key,
        }
    };
    let mf = custom::custom_manifest(id, &method).map_err(|e| e.message)?;
    let path = custom::save_user_manifest(&user_dir()?, &mf).map_err(|e| e.message)?;
    let mut out = json!({
        "provider": id,
        "path": path.display().to_string(),
        "keyEnv": mf.needs_key().then(|| mf.key_env()),
        "advanced": proxy,
    });
    if let AuthMethod::SubscriptionProxy { endpoint } = &method {
        out["panel"] = custom::proxy_panel(endpoint).into();
        if loopback(endpoint)
            && let Ok(p) = pooled(&mf)
        {
            out["check"] = check_json(&probe::check(p.as_ref()).await);
        }
    }
    Ok(out)
}

fn remove(id: &str) -> Res {
    custom::remove_user_manifest(&user_dir()?, id)
        .map(Value::Bool)
        .map_err(|e| e.message)
}

async fn discover() -> Res {
    let reg = registry()?;
    let found = probe::discover(&reg).await;
    Ok(found
        .into_iter()
        .map(|f| {
            json!({
                "provider": f.provider, "name": f.name, "baseUrl": f.base_url,
                "models": f.models, "moved": f.moved,
            })
        })
        .collect())
}

fn keyed(id: &str) -> Result<ProviderManifest, String> {
    let m = manifest(&registry()?, id)?;
    if m.needs_key() {
        Ok(m)
    } else {
        Err(format!("{id} anahtar kullanmıyor"))
    }
}

fn keys(id: &str) -> Res {
    let m = keyed(id)?;
    let mut out = Vec::new();
    for slot in 1..=MAX_POOL_KEYS {
        let acct = pool_account(&m.id, slot).map_err(|e| e.to_string())?;
        if let Some(k) = KeychainStore.get(&acct).map_err(|e| e.to_string())? {
            out.push(json!({ "slot": slot, "masked": mask(k.expose_secret()) }));
        }
    }
    Ok(Value::Array(out))
}

fn add_key(id: &str, key: &str) -> Res {
    let m = keyed(id)?;
    match kor_secrets::add_pool_key(&m.id, &SecretString::from(key.to_owned()), &KeychainStore) {
        Ok(Some(slot)) => Ok(json!({ "slot": slot })),
        Ok(None) => Err(format!("{MAX_POOL_KEYS} yuvanın hepsi dolu")),
        Err(e) => Err(e.to_string()),
    }
}

fn remove_key(id: &str, slot: usize) -> Res {
    let m = keyed(id)?;
    if !(1..=MAX_POOL_KEYS).contains(&slot) {
        return Err("Geçersiz yuva".into());
    }
    let acct = pool_account(&m.id, slot).map_err(|e| e.to_string())?;
    KeychainStore
        .delete(&acct)
        .map(Value::Bool)
        .map_err(|e| e.to_string())
}
