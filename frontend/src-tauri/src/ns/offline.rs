//! `offline.*`: `kor offline init|status|run|bundle|import` için ince köprü (parity.toml, ADR 0034).
//! İş mantığı `kor_runtime::offline` içinde. Uzun işler arka planda çalışır; ilerleme
//! `kor:offline/progress` (`offline::Event`), bitiş `kor:offline/done` `{ job, ok, result?, error? }`.
//!
//! - `offline.status()` → `Status` + `ollamaModels`
//! - `offline.plan({ runtime?, models?, defaultModel?, embedding?, catalog? })` → `Plan`
//! - `offline.init(opts)` → boyutla onay penceresi, sonra `{ started }`
//! - `offline.setEnabled(bool)` → çevrimdışı mod (`[offline] enabled`, gizliliği de açar);
//!   llama.cpp çalıştırıcısıyla `llama-server`'ı başlatır/durdurur → `status`
//! - `offline.bundle({ models?, platform?, embedding?, desktop? })` → kaydetme penceresi
//!   (`desktop: true` ise önce kurulum dosyası seçilir), sonra `{ started }`
//! - `offline.import()` → dosya seçimi, manifest özetiyle onay, sonra `{ started }`

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use kor_runtime::offline::{self, BundleOptions, Event, InitOptions, LocalServer};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, confirm, open_dialog, root, save_dialog};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `offline::call`'a yönlendirir).
pub const NS: &str = "offline";

/// Aynı anda tek uzun iş (kurulum, paket, içe aktarma).
static BUSY: AtomicBool = AtomicBool::new(false);
/// Çevrimdışı mod açıkken Kor'un başlattığı `llama-server`.
static SERVER: LazyLock<Mutex<Option<LocalServer>>> = LazyLock::new(Mutex::default);

fn server() -> std::sync::MutexGuard<'static, Option<LocalServer>> {
    SERVER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mb(bytes: u64) -> String {
    // Yalnızca gösterim.
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.1} MB", b / 1e6)
    }
}

struct Ctx {
    loaded: kor_config::Loaded,
    reg: kor_providers::Registry,
}

fn ctx(state: &AppState) -> Result<Ctx, String> {
    let root = root(state)?;
    let loaded = crate::bridge::load_config(&root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let reg = kor_runtime::registry(&l, &mut Vec::new())?;
    Ok(Ctx { loaded, reg })
}

fn user_config() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("config.toml"))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

async fn status(state: &AppState) -> Res {
    let c = ctx(state)?;
    let s = offline::status(&c.loaded.config.offline, &c.reg).await;
    let mut v = serde_json::to_value(s).map_err(|e| e.to_string())?;
    v["ollamaModels"] = json!(offline::OLLAMA_MODELS);
    v["serving"] = json!(server().as_ref().map(|s| s.model.clone()));
    Ok(v)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .take(32)
        .collect()
}

fn init_options(o: &Value) -> InitOptions {
    InitOptions {
        runtime: o["runtime"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        models: strings(&o["models"]),
        default_model: o["defaultModel"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        embedding: o["embedding"].as_bool().unwrap_or(true),
        catalog: o["catalog"].as_bool().unwrap_or(true),
    }
}

/// `job` arka planda; `BUSY` serbest kalınca `kor:offline/done` yayılır.
fn spawn<F>(app: &AppHandle, job: &'static str, work: F) -> Res
where
    F: FnOnce(AppHandle) -> Result<Value, String> + Send + 'static,
{
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Başka bir çevrimdışı işlem sürüyor".into());
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = work(app.clone());
        BUSY.store(false, Ordering::SeqCst);
        let done = match r {
            Ok(v) => json!({ "job": job, "ok": true, "result": v }),
            Err(e) => json!({ "job": job, "ok": false, "error": e }),
        };
        let _ = app.emit("kor:offline/done", done);
    });
    Ok(json!({ "started": true }))
}

fn emitter(app: &AppHandle) -> impl Fn(Event) + Send + Sync + '_ {
    move |e: Event| {
        let _ = app.emit("kor:offline/progress", &e);
    }
}

async fn init(o: &Value, app: &AppHandle, state: &AppState) -> Res {
    let c = ctx(state)?;
    let plan = offline::plan(&init_options(o), &c.loaded.config.offline, &c.reg).await?;
    if !plan.downloads.is_empty() {
        let list: Vec<String> = plan
            .downloads
            .iter()
            .map(|d| format!("• {} ({})", d.name, d.size.map_or_else(|| "?".into(), mb)))
            .collect();
        let msg = format!(
            "Kor şunları indirip doğrulayacak ve kendi klasörüne kuracak (toplam {}):\n\n{}",
            mb(plan.total()),
            list.join("\n")
        );
        if !confirm(app, "Çevrimdışı kurulum", &msg, "İndir ve kur").await {
            return Ok(json!({ "started": false }));
        }
    }
    let file = user_config()?;
    spawn(app, "init", move |app| {
        let emit = emitter(&app);
        tauri::async_runtime::block_on(offline::init(&plan, &file, &emit))
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
    })
}

async fn set_enabled(on: bool, state: &AppState) -> Res {
    let file = user_config()?;
    kor_config::set_scalar(&file, "offline.enabled", if on { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    // Ağ politikası yeni ayarla yeniden uygulanır (ADR 0035).
    let c = ctx(state)?;
    if on {
        let cfg = c.loaded.config.offline.clone();
        if !cfg.model.is_empty() {
            state.set_model(cfg.model.clone());
        }
        let running = server().is_some();
        if !running && let Some(s) = offline::serve(&cfg).await? {
            *server() = Some(s);
        }
    } else {
        server().take(); // durdurur
    }
    status(state).await
}

async fn bundle(o: &Value, app: &AppHandle, state: &AppState) -> Res {
    let c = ctx(state)?;
    let desktop = if o["desktop"].as_bool() == Some(true) {
        match open_dialog(app, None).await {
            Some(p) => Some(p),
            None => return Ok(json!({ "started": false })),
        }
    } else {
        None
    };
    let platform = o["platform"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let name = format!(
        "kor-offline-{}.zip",
        platform.as_deref().unwrap_or(&kor_components::target())
    );
    let Some(out) = save_dialog(app, None, &name).await else {
        return Ok(json!({ "started": false }));
    };
    let opts = BundleOptions {
        out,
        platform,
        models: strings(&o["models"]),
        cli: None,
        desktop,
        embedding: o["embedding"].as_bool().unwrap_or(true),
    };
    let cfg = c.loaded.config.offline;
    spawn(app, "bundle", move |app| {
        let emit = emitter(&app);
        tauri::async_runtime::block_on(offline::create_bundle(&opts, &cfg, &emit)).map(|m| {
            json!({
                "path": opts.out.to_string_lossy(),
                "files": m.files.len(),
                "size": m.total(),
                "platform": m.platform,
            })
        })
    })
}

async fn import(app: &AppHandle) -> Res {
    let Some(file) = open_dialog(app, None).await else {
        return Ok(json!({ "started": false }));
    };
    let m = offline::inspect(&file)?;
    let msg = format!(
        "{} için {} dosya ({}), Kor {}.\nHer dosyanın SHA-256 özeti doğrulanır ve yalnızca Kor'un kendi \
         klasörlerine kurulur; sistemde değişiklik yapılmaz.",
        m.platform,
        m.files.len(),
        mb(m.total()),
        m.kor_version
    );
    if !confirm(app, "Çevrimdışı paketi içe aktar", &msg, "İçe aktar").await {
        return Ok(json!({ "started": false }));
    }
    let user = user_config().ok();
    spawn(app, "import", move |app| {
        let emit = emitter(&app);
        offline::import(&file, user.as_deref(), &emit)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
    })
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let opts = args.first().cloned().unwrap_or(Value::Null);
    Some(match name {
        "status" => status(state).await,
        "plan" => match ctx(state) {
            Ok(c) => offline::plan(&init_options(&opts), &c.loaded.config.offline, &c.reg)
                .await
                .and_then(|p| serde_json::to_value(p).map_err(|e| e.to_string())),
            Err(e) => Err(e),
        },
        "init" => init(&opts, app, state).await,
        "setEnabled" => set_enabled(b(args, 0), state).await,
        "bundle" => bundle(&opts, app, state).await,
        "import" => import(app).await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_read_with_defaults() {
        let o = init_options(&json!({ "models": ["qwen3-4b", 3], "runtime": "" }));
        assert_eq!(o.models, ["qwen3-4b"]);
        assert!(o.runtime.is_none() && o.embedding && o.catalog);
        let o = init_options(&json!({ "embedding": false, "defaultModel": "local/x" }));
        assert!(!o.embedding);
        assert_eq!(o.default_model.as_deref(), Some("local/x"));
        assert_eq!(mb(2_500_000_000), "2.5 GB");
    }
}
