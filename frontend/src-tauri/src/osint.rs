//! İşletim sisteminden Kor'a giriş (§25.1, §25.3, §27.6; ADR 0052): ilk açılışın ve tek örnek eklentisinin
//! ilettiği ikinci açılışın argümanları, `kor://` bağlantıları (macOS'ta Dock'a / "Birlikte aç"a bırakılan
//! klasörler de `file://` olarak buradan gelir), global kısayol ve görev çubuğu atlama listesi / Dock menüsü.
//!
//! Ayrıştırma `kor_runtime::osint`'te (CLI ile ortak). Burada: pencereyi öne getirmek, isteği
//! `kor:osint/request` olayıyla arayüze vermek (arayüz dinlemeye başlamadan gelenler `osint.pending` ile
//! alınır), çoklu seçimde Windows'un her dosya için ayrı başlattığı "Kor ile düzenle" çağrılarını 500 ms
//! içinde tek isteğe toplamak ve "Kor CLI ile aç"ı pencere açmadan terminale yönlendirmek.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kor_runtime::osint::{self, Origin, Request};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::api::Res;
use crate::state::AppState;

/// Olay adı (arayüz `api.osint.onRequest`).
pub const EVENT: &str = "kor:osint/request";
/// Windows bir dosya fiilini seçili her dosya için ayrı çalıştırır; bu süre içinde gelenler tek toplu istek olur.
const EDIT_WINDOW: Duration = Duration::from_millis(500);
/// Atlama listesi / Dock menüsündeki son proje sayısı (§25.3).
const JUMP_RECENT: usize = 5;
/// Kalıcı son projeler listesinin uzunluğu.
const MAX_RECENT: usize = 10;

/// Arayüz `osint.pending` ile ilk kez sorana kadar istekler kuyrukta bekler, sonra doğrudan olay olarak gider.
static READY: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Vec<Value>> = Mutex::new(Vec::new());
/// Toplanan "Kor ile düzenle" dosyaları ve son eklenme anı.
static EDIT: Mutex<(Vec<PathBuf>, Option<Instant>)> = Mutex::new((Vec::new(), None));
/// Kayıtlı global kısayol (yeniden kayıtta eskisi bırakılır).
static HOTKEY: Mutex<Option<Shortcut>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------
// Saf kurallar (testli)
// ---------------------------------------------------------------------------------------------

/// Bir isteğin masaüstünde ne yapacağı.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Pencereyi öne getir ve isteği arayüze ver.
    Emit,
    /// Aynı proje zaten açık, eklenecek dosya yok: yalnızca öne getir.
    FocusOnly,
    /// Toplu düzenleme kuyruğuna ekle.
    Collect(Vec<PathBuf>),
    /// Terminalde `kor` aç.
    Terminal(PathBuf),
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        kor_tools::without_verbatim(&c).to_string_lossy().to_lowercase()
    };
    norm(a) == norm(b)
}

fn route(req: &Request, open_root: Option<&Path>) -> Route {
    match req {
        Request::Open { root, files, origin: Origin::Shell } if files.is_empty() && open_root.is_some_and(|o| same_dir(o, root)) => {
            Route::FocusOnly
        }
        Request::Edit { files, origin: Origin::Shell } => Route::Collect(files.clone()),
        Request::Terminal { root, .. } => Route::Terminal(root.clone()),
        _ => Route::Emit,
    }
}

/// `new` dosyalarını sırayı koruyarak, tekrarsız ve en çok `osint::MAX_PATHS` olacak şekilde ekler.
fn merge(into: &mut Vec<PathBuf>, new: Vec<PathBuf>) {
    for p in new {
        if into.len() >= osint::MAX_PATHS {
            break;
        }
        if !into.contains(&p) {
            into.push(p);
        }
    }
}

/// Son eklemeden beri toplama penceresi doldu mu; dolmadıysa kalan süre.
fn remaining(last: Instant, now: Instant) -> Option<Duration> {
    let waited = now.saturating_duration_since(last);
    (waited < EDIT_WINDOW).then(|| EDIT_WINDOW - waited)
}

/// Kısayol metnini doğrular: boş = kapalı.
fn parse_hotkey(s: &str) -> Result<Option<Shortcut>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.len() > 64 {
        return Err("Kısayol çok uzun".into());
    }
    s.parse::<Shortcut>()
        .map(Some)
        .map_err(|e| format!("Geçersiz kısayol `{s}`: {e}"))
}

/// En yeni başa, tekrarsız, en çok `MAX_RECENT`.
fn push_recent(list: &mut Vec<String>, path: &str) -> bool {
    if list.first().is_some_and(|f| f == path) {
        return false;
    }
    list.retain(|p| p != path);
    list.insert(0, path.to_owned());
    list.truncate(MAX_RECENT);
    true
}

/// Atlama listesi / Dock menüsü içeriği.
fn jump_list(l: &kor_i18n::Localizer, recent: &[String]) -> kor_platform::jumplist::List {
    use kor_platform::jumplist::{Item, List};
    List {
        recent_title: l.t("osint-jump-recent"),
        recent: recent
            .iter()
            .take(JUMP_RECENT)
            .map(|p| {
                let path = Path::new(p);
                Item {
                    title: path
                        .file_name()
                        .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned()),
                    args: vec![osint::open_url(path)],
                }
            })
            .collect(),
        tasks: vec![
            Item { title: l.t("osint-jump-new"), args: vec!["kor://new".into()] },
            Item { title: l.t("osint-jump-running"), args: vec!["kor://running".into()] },
        ],
    }
}

// ---------------------------------------------------------------------------------------------
// Uygulama
// ---------------------------------------------------------------------------------------------

/// Paketle gelen `kor` (ana programın yanında); geliştirme derlemesinde PATH'teki `kor`.
fn bundled_cli() -> PathBuf {
    crate::kor_cli::path()
}

fn home_config() -> Option<kor_config::Loaded> {
    let root = dirs::home_dir()?;
    crate::bridge::load_config(&root).ok()
}

/// `kor-desktop --terminal <yol>…` ("Kor CLI ile aç"): terminali açar; pencere hiç oluşturulmaz.
/// `true` ise program hemen çıkmalıdır.
pub fn terminal_only() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--terminal") {
        return false;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let preferred = home_config().and_then(|c| c.config.shell.terminal.clone());
    for req in osint::parse_args(&args, &cwd).into_iter().flatten() {
        if let Request::Terminal { root, .. } = req {
            if let Err(e) = kor_platform::terminal::open(&root, preferred.as_deref(), &bundled_cli(), &[]) {
                eprintln!("Terminal açılamadı: {e}");
            }
        }
    }
    true
}

fn focus(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn emit(app: &AppHandle, req: &Request) {
    let payload = serde_json::to_value(req).unwrap_or(Value::Null);
    focus(app);
    if READY.load(Ordering::SeqCst) {
        let _ = app.emit(EVENT, payload);
    } else {
        lock(&PENDING).push(payload);
    }
}

fn dispatch(app: &AppHandle, mut req: Request) {
    // Hızlı görevin (§25.1 alt menü) istemi kullanıcının dilinde burada yazılır; arayüz yalnızca gönderir.
    if let Request::Task { prompt, root: Some(root), files, quick: Some(q), .. } = &mut req {
        if prompt.is_empty() {
            *prompt = q.prompt(&localizer(), root, files);
        }
    }
    let open_root = app.state::<AppState>().root();
    match route(&req, open_root.as_deref()) {
        Route::Emit => emit(app, &req),
        Route::FocusOnly => focus(app),
        Route::Terminal(root) => {
            let preferred = home_config().and_then(|c| c.config.shell.terminal.clone());
            if let Err(e) = kor_platform::terminal::open(&root, preferred.as_deref(), &bundled_cli(), &[]) {
                eprintln!("Terminal açılamadı: {e}");
            }
        }
        Route::Collect(files) => collect(app, files),
    }
}

fn collect(app: &AppHandle, files: Vec<PathBuf>) {
    let start = {
        let mut g = lock(&EDIT);
        merge(&mut g.0, files);
        let first = g.1.is_none();
        g.1 = Some(Instant::now());
        first
    };
    if !start {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let wait = {
                let g = lock(&EDIT);
                g.1.and_then(|last| remaining(last, Instant::now()))
            };
            match wait {
                Some(d) => tokio::time::sleep(d).await,
                None => break,
            }
        }
        let files = {
            let mut g = lock(&EDIT);
            g.1 = None;
            std::mem::take(&mut g.0)
        };
        if !files.is_empty() {
            emit(&app, &Request::Edit { files, origin: Origin::Shell });
        }
    });
}

/// Argümanlar (ilk açılış ya da tek örnek eklentisinin ilettiği ikinci açılış). `kor:` bağlantıları
/// deep-link eklentisinden gelir; burada atlanır ki iki kez işlenmesin.
fn handle_args(app: &AppHandle, args: &[String], cwd: &Path) {
    let args: Vec<String> = args.iter().filter(|a| !a.starts_with("kor:")).cloned().collect();
    for r in osint::parse_args(&args, cwd) {
        match r {
            Ok(req) => dispatch(app, req),
            Err(e) => eprintln!("Kor: geçersiz argüman: {e}"),
        }
    }
}

/// `kor://` ve (macOS'ta Dock'a / "Birlikte aç"a bırakılan) `file://` adresleri.
fn handle_urls(app: &AppHandle, urls: Vec<tauri::Url>) {
    let mut dropped = Vec::new();
    for u in urls {
        if u.scheme() == "file" {
            if let Ok(p) = u.to_file_path() {
                dropped.push(p);
            }
            continue;
        }
        match osint::parse_url(u.as_str()) {
            Ok(req) => dispatch(app, req),
            Err(e) => eprintln!("Kor: geçersiz bağlantı: {e}"),
        }
    }
    if let Some((root, files)) = osint::resolve(&dropped) {
        dispatch(app, Request::Open { root, files, origin: Origin::Shell });
    }
}

/// Tek örnek eklentisinin geri çağrısı: ikinci açılışın argümanları bu pencereye gelir.
pub fn second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    let args: Vec<String> = argv.into_iter().skip(1).collect();
    if args.is_empty() {
        focus(app);
        return;
    }
    handle_args(app, &args, Path::new(&cwd));
}

/// Global kısayol eklentisi: basılınca öne gel ve komut paletini aç.
pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                emit(app, &Request::Palette);
            }
        })
        .build()
}

fn register_hotkey(app: &AppHandle, next: Option<Shortcut>) -> Result<(), String> {
    let gs = app.global_shortcut();
    let mut current = lock(&HOTKEY);
    if let Some(old) = current.take() {
        let _ = gs.unregister(old);
    }
    if let Some(s) = next {
        gs.register(s)
            .map_err(|e| format!("Kısayol kaydedilemedi (başka bir uygulama kullanıyor olabilir): {e}"))?;
        *current = Some(s);
    }
    Ok(())
}

fn recent_file() -> Option<PathBuf> {
    kor_platform::data_dir().map(|d| d.join("desktop-recent.json"))
}

fn load_recent() -> Vec<String> {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default()
}

fn save_recent(list: &[String]) {
    if let Some(f) = recent_file() {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, serde_json::to_string(list).unwrap_or_default());
    }
}

fn localizer() -> kor_i18n::Localizer {
    let lang = home_config().map_or(kor_i18n::Lang::En, |c| c.config.ui.language.resolve());
    kor_i18n::Localizer::new(lang)
}

fn apply_jump_list(app: &AppHandle, recent: &[String]) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let list = jump_list(&localizer(), recent);
    // macOS AppKit ana iş parçacığı ister; Windows kendi COM iş parçacığını açar.
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = kor_platform::jumplist::apply(&exe, &list) {
            eprintln!("Atlama listesi güncellenemedi: {e}");
        }
    });
}

/// Açılan proje son projelere girer ve atlama listesi / Dock menüsü tazelenir. `AppState` yalnızca bellekte
/// tuttuğu için açık klasör birkaç saniyede bir yoklanır.
fn watch_recent(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut recent = load_recent();
        apply_jump_list(&app, &recent);
        let mut last: Option<PathBuf> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let root = app.state::<AppState>().root();
            if root == last {
                continue;
            }
            last.clone_from(&root);
            if let Some(r) = root.filter(|r| r.is_dir()) {
                if push_recent(&mut recent, &r.to_string_lossy()) {
                    save_recent(&recent);
                    apply_jump_list(&app, &recent);
                }
            }
        }
    });
}

/// `setup` içinde: bağlantı dinleyicisi, ilk açılışın argümanları ve bağlantısı, kısayol, atlama listesi.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| handle_urls(&handle, event.urls()));
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        handle_urls(app, urls);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    handle_args(app, &args, &std::env::current_dir().unwrap_or_default());
    let hotkey = home_config().map_or_else(|| kor_config::DesktopConfig::default().hotkey, |c| c.config.desktop.hotkey);
    match parse_hotkey(&hotkey) {
        Ok(s) => {
            if let Err(e) = register_hotkey(app, s) {
                eprintln!("{e}");
            }
        }
        Err(e) => eprintln!("{e}"),
    }
    watch_recent(app.clone());
}

// ---------------------------------------------------------------------------------------------
// Köprü yöntemleri (`ns/osint.rs`)
// ---------------------------------------------------------------------------------------------

/// `osint.pending`: arayüz dinlemeye başlamadan gelen istekler; bundan sonrası olay olarak gelir.
pub fn pending() -> Value {
    READY.store(true, Ordering::SeqCst);
    Value::Array(std::mem::take(&mut *lock(&PENDING)))
}

/// `osint.drop(paths)`: pencereye (yazma alanı dışına) bırakılan tek klasör proje olarak açılır.
pub fn dropped(paths: &[String]) -> Value {
    match paths {
        [one] if Path::new(one).is_dir() => {
            let root = osint::project_root(Path::new(one));
            serde_json::to_value(Request::Open { root, files: vec![], origin: Origin::Shell }).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// `osint.recent`: kalıcı son projeler (atlama listesinin kaynağı).
pub fn recent() -> Value {
    json!(load_recent())
}

fn user_config() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("config.toml"))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

/// `osint.hotkeyGet`.
pub fn hotkey_get() -> Value {
    let value = home_config().map_or_else(|| kor_config::DesktopConfig::default().hotkey, |c| c.config.desktop.hotkey);
    json!({ "hotkey": value, "default": kor_config::DesktopConfig::default().hotkey, "active": lock(&HOTKEY).is_some() })
}

/// `osint.hotkeySet(value)`: doğrular, hemen kaydeder (başarısızsa eskisini geri kaydeder), sonra ayara yazar.
pub fn hotkey_set(app: &AppHandle, value: &str) -> Res {
    let next = parse_hotkey(value)?;
    let previous = lock(&HOTKEY).as_ref().copied();
    if let Err(e) = register_hotkey(app, next) {
        let _ = register_hotkey(app, previous);
        return Err(e);
    }
    kor_config::set_string(&user_config()?, "desktop.hotkey", value.trim()).map_err(|e| e.to_string())?;
    Ok(hotkey_get())
}

fn apps() -> kor_platform::shell_integration::Apps {
    let cli = crate::kor_cli::find();
    kor_platform::shell_integration::Apps { desktop: std::env::current_exe().ok(), cli }
}

/// `osint.menuStatus`.
pub fn menu_status() -> Res {
    serde_json::to_value(kor_platform::shell_integration::status(&apps())).map_err(|e| e.to_string())
}

/// `osint.menuInstall(extended)`: yalnızca arayüzdeki açık onaydan sonra (kullanıcı kaydına yazar).
pub fn menu_install(extended: bool) -> Res {
    let lang = home_config().map_or(kor_i18n::Lang::Tr, |c| c.config.ui.language.resolve());
    let opts = kor_platform::shell_integration::Options { extended, lang: lang.code().to_owned() };
    // The CLI (`kor integrate install`) reads the same switch.
    kor_config::set_scalar(&user_config()?, "shell.context_menu.extended", if extended { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    kor_platform::shell_integration::install(&apps(), &opts)
        .map_err(|e| format!("Sağ tık menüsü kurulamadı: {e}"))
        .and_then(|s| serde_json::to_value(s).map_err(|e| e.to_string()))
}

/// İlk açılış (ADR 0052; 11 Kasım 2026 kullanıcı kararı): sağ tık menüsü, varsayılan kabuğun
/// entegrasyonu ve bu işletim sisteminin kullanılabilir §27.10 öğeleri bir kez kurulur; ne yapıldığı
/// veri klasöründeki işarete yazılır, kullanıcının sonradan kapattığı öğe bir daha kurulmaz. `tauri dev`,
/// `cargo test`/`cargo run` ve `KOR_HOME`/`KOR_CONFIG_DIR`/`KOR_CACHE_DIR` ile hiç çalışmaz (gerçek
/// kayıt defteri ve kabuk profilleri yalnızca kurulu sürümde değişir).
pub fn first_run(app: &AppHandle) {
    use kor_platform::os_integration as osx;
    use kor_runtime::autointegrate::{self as auto, Item, Outcome};
    let blocked = auto::guard_here(tauri::is_dev());
    if blocked.is_some() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(data) = kor_platform::data_dir() else {
            return;
        };
        let cfg = home_config().map(|c| c.config).unwrap_or_default();
        // Items already attempted (by the CLI or an earlier launch) are skipped inside `run_once`.
        if !cfg.integrations.auto {
            return;
        }
        let shell = Some(kor_runtime::shell::default_kind()).filter(|k| kor_runtime::shell::present(*k));
        let mut env = crate::ns::osextra::env(&app).ok();
        let avail: Vec<&'static str> = env
            .as_ref()
            .map(|e| {
                osx::status_all(e)
                    .into_iter()
                    .filter(|s| matches!(s.kind, "install" | "desktop") && s.available && !s.installed)
                    .map(|s| s.id)
                    .collect()
            })
            .unwrap_or_default();
        let items = auto::plan(&cfg.integrations, shell, &avail);
        let lang = cfg.ui.language.resolve();
        let out = auto::run_once(&data, blocked, &items, |it| match it {
            Item::ContextMenu => {
                let opts = kor_platform::shell_integration::Options { extended: cfg.shell.context_menu.extended, lang: lang.code().to_owned() };
                kor_platform::shell_integration::install(&apps(), &opts).map(drop).map_err(|e| e.to_string())
            }
            Item::Shell(k) => kor_runtime::shell::plan_install(*k)
                .map_err(|e| format!("{e:?}"))
                .and_then(|p| kor_runtime::shell::apply(&p).map_err(|e| e.to_string())),
            Item::Os(id) => match env.as_mut() {
                Some(e) => osx::install(e, id, &osx::Options::default()).map(drop).map_err(|e| e.to_string()),
                None => Err("no-env".into()),
            },
        });
        match out {
            Ok(Outcome::Ran(m)) if !m.installed.is_empty() => {
                let l = kor_i18n::Localizer::new(lang);
                crate::presence::notify(
                    &app,
                    kor_runtime::presence::notifications::Notification {
                        category: kor_runtime::presence::notifications::Category::Component,
                        title: l.t("autoint-notice-title"),
                        body: l.t_args("autoint-notice-body", &kor_i18n::args!("list" => auto::names(&l, &m.installed))),
                        key: Some("autointegrate:first-run".into()),
                        ..Default::default()
                    },
                );
            }
            Ok(_) => {}
            Err(e) => eprintln!("Kor: ilk açılış entegrasyon kaydı yazılamadı: {e}"),
        }
    });
}

/// `osint.menuUninstall`: Kor'un eklediği her şeyi kaldırır.
pub fn menu_uninstall() -> Res {
    let s = kor_platform::shell_integration::uninstall().map_err(|e| format!("Sağ tık menüsü kaldırılamadı: {e}"))?;
    crate::api::remember_off("integrations.context_menu");
    serde_json::to_value(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_focuses_same_project_and_collects_edits() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        let open = Request::Open { root: root.clone(), files: vec![], origin: Origin::Shell };
        assert_eq!(route(&open, Some(&root)), Route::FocusOnly);
        assert_eq!(route(&open, None), Route::Emit);
        let with_file = Request::Open { root: root.clone(), files: vec![root.join("a")], origin: Origin::Shell };
        assert_eq!(route(&with_file, Some(&root)), Route::Emit);
        // A link always reaches the UI (it asks before switching).
        let link = Request::Open { root: root.clone(), files: vec![], origin: Origin::Link };
        assert_eq!(route(&link, Some(&root)), Route::Emit);
        let edit = Request::Edit { files: vec![root.join("x")], origin: Origin::Shell };
        assert_eq!(route(&edit, None), Route::Collect(vec![root.join("x")]));
        let edit_link = Request::Edit { files: vec![root.join("x")], origin: Origin::Link };
        assert_eq!(route(&edit_link, None), Route::Emit);
        assert_eq!(
            route(&Request::Terminal { root: root.clone(), files: vec![] }, None),
            Route::Terminal(root)
        );
        assert_eq!(route(&Request::Palette, None), Route::Emit);
    }

    #[test]
    fn edit_batches_merge_and_wait_for_quiet() {
        let mut v = vec![PathBuf::from("/a")];
        merge(&mut v, vec![PathBuf::from("/b"), PathBuf::from("/a")]);
        assert_eq!(v, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        let mut big = Vec::new();
        merge(&mut big, (0..osint::MAX_PATHS + 10).map(|i| PathBuf::from(format!("/{i}"))).collect());
        assert_eq!(big.len(), osint::MAX_PATHS);
        let t = Instant::now();
        assert_eq!(remaining(t, t), Some(EDIT_WINDOW));
        assert_eq!(remaining(t, t + Duration::from_millis(200)), Some(Duration::from_millis(300)));
        assert_eq!(remaining(t, t + EDIT_WINDOW), None);
    }

    #[test]
    fn hotkeys_are_validated() {
        assert!(parse_hotkey("").unwrap().is_none());
        assert!(parse_hotkey("  ").unwrap().is_none());
        assert!(parse_hotkey("CmdOrCtrl+Shift+K").unwrap().is_some());
        assert!(parse_hotkey(&kor_config::DesktopConfig::default().hotkey).unwrap().is_some());
        assert!(parse_hotkey("Alt+Space").unwrap().is_some());
        assert!(parse_hotkey("Shift+Nope").is_err());
        assert!(parse_hotkey(&"K".repeat(80)).is_err());
    }

    #[test]
    fn recent_projects_and_jump_list() {
        let mut r = vec!["/a".to_owned(), "/b".to_owned()];
        assert!(!push_recent(&mut r, "/a"));
        assert!(push_recent(&mut r, "/b"));
        assert_eq!(r, vec!["/b", "/a"]);
        for i in 0..20 {
            push_recent(&mut r, &format!("/p{i}"));
        }
        assert_eq!(r.len(), MAX_RECENT);
        let l = kor_i18n::Localizer::new(kor_i18n::Lang::Tr);
        let list = jump_list(&l, &r);
        assert_eq!(list.recent.len(), JUMP_RECENT);
        assert_eq!(list.recent[0].title, "p19");
        assert!(list.recent[0].args[0].starts_with("kor://open?path="));
        assert_eq!(list.tasks.iter().map(|t| t.args[0].as_str()).collect::<Vec<_>>(), ["kor://new", "kor://running"]);
        // Every link in the list parses back.
        for it in list.recent.iter().chain(&list.tasks) {
            kor_runtime::osint::parse_url(&it.args[0]).unwrap();
        }
    }

    #[test]
    fn a_single_dropped_folder_opens_its_project() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        let sub = d.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, "").unwrap();
        let v = dropped(&[sub.to_string_lossy().into_owned()]);
        assert_eq!(v["kind"], "open");
        assert_eq!(PathBuf::from(v["root"].as_str().unwrap()), d.path());
        assert!(dropped(&[f.to_string_lossy().into_owned()]).is_null());
        assert!(dropped(&[sub.to_string_lossy().into_owned(), sub.to_string_lossy().into_owned()]).is_null());
    }
}
