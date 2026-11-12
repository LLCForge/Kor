//! `proxy.*`: `kor proxy serve|status|key` için ince köprü (parity.toml). Sunucu masaüstü
//! sürecinin içinde çalışır; iş mantığı `kor_proxy` içinde, burası yalnızca JSON'a çevirir.
//! Anahtar yalnızca açıkça istendiğinde (`key(false, true)`) tam döner.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use kor_proxy::oauth::{self, Prompt};
use kor_secrets::{ExposeSecret, KeychainStore};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `proxy::call`'a yönlendirir).
pub const NS: &str = "proxy";

static RUNNING: Mutex<Option<kor_proxy::Running>> = Mutex::const_new(None);

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        // args: port?, host?, noAuth
        "start" => start(args, state).await,
        "stop" => stop().await,
        "status" => status().await,
        "config" => config().await,
        // args: rotate, show
        "key" => key(b(args, 0), b(args, 1)),
        "oauthAccounts" => oauth_accounts(),
        "oauthLogin" => oauth_login(app, args).await,
        "oauthRefresh" => oauth_refresh(args).await,
        "oauthRemove" => oauth_remove(args).await,
        "oauthSetMeta" => oauth_set_meta(args).await,
        "routing" => routing_get(),
        "routingSet" => routing_set(args).await,
        _ => return None,
    })
}

fn proxy_settings() -> Result<kor_proxy::config::Settings, String> {
    let path = kor_proxy::default_config_path();
    match &path {
        Some(p) if p.exists() => kor_proxy::config::load(p),
        _ => Ok(kor_proxy::config::Settings::default()),
    }
}

fn auth_dir(st: &kor_proxy::config::Settings) -> PathBuf {
    st.auth_dir.clone().unwrap_or_else(oauth::default_auth_dir)
}

async fn reload_running_gateway() {
    let Ok(st) = proxy_settings() else {
        return;
    };
    let running = RUNNING.lock().await;
    if let Some(r) = running.as_ref().filter(|r| !r.is_finished()) {
        let _ = r.gateway.reload(st);
    }
}

fn prompt_payload(prompt: &Prompt) -> Value {
    match prompt {
        Prompt::Browser { url } => json!({ "kind": "browser", "url": url }),
        Prompt::Device {
            user_code,
            verification_uri,
            verification_uri_complete,
        } => {
            let link = verification_uri_complete
                .as_deref()
                .unwrap_or(verification_uri);
            json!({
                "kind": "device",
                "url": link,
                "userCode": user_code,
                "verificationUri": verification_uri,
                "verificationUriComplete": verification_uri_complete,
            })
        }
    }
}

fn oauth_accounts() -> Res {
    let st = proxy_settings()?;
    let dir = auth_dir(&st);
    let accounts = oauth::list_accounts(&dir);
    Ok(json!({
        "dir": dir.display().to_string(),
        "accounts": accounts,
    }))
}

async fn oauth_login(app: &AppHandle, args: &[Value]) -> Res {
    let provider_name = s(args, 0, "proxy.oauthLogin")?;
    let Some(provider) = oauth::Provider::parse(provider_name) else {
        return Err(format!(
            "Bilinmeyen sağlayıcı: `{provider_name}` (claude, codex, codex-device, antigravity, kimi, kimi-ai, devin, copilot)"
        ));
    };
    let st = proxy_settings()?;
    let dir = auth_dir(&st);
    let opts = oauth::options_from_settings(&st, false);
    let app = app.clone();
    let saved = oauth::login_with(provider, &opts, |prompt| {
        let payload = prompt_payload(prompt);
        let _ = app.emit("kor:proxy/oauth", payload.clone());
        if let Some(url) = payload.get("url").and_then(Value::as_str) {
            let _ = app.opener().open_url(url, None::<&str>);
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    reload_running_gateway().await;
    Ok(json!({
        "provider": saved.provider.to_string(),
        "label": saved.label,
        "fileName": saved.file_name,
        "path": saved.path.display().to_string(),
        "dir": dir.display().to_string(),
    }))
}

async fn oauth_refresh(args: &[Value]) -> Res {
    let file = s(args, 0, "proxy.oauthRefresh")?;
    let st = proxy_settings()?;
    let dir = auth_dir(&st);
    let path = dir.join(file);
    let opts = oauth::options_from_settings(&st, false);
    let saved = oauth::refresh_file(&path, &opts)
        .await
        .map_err(|e| e.to_string())?;
    reload_running_gateway().await;
    Ok(json!({
        "fileName": saved.file_name,
        "label": saved.label,
        "provider": saved.provider.to_string(),
    }))
}

async fn oauth_remove(args: &[Value]) -> Res {
    let file = s(args, 0, "proxy.oauthRemove")?;
    let st = proxy_settings()?;
    let dir = auth_dir(&st);
    oauth::remove_account(&dir, file).map_err(|e| e.to_string())?;
    reload_running_gateway().await;
    Ok(json!({ "removed": file, "dir": dir.display().to_string() }))
}

async fn oauth_set_meta(args: &[Value]) -> Res {
    let file = s(args, 0, "proxy.oauthSetMeta")?;
    let weight = args
        .get(1)
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok());
    let priority = args.get(2).and_then(Value::as_i64);
    let st = proxy_settings()?;
    let path = auth_dir(&st).join(file);
    oauth::set_account_meta(&path, weight, priority).map_err(|e| e.to_string())?;
    reload_running_gateway().await;
    Ok(json!(true))
}

fn routing_get() -> Res {
    let st = proxy_settings()?;
    Ok(json!({
        "strategy": st.strategy.yaml_name(),
        "requestRetry": st.request_retry,
    }))
}

async fn routing_set(args: &[Value]) -> Res {
    let strategy = s(args, 0, "proxy.routingSet")?;
    let request_retry = args
        .get(1)
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(3);
    let strat = kor_proxy::config::Strategy::parse_name(strategy)?;
    let path = kor_proxy::default_config_path().ok_or("Proxy yapılandırma dosyası yolu yok")?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let st = kor_proxy::config::patch_routing(&path, strat, request_retry)?;
    reload_running_gateway().await;
    Ok(json!({
        "strategy": st.strategy.yaml_name(),
        "requestRetry": st.request_retry,
    }))
}

fn privacy(state: &AppState) -> bool {
    kor_runtime::privacy_mode()
        || root(state)
            .ok()
            .and_then(|r| crate::bridge::load_config(&r).ok())
            .is_some_and(|l| l.config.privacy.mode)
}

async fn start(args: &[Value], state: &AppState) -> Res {
    let mut running = RUNNING.lock().await;
    if let Some(r) = running.as_ref()
        && !r.is_finished()
    {
        return Ok(json!({ "running": true, "url": format!("http://{}", r.addr) }));
    }
    let path = kor_proxy::default_config_path();
    let st = match &path {
        Some(p) => kor_proxy::config::load(p)?,
        None => kor_proxy::config::Settings::default(),
    };
    let port = args
        .first()
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(st.port);
    let host = args
        .get(1)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| st.host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.parse()
            .map_err(|_| format!("Geçersiz adres: {host}"))?
    };
    let no_auth = b(args, 2);
    let key = if no_auth {
        None
    } else {
        kor_proxy::access_key(&KeychainStore, true)?
    };
    let privacy = privacy(state);
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::Tr);
    let mut notes = Vec::new();
    let sources = kor_proxy::Sources::user(&l, privacy, &mut notes)?;
    let access = kor_proxy::Access {
        key: key.clone(),
        no_auth,
    };
    let gw = kor_proxy::Gateway::new(st, path, sources, access, &l);
    let warnings = gw.warnings.clone();
    let r = kor_proxy::start(Arc::new(gw), SocketAddr::new(ip, port)).await?;
    let url = format!("http://{}", r.addr);
    *running = Some(r);
    Ok(json!({
        "running": true,
        "url": url,
        "panel": format!("{url}/management.html"),
        "key": key.as_ref().map(kor_proxy::masked),
        "noAuth": no_auth,
        "privacy": privacy,
        "warnings": warnings,
        "notes": notes,
    }))
}

async fn stop() -> Res {
    match RUNNING.lock().await.take() {
        Some(r) => {
            r.stop().await;
            Ok(json!({ "running": false, "stopped": true }))
        }
        None => Ok(json!({ "running": false, "stopped": false })),
    }
}

async fn status() -> Res {
    let running = RUNNING.lock().await;
    match running.as_ref().filter(|r| !r.is_finished()) {
        Some(r) => {
            let mut v = r.gateway.usage_json();
            v["running"] = true.into();
            v["url"] = format!("http://{}", r.addr).into();
            Ok(v)
        }
        None => Ok(json!({ "running": false })),
    }
}

/// `proxy.config()`: ayar dosyasının yolu ve çalışan sunucunun (yoksa dosyanın) ayarları;
/// üst akış anahtarları maskeli (`Gateway::config_json`).
async fn config() -> Res {
    let path = kor_proxy::default_config_path();
    let file = path.as_ref().map(|p| p.display().to_string());
    let exists = path.as_ref().is_some_and(|p| p.exists());
    let running = RUNNING.lock().await;
    if let Some(r) = running.as_ref().filter(|r| !r.is_finished()) {
        return Ok(
            json!({ "path": file, "exists": exists, "running": true, "config": r.gateway.config_json() }),
        );
    }
    let st = match &path {
        Some(p) if exists => kor_proxy::config::load(p)?,
        _ => kor_proxy::config::Settings::default(),
    };
    Ok(json!({
        "path": file,
        "exists": exists,
        "running": false,
        "config": { "host": st.host, "port": st.port, "aliases": st.aliases, "ignored": st.ignored },
    }))
}

fn key(rotate: bool, show: bool) -> Res {
    let k = if rotate {
        Some(kor_proxy::rotate_key(&KeychainStore)?)
    } else {
        kor_proxy::access_key(&KeychainStore, false)?
    };
    Ok(match k {
        Some(k) => json!({
            "masked": kor_proxy::masked(&k),
            "rotated": rotate,
            // Kullanıcının kendi yerel ağ geçidi anahtarı; yalnızca açıkça istenince.
            "key": show.then(|| k.expose_secret().to_owned()),
        }),
        None => json!({ "masked": null }),
    })
}
