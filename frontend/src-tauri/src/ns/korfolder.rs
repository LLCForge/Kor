//! `korfolder.*`: `~/Kor/` klasörü ve `kor mount` sürücüsü için ince köprü (§27.5, ADR 0052).
//! İş mantığı `kor_runtime::korfolder`; sürücü `kor_platform::mount`. Açıkken masaüstü çalıştığı sürece
//! bir izleyici görünümleri günceller ve `Görevler/` dosyalarını çalıştırır; her geçişte
//! `kor:korfolder/changed` (`watch::Pass`) yayılır.
//!
//! - `korfolder.status()` → `Status` + `mount: { url, at } | null`
//! - `korfolder.enable()` → klasörü oluşturur (arayüzdeki onaydan sonra), açık projeyi bağlar → `SyncReport`
//! - `korfolder.disable()` → yalnızca Kor'un oluşturduklarını kaldırır → `RemoveReport`
//! - `korfolder.sync()` → `Pass`
//! - `korfolder.open()` → klasörü dosya yöneticisinde açar
//! - `korfolder.mount(at?)` / `korfolder.unmount()` → `{ url, at } | null`

use std::sync::{Arc, LazyLock, Mutex};

use kor_runtime::korfolder::{self, Layout, dav, watch};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, opt_s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `korfolder::call`'a yönlendirir).
pub const NS: &str = "korfolder";

static WATCHER: LazyLock<Mutex<Option<watch::Watcher>>> = LazyLock::new(Mutex::default);
static MOUNT: LazyLock<Mutex<Option<(dav::Server, String)>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &'static LazyLock<Mutex<T>>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn layout() -> Result<Layout, String> {
    Layout::current().ok_or_else(|| "Ev klasörü bulunamadı".into())
}

fn lang(state: &AppState) -> kor_i18n::Lang {
    let root = crate::api::root(state)
        .ok()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    crate::bridge::load_config(&root).map_or(kor_i18n::Lang::Tr, |c| c.config.ui.language.resolve())
}

/// Açılışta: klasör açıksa izlemeye başlar (`ns::start`).
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = tauri::Manager::state::<AppState>(&app);
        let lang = lang(&state);
        watch_on(&app, lang);
    });
}

/// Tokio bağlamında çağrılır.
fn watch_on(app: &AppHandle, lang: kor_i18n::Lang) {
    let app = app.clone();
    if let Some(Ok(w)) = watch::start_if_enabled(
        lang,
        Arc::new(move |p: &watch::Pass| {
            let _ = app.emit("kor:korfolder/changed", p);
        }),
    ) {
        *lock(&WATCHER) = Some(w);
    }
}

fn mount_json() -> Value {
    lock(&MOUNT)
        .as_ref()
        .map_or(Value::Null, |(s, at)| json!({ "url": s.url, "at": at }))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "status" => status(),
        "enable" => enable(app, state).await,
        "disable" => disable(),
        "sync" => sync(state).await,
        "open" => open(app),
        "mount" => mount(opt_s(args, 0).map(str::to_owned)).await,
        "unmount" => unmount(),
        _ => return None,
    })
}

fn status() -> Res {
    let l = layout()?;
    let mut v = serde_json::to_value(korfolder::status(&l)).map_err(|e| e.to_string())?;
    v["mount"] = mount_json();
    Ok(v)
}

async fn enable(app: &AppHandle, state: &AppState) -> Res {
    let l = layout()?;
    let project = crate::api::root(state).ok();
    let report = tauri::async_runtime::spawn_blocking({
        let l = l.clone();
        move || korfolder::enable(&l, project.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{}: {e}", l.root.display()))?;
    watch_on(app, lang(state));
    serde_json::to_value(report).map_err(|e| e.to_string())
}

fn disable() -> Res {
    let l = layout()?;
    *lock(&WATCHER) = None;
    if let Some((server, at)) = lock(&MOUNT).take() {
        let _ = kor_platform::mount::unmount(&server.url, &at);
    }
    let r = korfolder::disable(&l).map_err(|e| e.to_string())?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

async fn sync(state: &AppState) -> Res {
    let l = layout()?;
    if !l.enabled() {
        return Err("~/Kor klasörü kapalı.".into());
    }
    if let Ok(root) = crate::api::root(state) {
        let _ = korfolder::add(&l, &root);
    }
    let runner = watch::AgentRunner { lang: lang(state) };
    let pass = watch::pass(&l, &runner).await;
    serde_json::to_value(pass).map_err(|e| e.to_string())
}

fn open(app: &AppHandle) -> Res {
    let l = layout()?;
    if !l.root.is_dir() {
        return Err("~/Kor klasörü yok.".into());
    }
    app.opener()
        .open_path(l.root.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

async fn mount(at: Option<String>) -> Res {
    let l = layout()?;
    if !l.enabled() {
        return Err("Önce ~/Kor klasörünü açın.".into());
    }
    if lock(&MOUNT).is_some() {
        return Ok(mount_json());
    }
    let server = dav::serve(l.root.clone())
        .await
        .map_err(|e| e.to_string())?;
    let at = at
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| match kor_platform::os() {
            kor_platform::Os::Windows => "*".into(),
            kor_platform::Os::Mac => l.root.with_file_name("KorDrive").display().to_string(),
            _ => String::new(),
        });
    let url = server.url.clone();
    let shown = tauri::async_runtime::spawn_blocking(move || kor_platform::mount::mount(&url, &at))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            format!(
                "Sürücü bağlanamadı ({e}). Gereken: {}. ~/Kor normal klasör olarak kullanılabilir.",
                kor_platform::mount::client_hint(kor_platform::os())
            )
        })?;
    *lock(&MOUNT) = Some((server, shown));
    Ok(mount_json())
}

fn unmount() -> Res {
    if let Some((server, at)) = lock(&MOUNT).take() {
        kor_platform::mount::unmount(&server.url, &at).map_err(|e| e.to_string())?;
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    #[test]
    fn methods_are_routed() {
        let src = include_str!("korfolder.rs");
        for m in [
            "status", "enable", "disable", "sync", "open", "mount", "unmount",
        ] {
            assert!(src.contains(&format!("\"{m}\" =>")), "{m}");
        }
    }
}
