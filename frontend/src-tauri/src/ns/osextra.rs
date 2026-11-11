//! `osextra.*`: işletim sistemine özgü entegrasyonlar (§27.10, ADR 0052). İş mantığı
//! `kor_platform::os_integration` içinde (CLI `kor os-integration` ile ortak); burası JSON'a çevirir.
//!
//! - `osextra.list()` → `{ os, items: [{ id, os, kind, via?, installed, available, reason? }], wsl: string[] }`.
//! - `osextra.install(id, handoffUrl?)` → öğe durumu. Arayüzdeki onay penceresi onaydır.
//! - `osextra.uninstall(id)` → `{ status, left? }` (`left`: kullanıcıya kalan adımın kodu).
//! - `osextra.portals()` → `[{ portal, granted, detail? }]` (Linux; tek ekrandan sırayla izin ister).
//! - `osextra.remoteClipboard()` → `bool`: pano iPhone/iPad'den mi geldi (macOS, öğe açıksa).
//! - `osextra.handoff(url|null, title?)` → `bool`: açık oturumu Handoff'a bildirir / bırakır.
//! - `osextra.spotlightSync()` → `{ indexed }`: oturumları Spotlight'a yazar (öğe kapalıysa siler).

use std::collections::BTreeMap;

use kor_platform::os_integration::{self as osx, Env, Error, Options, mac, portals};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `osextra::call`'a yönlendirir).
pub const NS: &str = "osextra";

/// Spotlight'a yazılan en fazla oturum.
const MAX_SPOTLIGHT: usize = 500;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    _state: &AppState,
) -> Option<Res> {
    Some(match name {
        "list" => env(app).map(|e| list(&e)),
        "install" => match s(args, 0, "osextra.install") {
            Ok(id) => install(app, id, opt_s(args, 1)),
            Err(e) => Err(e),
        },
        "uninstall" => match s(args, 0, "osextra.uninstall") {
            Ok(id) => uninstall(app, id),
            Err(e) => Err(e),
        },
        "portals" => request_portals(app).await,
        "remoteClipboard" => Ok(Value::Bool(remote_clipboard(app))),
        "handoff" => handoff(app, opt_s(args, 0), opt_s(args, 1)),
        "spotlightSync" => spotlight_sync(app.clone()).await,
        _ => return None,
    })
}

/// Paketle gelen `kor` (yoksa PATH'teki), uygulama kaynakları ve simge.
pub(crate) fn env(app: &AppHandle) -> Result<Env, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let kor = crate::kor_cli::path();
    let resources = app.path().resource_dir().ok();
    Env::current(kor, resources, Some(exe)).map_err(|e| format!("Ev klasörü bulunamadı: {e}"))
}

fn list(env: &Env) -> Value {
    let os = match env.os {
        kor_platform::Os::Windows => "windows",
        kor_platform::Os::Mac => "macos",
        kor_platform::Os::Linux => "linux",
        kor_platform::Os::Other => "other",
    };
    json!({ "os": os, "items": osx::status_all(env), "wsl": kor_platform::wsl::distros() })
}

/// Hata kodu (`reason`) arayüzde Türkçe metne çevrilir; yine de okunur bir ileti verilir.
fn error_text(e: &Error) -> String {
    match e {
        Error::Unknown(id) => format!("Bilinmeyen entegrasyon: {id}"),
        Error::Unavailable(r) => format!("Bu bilgisayarda kullanılamıyor ({r})"),
        Error::NotManaged(r) => format!("Buradan kurulmaz ({r})"),
        Error::Foreign(p) => format!("{} başka bir uygulamaya ait; dokunulmadı", p.display()),
        Error::Io(e) => e.to_string(),
    }
}

fn install(app: &AppHandle, id: &str, handoff_url: Option<&str>) -> Res {
    let mut env = env(app)?;
    let opts = Options {
        handoff_url: handoff_url.filter(|u| !u.trim().is_empty()).map(str::to_owned),
        texts: osx::shortcuts::Texts::default(),
    };
    let st = osx::install(&mut env, id, &opts).map_err(|e| error_text(&e))?;
    Ok(json!(st))
}

fn uninstall(app: &AppHandle, id: &str) -> Res {
    let mut env = env(app)?;
    let (st, left) = osx::uninstall(&mut env, id).map_err(|e| error_text(&e))?;
    crate::api::remember_off(&format!("integrations.os.{id}"));
    match id {
        "spotlight" => {
            let _ = mac::spotlight_clear();
        }
        "handoff" => {
            let _ = app.run_on_main_thread(mac::handoff_end);
        }
        _ => {}
    }
    Ok(json!({ "status": st, "left": left }))
}

async fn request_portals(app: &AppHandle) -> Res {
    // `Env` is not `Send`: build it after the portal await, failing early if it cannot be built.
    drop(env(app)?);
    let texts = portals::Texts {
        folder_title: "Kor: projelerinizin bulunduğu klasörü seçin".into(),
        notify_title: "Kor".into(),
        notify_body: "Bildirim izni verildi.".into(),
    };
    let out = portals::request_all(&texts).await.map_err(|e| format!("Portal izinleri istenemedi: {e}"))?;
    let mut env = env(app)?;
    osx::install(&mut env, "portals", &Options::default()).map_err(|e| error_text(&e))?;
    let tokens: BTreeMap<String, String> = out
        .iter()
        .filter(|o| o.granted)
        .filter_map(|o| Some((o.portal.to_owned(), o.detail.clone()?)))
        .collect();
    osx::set_data(&env, "portals", tokens).map_err(|e| e.to_string())?;
    Ok(json!(out))
}

fn enabled(app: &AppHandle, id: &str) -> Option<BTreeMap<String, String>> {
    env(app).ok().and_then(|e| osx::flag(&e, id))
}

fn remote_clipboard(app: &AppHandle) -> bool {
    enabled(app, "universal-clipboard").is_some() && mac::remote_clipboard()
}

fn handoff(app: &AppHandle, url: Option<&str>, title: Option<&str>) -> Res {
    let Some(settings) = enabled(app, "handoff") else {
        return Ok(Value::Bool(false));
    };
    let Some(url) = url.filter(|u| u.starts_with("kor://resume?id=") && u.len() <= 512).map(str::to_owned) else {
        let _ = app.run_on_main_thread(mac::handoff_end);
        return Ok(Value::Bool(true));
    };
    let title: String = title.unwrap_or("Kor").chars().take(200).collect();
    let web = settings.get("url").cloned();
    app.run_on_main_thread(move || {
        let _ = mac::handoff_begin(&url, &title, web.as_deref());
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::Bool(true))
}

/// Oturumları Spotlight'a yazar (öğe kapalıysa Kor'un kayıtlarını siler). Açılışta ve oturum
/// kaydedildikten sonra çağrılır.
/// Açılışta: Spotlight/Handoff sonuçlarının `kor://` bağlantısı olarak dönmesi için işleyici (macOS; başka
/// OS'te `Unsupported`, yok sayılır) ve oturum dizininin düzenli tazelenmesi (yeni kaydedilen oturumlar
/// en geç 5 dakikada aranabilir olur).
pub fn start(app: AppHandle) {
    let _ = mac::install_continue_handler();
    if kor_platform::os() != kor_platform::Os::Mac {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = spotlight_sync(app.clone()).await;
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        }
    });
}

pub async fn spotlight_sync(app: AppHandle) -> Res {
    let on = enabled(&app, "spotlight").is_some();
    tauri::async_runtime::spawn_blocking(move || {
        if !on {
            let _ = mac::spotlight_clear();
            return Ok(json!({ "indexed": 0 }));
        }
        let items = spotlight_items();
        let n = items.len();
        mac::spotlight_index(&items).map_err(|e| format!("Spotlight dizini yazılamadı: {e}"))?;
        Ok(json!({ "indexed": n }))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn spotlight_items() -> Vec<mac::SpotItem> {
    let Some(dir) = kor_runtime::sessions_dir() else {
        return Vec::new();
    };
    kor_agent::session::list(&dir)
        .into_iter()
        .take(MAX_SPOTLIGHT)
        .filter_map(|p| {
            let id = p.file_stem()?.to_string_lossy().into_owned();
            let msgs = kor_agent::session::load(&p).ok()?;
            let text: String = msgs
                .iter()
                .filter(|m| matches!(m.role, kor_core::Role::User | kor_core::Role::Assistant))
                .map(kor_core::Message::plain_text)
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(4000)
                .collect();
            let title = text.lines().find(|l| !l.trim().is_empty())?.chars().take(80).collect();
            Some(mac::SpotItem { url: format!("kor://resume?id={}", kor_runtime::osint::encode(&id)), title, text })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn errors_read_in_turkish() {
        use super::*;
        assert!(error_text(&Error::Unknown("x".into())).contains("Bilinmeyen"));
        assert!(error_text(&Error::Foreign(std::path::PathBuf::from("a"))).contains("dokunulmadı"));
        assert_eq!(NS, "osextra");
    }
}
