//! `proxy.*`: `kor proxy serve|status|key` için ince köprü (parity.toml). Sunucu masaüstü
//! sürecinin içinde çalışır; iş mantığı `kor_proxy` içinde, burası yalnızca JSON'a çevirir.
//! Anahtar yalnızca açıkça istendiğinde (`key(false, true)`) tam döner.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use kor_secrets::{ExposeSecret, KeychainStore};
use serde_json::{Value, json};
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::api::{Res, b, root};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `proxy::call`'a yönlendirir).
pub const NS: &str = "proxy";

static RUNNING: Mutex<Option<kor_proxy::Running>> = Mutex::const_new(None);

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
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
        _ => return None,
    })
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
        host.parse().map_err(|_| format!("Geçersiz adres: {host}"))?
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
    let access = kor_proxy::Access { key: key.clone(), no_auth };
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
        return Ok(json!({ "path": file, "exists": exists, "running": true, "config": r.gateway.config_json() }));
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
