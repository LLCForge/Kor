//! `system.*`, `computer.*`, `layout.*`: sistem profili ve sağlık uyarıları (§19.2–19.3), masaüstü
//! denetimi katmanları ve göstergesi (§19.1), pencere düzenleri (§27.7) için ince köprü (ADR 0059).
//! İş mantığı `kor_health`, `kor_computer`, `kor_tools::layouts`.
//!
//! Açılışta (`start`): profil çıkarılır ve `health.refresh_minutes`'te bir yenilenir (değişince
//! `kor:system/changed`), sağlık dakikada bir denetlenir (yeni uyarı bildirim merkezine eklenir ve
//! sağ altta kart olarak `kor:system/health` yayılır), masaüstü denetimi her eylemde
//! `kor:computer/activity` yayar, açılan projeye bağlı pencere düzeni bir kez geri getirilir
//! (`kor:layout/restored`).
//!
//! - `system.profile()` → `Profile` · `system.refresh()` → `{ profile, changed }`
//! - `system.health(deep)` → `Alert[]` · `system.cleanSuggest()` → `Candidate[]`
//! - `system.clean(ids)` → `Outcome[]` (arayüz önce onay alır) · `system.mute(kind)`
//! - `computer.status()` · `computer.setEnabled(on)` · `computer.setTier(app, tier)` ·
//!   `computer.resetTier(app)` · `computer.stop()`
//! - `layout.list()` · `layout.save(name, linkProject)` · `layout.restore(name)` ·
//!   `layout.delete(name)` · `layout.linked()`

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kor_computer::layout::{self as klayout, Store};
use kor_computer::policy::{self, Class, Policy, Tier};
use kor_health::clean::{self, Scan};
use kor_health::health::{self, Sampler};
use kor_health::system::{self, RealProbe};
use kor_runtime::presence::notifications::{Category, Notification};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, s};
use crate::state::AppState;

// `ns/mod.rs` routes `system`, `computer` and `layout` here.

fn config(state: &AppState) -> kor_config::Config {
    let root = crate::api::root(state)
        .ok()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    crate::bridge::load_config(&root)
        .map(|l| l.config)
        .unwrap_or_default()
}

fn home() -> Result<PathBuf, String> {
    kor_health::kor_home().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn user_config() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("config.toml"))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

fn layouts() -> Result<Store, String> {
    Ok(Store::at(&home()?.join("layouts")))
}

fn to_value<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

/// Bugünün numarası (uyarı kartı günde bir kez).
fn day() -> u64 {
    system::now_secs() / 86_400
}

/// Uyarıyı bildirim merkezine ekler ve sağ alt kartı yayar; bugün gösterildiyse hiçbir şey yapmaz.
fn raise(app: &AppHandle, shown: &Mutex<HashSet<String>>, a: &health::Alert) {
    let key = format!("health:{}:{}", a.id, day());
    if !shown
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key.clone())
    {
        return;
    }
    crate::presence::notify(
        app,
        Notification {
            category: Category::Health,
            title: a.title.clone(),
            body: a.body.clone(),
            key: Some(key),
            ..Notification::default()
        },
    );
    let _ = app.emit("kor:system/health", a);
}

/// Uygulama açılışında (`ns::start`).
pub fn start(app: AppHandle) {
    let emitter = app.clone();
    kor_computer::indicator::set_listener(Arc::new(
        move |a: &kor_computer::indicator::Activity| {
            let _ = emitter.emit("kor:computer/activity", a);
        },
    ));
    tauri::async_runtime::spawn(async move {
        let state = tauri::Manager::state::<AppState>(&app);
        let shown: Arc<Mutex<HashSet<String>>> = Arc::default();
        let sampler = Arc::new(Mutex::new(Sampler::new()));
        let mut last_root: Option<PathBuf> = None;
        let mut restored: HashSet<PathBuf> = HashSet::new();
        let mut tick: u64 = 0;
        loop {
            let cfg = config(&state);
            let lang = cfg.ui.language.resolve();
            let root = crate::api::root(&state).ok();
            // §19.2: açılışta ve periyodik olarak profil.
            if tick.is_multiple_of(u64::from(cfg.health.refresh_minutes.max(1)))
                && let Ok(h) = home()
                && let Ok(Ok(r)) = blocking(move || system::refresh(&h, &RealProbe)).await
                && !r.changed.is_empty()
            {
                let _ = app.emit("kor:system/changed", &r);
            }
            // §19.3: sağlık; yavaş ölçümler (önbellekler, Docker) on dakikada bir.
            let t = cfg.health.clone();
            let deep = tick.is_multiple_of(10);
            let s2 = sampler.clone();
            let r2 = root.clone();
            let l = kor_i18n::Localizer::new(lang);
            if let Ok(alerts) = blocking(move || {
                let mut sample = s2
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .sample(&t);
                if deep {
                    kor_health::fill_slow(&mut sample, r2.as_deref());
                }
                health::evaluate(&sample, &t, r2.as_deref(), &l)
            })
            .await
            {
                for a in &alerts {
                    raise(&app, &shown, a);
                }
            }
            // §27.7: açılan projeye bağlı düzen bir kez geri gelir.
            if root != last_root {
                last_root.clone_from(&root);
                if let Some(r) = root.clone()
                    && restored.insert(r.clone())
                    && let Some(name) = klayout::linked(&r)
                    && let Ok(store) = layouts()
                {
                    match kor_tools::layouts::restore(&store, &name, &r, false).await {
                        Ok(plan) => {
                            let _ = app
                                .emit("kor:layout/restored", json!({ "name": name, "plan": plan }));
                        }
                        Err(e) => eprintln!("kor: düzen geri getirilemedi ({name}): {e}"),
                    }
                }
            }
            tick += 1;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

fn computer_status(state: &AppState) -> Value {
    let cfg = config(state).computer;
    let p = Policy::from_config(&cfg);
    let front = kor_platform::desktop_probe::foreground_app();
    let classes: Vec<Value> = [
        Class::Kor,
        Class::Terminal,
        Class::PasswordManager,
        Class::Banking,
        Class::Browser,
        Class::Ide,
        Class::Other,
    ]
    .iter()
    .map(|c| json!({ "class": c, "tier": c.default_tier() }))
    .collect();
    json!({
        "enabled": cfg.enabled,
        "front": front.as_ref().map(|a| json!({ "app": a, "tier": p.tier(a).0, "class": policy::classify(a) })),
        "watching": kor_platform::desktop_probe::idle_ms().is_some(),
        "paused": kor_computer::paused(),
        "activity": kor_computer::indicator::current(),
        "apps": policy::rows(&p),
        "classes": classes,
    })
}

fn root_or_home(state: &AppState) -> PathBuf {
    crate::api::root(state)
        .ok()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
}

fn ids(args: &[Value]) -> Vec<String> {
    args.first()
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn scan_for(root: &Path) -> Scan {
    Scan::standard(health::cache_roots(Some(root)))
}

pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match (ns, name) {
        ("system", "profile") => match home() {
            Ok(h) => match system::load(&h) {
                Some(p) => to_value(&p),
                None => {
                    blocking(move || system::refresh(&h, &RealProbe).map_err(|e| e.to_string()))
                        .await
                        .and_then(|r| r)
                        .and_then(|r| to_value(&r.profile))
                }
            },
            Err(e) => Err(e),
        },
        ("system", "refresh") => match home() {
            Ok(h) => {
                let r =
                    blocking(move || system::refresh(&h, &RealProbe).map_err(|e| e.to_string()))
                        .await
                        .and_then(|r| r);
                if let Ok(r) = &r
                    && !r.changed.is_empty()
                {
                    let _ = app.emit("kor:system/changed", r);
                }
                r.and_then(|r| to_value(&r))
            }
            Err(e) => Err(e),
        },
        ("system", "health") => {
            let cfg = config(state);
            let root = crate::api::root(state).ok();
            let deep = b(args, 0);
            let l = kor_i18n::Localizer::new(cfg.ui.language.resolve());
            blocking(move || kor_health::check_now(&cfg.health, root.as_deref(), deep, &l))
                .await
                .and_then(|a| to_value(&a))
        }
        ("system", "cleanSuggest") => {
            let root = root_or_home(state);
            blocking(move || clean::suggest(&scan_for(&root)))
                .await
                .and_then(|v| to_value(&v))
        }
        ("system", "clean") => {
            let ids = ids(args);
            if ids.is_empty() {
                return Some(Err("Silinecek öğe seçilmedi".into()));
            }
            let root = root_or_home(state);
            blocking(move || clean::clean(&scan_for(&root), &ids))
                .await
                .and_then(|v| to_value(&v))
        }
        ("system", "mute") => {
            let kind = match s(args, 0, "system.mute") {
                Ok(k) => k.to_owned(),
                Err(e) => return Some(Err(e)),
            };
            let mut muted = config(state).health.muted;
            if !muted.contains(&kind) {
                muted.push(kind);
            }
            user_config()
                .and_then(|f| {
                    kor_config::set_string_list(&f, "health.muted", &muted)
                        .map_err(|e| e.to_string())
                })
                .map(|()| json!(muted))
        }
        ("system", "exportAll") => {
            // `kor export --all` ile aynı arşiv; İndirilenler klasörüne (yoksa ev klasörüne) yazılır.
            let Some(dir) = dirs::download_dir().or_else(dirs::home_dir) else {
                return Some(Err("İndirilenler klasörü bulunamadı".into()));
            };
            let out = dir.join(kor_runtime::export::default_name());
            blocking(move || kor_runtime::export::export_all(&out, env!("CARGO_PKG_VERSION")))
                .await
                .and_then(|r| r)
                .and_then(|v| to_value(&v))
        }
        ("computer", "status") => Ok(computer_status(state)),
        ("computer", "setEnabled") => user_config()
            .and_then(|f| {
                kor_config::set_scalar(
                    &f,
                    "computer.enabled",
                    if b(args, 0) { "true" } else { "false" },
                )
                .map_err(|e| e.to_string())
            })
            .map(|()| computer_status(state)),
        ("computer", "setTier") => {
            let (Ok(app_name), Ok(tier)) = (
                s(args, 0, "computer.setTier"),
                s(args, 1, "computer.setTier"),
            ) else {
                return Some(Err("Uygulama ve katman gerekli".into()));
            };
            let Some(t) = Tier::parse(tier) else {
                return Some(Err(format!("Bilinmeyen katman: {tier}")));
            };
            policy::config_key(app_name)
                .and_then(|key| {
                    user_config().and_then(|f| {
                        kor_config::set_string(&f, &key, t.key()).map_err(|e| e.to_string())
                    })
                })
                .map(|()| {
                    policy::set_policy(Policy::from_config(&config(state).computer));
                    computer_status(state)
                })
        }
        ("computer", "resetTier") => match s(args, 0, "computer.resetTier") {
            Ok(app_name) => policy::config_key(app_name)
                .and_then(|key| {
                    user_config()
                        .and_then(|f| kor_config::unset(&f, &key).map_err(|e| e.to_string()))
                })
                .map(|()| {
                    policy::set_policy(Policy::from_config(&config(state).computer));
                    computer_status(state)
                }),
            Err(e) => Err(e),
        },
        ("computer", "stop") => {
            kor_computer::interrupt();
            Ok(computer_status(state))
        }
        ("layout", "list") => {
            let linked = crate::api::root(state)
                .ok()
                .and_then(|r| klayout::linked(&r));
            layouts().map(|st| json!({ "layouts": st.list(), "linked": linked }))
        }
        ("layout", "linked") => Ok(json!(
            crate::api::root(state)
                .ok()
                .and_then(|r| klayout::linked(&r))
        )),
        ("layout", "save") => {
            let name = match s(args, 0, "layout.save") {
                Ok(n) => n.to_owned(),
                Err(e) => return Some(Err(e)),
            };
            let root = root_or_home(state);
            let project = (b(args, 1)).then(|| crate::api::root(state).ok()).flatten();
            match layouts() {
                Ok(st) => kor_tools::layouts::save(&st, &name, &root, project.as_deref())
                    .await
                    .and_then(|l| to_value(&l)),
                Err(e) => Err(e),
            }
        }
        ("layout", "restore") => {
            let name = match s(args, 0, "layout.restore") {
                Ok(n) => n.to_owned(),
                Err(e) => return Some(Err(e)),
            };
            let root = root_or_home(state);
            match layouts() {
                Ok(st) => kor_tools::layouts::restore(&st, &name, &root, false)
                    .await
                    .and_then(|p| to_value(&p)),
                Err(e) => Err(e),
            }
        }
        ("layout", "delete") => match (s(args, 0, "layout.delete"), layouts()) {
            (Ok(name), Ok(st)) => st.delete(name).map(|()| {
                if let Ok(r) = crate::api::root(state)
                    && klayout::linked(&r).as_deref() == Some(name)
                {
                    let _ = klayout::link(&r, None);
                }
                Value::Null
            }),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_read_from_the_first_argument() {
        assert_eq!(ids(&[json!(["a1", 3, "b2"])]), ["a1", "b2"]);
        assert!(ids(&[]).is_empty());
        assert!(day() > 19_000);
    }

    #[test]
    fn every_method_in_the_docs_is_routed() {
        // The module doc lists the contract; each name must have a match arm.
        let src = include_str!("system.rs");
        for m in [
            "profile",
            "refresh",
            "health",
            "cleanSuggest",
            "clean",
            "mute",
            "status",
            "setEnabled",
            "setTier",
            "resetTier",
            "stop",
            "list",
            "linked",
            "save",
            "restore",
            "delete",
        ] {
            assert!(src.contains(&format!(", \"{m}\")")), "{m}");
        }
    }
}
