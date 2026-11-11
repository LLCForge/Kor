//! Masaüstü paketleme (ADR 0032).
//!
//! 1. İmzalı güncelleme: `kor update` ile aynı kaynak ve kanal (GitHub sürümleri, `update.channel`). Kanalın
//!    sürümü `kor_runtime::update::check` ile seçilir, o sürümün `latest.json` dosyası Tauri updater'a uç nokta
//!    olarak verilir; paket, derlemeye gömülü açık anahtarla doğrulanmadan kurulmaz. Açılışta ve günde bir kez
//!    denetlenir; gizlilik modunda ve CI'da ağ isteği yok. `update.auto` açıksa arka planda indirilir, kurulum
//!    her zaman kullanıcı onayıyla (yeniden başlat) olur. Anahtarsız derlemeler ve `.deb`/`.rpm` yalnızca bildirir.
//! 2. CLI: paketle gelen `kor` yalnızca kullanıcı onayıyla PATH'e eklenir / kaldırılır (`kor_platform::cli_path`).

use kor_platform::NoWindow as _;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::utils::config::BundleType;
use tauri::utils::platform::bundle_type;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

use crate::api::Res;
use crate::state::AppState;
use kor_runtime::update::Channel;

struct Status {
    phase: &'static str,
    available: Option<String>,
    release_url: Option<String>,
    /// Bulunan sürümün `latest.json` adresi (imzalı masaüstü paketleri).
    manifest: Option<String>,
    percent: Option<u64>,
    error: Option<String>,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    phase: "idle",
    available: None,
    release_url: None,
    manifest: None,
    percent: None,
    error: None,
});

/// İndirilmiş ve imzası doğrulanmış paket; kullanıcı "yeniden başlat ve kur" deyince kurulur.
static PENDING: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>> = Mutex::new(None);

fn with_status<T>(f: impl FnOnce(&mut Status) -> T) -> T {
    f(&mut STATUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// `update.channel`, `update.auto` ve gizlilik modu (klasör seçilmemişse ev dizinindeki ayarlar).
fn settings(state: &AppState) -> (Channel, bool, bool) {
    let root = crate::api::root(state)
        .ok()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let cfg = crate::bridge::load_config(&root).ok();
    let channel = match cfg.as_ref().map(|c| c.config.update.channel) {
        Some(kor_config::UpdateChannel::Beta) => Channel::Beta,
        Some(kor_config::UpdateChannel::Nightly) => Channel::Nightly,
        _ => Channel::Stable,
    };
    let auto = cfg.as_ref().is_none_or(|c| c.config.update.auto);
    let privacy = kor_runtime::privacy_mode() || cfg.is_some_and(|c| c.config.privacy.mode);
    (channel, auto, privacy)
}

/// Bu derlemenin güncelleme açık anahtarı (CI `plugins.updater.pubkey` ile gömer; yerel derlemede boş).
fn has_pubkey(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(Value::as_str)
        .is_some_and(|k| !k.trim().is_empty())
}

/// Yerinde kurulabilir mi: anahtar var ve paket türünü Tauri updater değiştirebiliyor.
/// `.deb`/`.rpm` paket yöneticisine aittir (§15.17: yalnızca bildirilir).
fn signed(app: &AppHandle) -> bool {
    has_pubkey(app)
        && matches!(
            bundle_type(),
            Some(BundleType::Nsis | BundleType::Msi | BundleType::App | BundleType::AppImage)
        )
}

fn distribution() -> &'static str {
    match bundle_type() {
        Some(BundleType::Nsis) => "nsis",
        Some(BundleType::Msi) => "msi",
        Some(BundleType::App | BundleType::Dmg) => "mac",
        Some(BundleType::AppImage) => "appimage",
        Some(BundleType::Deb) => "deb",
        Some(BundleType::Rpm) => "rpm",
        _ => "dev",
    }
}

/// `g16UpdateStatus` yanıtı (`ipc-g16.ts` UpdateStatus).
pub fn status(app: &AppHandle, state: &AppState) -> Value {
    let (channel, _, privacy) = settings(state);
    with_status(|s| {
        json!({
            "version": app.package_info().version.to_string(),
            "distribution": distribution(),
            "enabled": !privacy,
            "reason": privacy.then_some("Gizlilik modunda güncelleme denetimi yapılmaz."),
            "source": "kullanıcı",
            "host": "github.com",
            "channel": channel.as_str(),
            "signed": signed(app),
            "phase": s.phase,
            "available": s.available,
            "releaseUrl": s.release_url,
            "percent": s.percent,
            "error": s.error,
        })
    })
}

/// Kanaldaki yeni sürümü arar (`kor update check` ile aynı); bulunursa `kor:g16/updateAvailable` yayar.
pub async fn check(app: &AppHandle, state: &AppState) -> Value {
    let (channel, auto, privacy) = settings(state);
    let busy = with_status(|s| matches!(s.phase, "checking" | "downloading" | "downloaded"));
    if privacy || busy {
        return status(app, state);
    }
    with_status(|s| s.phase = "checking");
    let current = app.package_info().version.to_string();
    let found = kor_runtime::update::check(&current, channel).await.map(|c| c.available);
    let announce = with_status(|s| {
        s.error = None;
        s.percent = None;
        match found {
            Ok(Some(r)) => {
                let tag = r.tag_name.trim_start_matches('v').to_owned();
                let fresh = s.available.as_deref() != Some(tag.as_str());
                s.phase = "available";
                s.manifest = r
                    .assets
                    .iter()
                    .find(|a| a.name == "latest.json")
                    .map(|a| a.browser_download_url.clone());
                s.release_url = Some(r.html_url);
                s.available = Some(tag);
                fresh
            }
            Ok(None) => {
                s.phase = "not-available";
                s.available = None;
                false
            }
            Err(e) => {
                s.phase = "error";
                s.error = Some(e);
                false
            }
        }
    });
    if announce {
        let (version, manifest) = with_status(|s| (s.available.clone(), s.manifest.is_some()));
        let _ = app.emit(
            "kor:g16/updateAvailable",
            json!({ "version": version, "signed": signed(app) && manifest }),
        );
        // Bildirim merkezi + tepsi menüsündeki "Güncelle" (ADR 0051).
        if let Some(v) = &version {
            crate::presence::update_available(app, v);
        }
        if auto && signed(app) && manifest {
            start_download(app.clone(), channel);
        }
    }
    status(app, state)
}

/// İmzalı paketi arka planda indirir ve doğrular (kurmaz).
fn start_download(app: AppHandle, channel: Channel) {
    let Some(url) = with_status(|s| {
        if s.phase != "available" {
            return None;
        }
        let url = s.manifest.clone()?;
        s.phase = "downloading";
        s.percent = Some(0);
        Some(url)
    }) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let result = download(&app, &url, channel).await;
        with_status(|s| match result {
            Ok(Some(pending)) => {
                *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pending);
                s.phase = "downloaded";
                s.percent = Some(100);
            }
            Ok(None) => {
                s.phase = "not-available";
                s.percent = None;
            }
            Err(e) => {
                s.phase = "error";
                s.percent = None;
                s.error = Some(e);
            }
        });
    });
}

async fn download(
    app: &AppHandle,
    manifest: &str,
    channel: Channel,
) -> Result<Option<(tauri_plugin_updater::Update, Vec<u8>)>, String> {
    let url = manifest.parse().map_err(|_| "Geçersiz güncelleme adresi".to_owned())?;
    let mut builder = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(600));
    if channel == Channel::Nightly {
        // nightly etiketi sürüm numarası artmadan yenilenir: farklı olması yeterli.
        builder = builder.version_comparator(|current, remote| remote.version != current);
    }
    let Some(update) = builder
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let mut got = 0u64;
    let bytes = update
        .download(
            |chunk, total| {
                got += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    with_status(|s| s.percent = Some((got * 100 / total).min(99)));
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Güncelleme indirilemedi ya da imzası doğrulanamadı: {e}"))?;
    Ok(Some((update, bytes)))
}

/// Masaüstü arka plan denetimi aralığı (§27.4).
const DESKTOP_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Bulunan yeni sürüm (başlık çubuğu pill'i ve tepsi menüsü).
pub fn available_version() -> Option<String> {
    with_status(|s| s.available.clone())
}

/// "Kapanışta güncelle": indirilmiş ve doğrulanmış paket uygulama kapanırken kurulur (yeniden başlatmadan).
pub fn install_pending_on_exit() {
    let pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some((update, bytes)) = pending
        && let Err(e) = update.install(bytes)
    {
        eprintln!("kor: güncelleme kapanışta kurulamadı: {e}");
    }
}

/// `shellui.updateOnExit`: imzalı paket indirilmeye başlar; kurulum kapanışta.
pub async fn prepare_on_exit(app: &AppHandle, state: &AppState) -> Res {
    let (channel, _, privacy) = settings(state);
    if privacy {
        return Err("Gizlilik modunda güncelleme yapılmaz.".into());
    }
    if with_status(|s| s.phase != "available" && s.phase != "downloading" && s.phase != "downloaded") {
        check(app, state).await;
    }
    let manifest = with_status(|s| s.manifest.is_some());
    if !(signed(app) && manifest) {
        return Err("Bu kurulum yerinde güncellenemiyor; sürüm sayfasından indirin.".into());
    }
    start_download(app.clone(), channel);
    Ok(status(app, state))
}

/// `g16UpdateInstall`: indirilmiş paket varsa kurar ve yeniden başlatır; imzalı paket varsa indirmeyi başlatır;
/// yoksa (anahtarsız derleme, `.deb`/`.rpm`) sürüm sayfasını açar.
pub async fn install(app: &AppHandle, state: &AppState) -> Res {
    let pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some((update, bytes)) = pending {
        update
            .install(bytes)
            .map_err(|e| format!("Güncelleme kurulamadı: {e}"))?;
        app.restart();
    }
    let (channel, _, privacy) = settings(state);
    if privacy {
        return Err("Gizlilik modunda güncelleme yapılmaz.".into());
    }
    if with_status(|s| s.phase != "available") {
        check(app, state).await;
    }
    let (manifest, page) = with_status(|s| (s.manifest.is_some(), s.release_url.clone()));
    if signed(app) && manifest {
        start_download(app.clone(), channel);
        return Ok(status(app, state));
    }
    let page = page.ok_or("Yeni sürüm yok")?;
    app.opener()
        .open_url(page, None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(status(app, state))
}

/// Açılışta ve sonra günde bir kez denetler; kurulu (kopya) CLI'ı tazeler.
pub fn start(app: AppHandle) {
    if let Some(cli) = cli_source() {
        let _ = kor_platform::cli_path::refresh(&cli);
        // §24.4 (ADR 0064): a drag-installed .app or an AppImage has no installer step, so the Chrome
        // native host is registered (per user, idempotent) here; no extension ids, nothing written.
        let brand = kor_platform::brand();
        let _ = kor_platform::native_host::register(
            kor_platform::native_host::Scope::User,
            &brand.chrome.native_host,
            &brand.product.description,
            &cli,
            &brand.chrome.extension_ids,
        );
    }
    if std::env::var_os("CI").is_some() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            check(&app, &state).await;
            // §27.4: açılışta ve 6 saatte bir (CLI kendi günlük sınırını korur).
            tokio::time::sleep(DESKTOP_CHECK_INTERVAL).await;
        }
    });
}

// ---------------------------------------------------------------------------------------------
// CLI → PATH
// ---------------------------------------------------------------------------------------------

/// Paketle gelen `kor` (Tauri `externalBin`: ana programın yanında). Geliştirme derlemesinde yok.
fn cli_source() -> Option<std::path::PathBuf> {
    crate::kor_cli::bundled()
}

fn cli_json(c: Option<kor_platform::cli_path::CliPath>) -> Value {
    match c {
        None => json!({ "available": false, "installed": false, "removable": false, "byPackage": false, "location": null }),
        Some(c) => json!({
            "available": true,
            "installed": c.installed,
            "removable": c.removable,
            "byPackage": c.by_package,
            "location": c.location.map(|p| p.to_string_lossy().into_owned()),
        }),
    }
}

/// `g16CliStatus`.
pub fn cli_status() -> Value {
    cli_json(cli_source().map(|s| kor_platform::cli_path::status(&s)))
}

/// `g16CliInstall` (yalnızca arayüzdeki açık onaydan sonra çağrılır; sistem değişikliğidir).
pub fn cli_install() -> Res {
    let src = cli_source().ok_or("Bu kurulumda paketlenmiş kor CLI yok")?;
    let out = kor_platform::cli_path::install(&src)
        .map(|c| cli_json(Some(c)))
        .map_err(|e| format!("kor PATH'e eklenemedi: {e}"))?;
    // §26.1: aynı onayla bulunan kabuklara tamamlamalar da kurulur (ADR 0050); başarısızlık PATH'i bozmaz.
    let _ = std::process::Command::new(&src).no_window().args(["completions", "--install", "--yes"]).output();
    Ok(out)
}

/// `g16CliRemove`: yalnızca Kor'un yaptığı değişikliği geri alır.
pub fn cli_remove() -> Res {
    let src = cli_source().ok_or("Bu kurulumda paketlenmiş kor CLI yok")?;
    kor_platform::cli_path::remove(&src)
        .map(|c| cli_json(Some(c)))
        .map_err(|e| format!("kor PATH'ten kaldırılamadı: {e}"))
}
