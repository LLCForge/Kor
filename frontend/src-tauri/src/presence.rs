//! Masaüstü kabuk varlığı (§27.1 tepsi, §27.2 bildirimler, §27.8 PiP, §27.9 görev çubuğu ve çoklu monitör,
//! §28.4 "beni çağır", §28.5 ETA ve onay kuyruğu; ADR 0051).
//!
//! Sohbet olayları (`chat::emit`) `kor_runtime::presence::tracker` ile katlanır; buradan tepsi simgesi/menüsü,
//! OS bildirimi, görev çubuğu ilerlemesi, dikkat çekme ve `kor status` için anlık görüntü üretilir. CLI'ın
//! `kor approve|deny` yanıtları `<kor home>/presence/answers/` klasöründen saniyede bir alınır. İşletim
//! sistemine özgü kod yoktur: Tauri API'leri ve `kor_platform` (toast, taskbar, foreground).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use kor_i18n::{Localizer, args};
use kor_runtime::presence::attention::{self, Alert, Context, Policy};
use kor_runtime::presence::desktop::{self, Approval, Running, Snapshot, Source};
use kor_runtime::presence::eta::{self, History, Sample};
use kor_runtime::presence::layout::{self, Layouts, Monitor, Rect};
use kor_runtime::presence::notifications::{Category, Notification, Store};
use kor_runtime::presence::queue;
use kor_runtime::presence::tracker::{Effect, Tracker};
use serde_json::{Value, json};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::state::AppState;

pub const MAIN: &str = "main";
pub const PIP: &str = "pip";
pub const POPUP: &str = "traypop";
const TRAY: &str = "kor-tray";
/// Mantıksal piksel (§27.8: ≈ 240×72).
const PIP_SIZE: (f64, f64) = (240.0, 72.0);
const POPUP_SIZE: (f64, f64) = (340.0, 480.0);
/// Ember (çalışıyor) ve sarı (onay bekliyor), `kor-theme` renkleri.
const EMBER: [u8; 4] = [0xE8, 0x6A, 0x2C, 0xFF];
const AMBER: [u8; 4] = [0xF2, 0xB7, 0x05, 0xFF];

#[derive(Debug, Clone, Default)]
struct Settings {
    policy: Policy,
    tray: kor_config::Tray,
    pip: kor_config::Pip,
    lang: Option<kor_i18n::Lang>,
}

#[derive(Default)]
struct Shell {
    tracker: Mutex<Tracker>,
    settings: Mutex<Settings>,
    samples: Mutex<Vec<Sample>>,
    paused: AtomicBool,
    last_input: AtomicU64,
    main_focused: AtomicBool,
    dirty: AtomicBool,
    /// PiP kullanıcı tarafından açıldı (otomatik gizleme dokunmaz).
    pip_manual: AtomicBool,
    /// Tam ekran uygulama yüzünden gizlendi.
    pip_suppressed: AtomicBool,
    pip_hide_gen: AtomicU64,
    blink: AtomicBool,
    /// Son tur hatayla bitti (görev çubuğu kırmızı, sonraki tura ya da odaklanmaya kadar).
    failed: AtomicBool,
    update_on_exit: AtomicBool,
    layouts_pending: Mutex<BTreeSet<String>>,
}

fn shell() -> &'static Shell {
    static S: OnceLock<Shell> = OnceLock::new();
    S.get_or_init(Shell::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    kor_runtime::cost::now_ms()
}

fn hour() -> u8 {
    use chrono::Timelike as _;
    u8::try_from(chrono::Local::now().hour()).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Ayarlar
// ---------------------------------------------------------------------------------------------

fn load_settings(app: &AppHandle) {
    let state = app.state::<AppState>();
    let root = state.root().or_else(dirs::home_dir).unwrap_or_default();
    let cfg = crate::bridge::load_config(&root)
        .map(|l| l.config)
        .unwrap_or_default();
    let mut policy = Policy::from(&cfg.notify);
    // §28.14 `auto.sound = false` kapatır (ADR 0050).
    policy.sound &= cfg.auto.sound;
    *lock(&shell().settings) = Settings {
        policy,
        tray: cfg.tray.clone(),
        pip: cfg.pip.clone(),
        lang: Some(cfg.ui.language.resolve()),
    };
}

fn settings() -> Settings {
    lock(&shell().settings).clone()
}

fn l() -> Localizer {
    Localizer::new(settings().lang.unwrap_or(kor_i18n::Lang::Tr))
}

/// `shellui.settings` yanıtı.
pub fn settings_json() -> Value {
    let s = settings();
    json!({
        "notify": {
            "toast": s.policy.toast, "bell": s.policy.bell, "flash": s.policy.flash,
            "sound": s.policy.sound, "idleSecs": s.policy.idle_secs,
            "quietHours": s.policy.quiet.map(|(a, b)| format!("{a:02}-{b:02}")).unwrap_or_default(),
            "muted": s.policy.muted.iter().map(|c| c.key()).collect::<Vec<_>>(),
        },
        "tray": { "enabled": s.tray.enabled, "closeToTray": s.tray.close_to_tray },
        "pip": { "autoShow": s.pip.auto_show, "hideFullscreen": s.pip.hide_fullscreen, "opacity": s.pip.opacity },
    })
}

/// Kullanıcı `config.toml` anahtarı (izin listesi) ve uygulanması.
pub fn set_setting(app: &AppHandle, key: &str, value: &Value) -> Result<Value, String> {
    let file = kor_config::user_dir()
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?
        .join("config.toml");
    let bool_keys = [
        "notify.toast",
        "notify.bell",
        "notify.flash",
        "notify.sound",
        "tray.enabled",
        "tray.close_to_tray",
        "pip.auto_show",
        "pip.hide_fullscreen",
    ];
    if bool_keys.contains(&key) {
        let v = value.as_bool().ok_or("Açık/kapalı değeri gerekli")?;
        kor_config::set_scalar(&file, key, if v { "true" } else { "false" })
            .map_err(|e| e.to_string())?;
    } else if key == "notify.idle_secs" {
        let v = value
            .as_u64()
            .filter(|v| *v <= 3600)
            .ok_or("Saniye 0-3600 olmalı")?;
        kor_config::set_scalar(&file, key, &v.to_string()).map_err(|e| e.to_string())?;
    } else if key == "pip.opacity" {
        let v = value
            .as_f64()
            .filter(|v| (0.3..=1.0).contains(v))
            .ok_or("Saydamlık 0.3-1.0 olmalı")?;
        kor_config::set_scalar(&file, key, &format!("{v:.2}")).map_err(|e| e.to_string())?;
    } else if key == "notify.quiet_hours" {
        let v = value.as_str().ok_or("Saat aralığı metni gerekli")?.trim();
        if !v.is_empty() && attention::quiet_hours(v).is_none() {
            return Err("Sessiz saatler SS-SS biçiminde olmalı (ör. 22-08)".into());
        }
        kor_config::set_string(&file, key, v).map_err(|e| e.to_string())?;
    } else if key == "notify.muted" {
        let list: Vec<String> = value
            .as_array()
            .ok_or("Kategori listesi gerekli")?
            .iter()
            .filter_map(Value::as_str)
            .filter(|c| Category::parse(c).is_some())
            .map(str::to_owned)
            .collect();
        kor_config::set_string_list(&file, key, &list).map_err(|e| e.to_string())?;
    } else {
        return Err(format!("Bilinmeyen ayar: {key}"));
    }
    load_settings(app);
    apply_tray_setting(app);
    mark_dirty(app);
    Ok(settings_json())
}

// ---------------------------------------------------------------------------------------------
// Başlatma ve döngü
// ---------------------------------------------------------------------------------------------

/// Uygulama açılışında.
pub fn start(app: &AppHandle) {
    load_settings(app);
    *lock(&shell().samples) = History::standard().map(|h| h.samples()).unwrap_or_default();
    shell().last_input.store(now_ms(), Ordering::Relaxed);
    restore_main(app);
    apply_tray_setting(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            tick += 1;
            take_answers(&app);
            if tick.is_multiple_of(10) {
                load_settings(&app);
            }
            let dirty = shell().dirty.swap(false, Ordering::Relaxed);
            if dirty || tick.is_multiple_of(5) {
                write_snapshot();
            }
            if dirty {
                refresh(&app);
            } else if !lock(&shell().tracker).sessions.is_empty() {
                // ETA ve süre saniyede bir ilerler.
                let _ = app.emit("kor:shellui/changed", ());
            }
            blink_tray(&app);
            if tick == 3 || tick.is_multiple_of(60) {
                let a = app.clone();
                tauri::async_runtime::spawn(async move { refresh_peers(&a).await });
            }
            if tick.is_multiple_of(2) {
                fullscreen_check(&app).await;
            }
            save_layouts(&app);
        }
    });
}

/// Uygulama kapanırken: anlık görüntü kaldırılır (CLI masaüstünü kapalı görür), bekleyen güncelleme kurulur.
pub fn shutdown() {
    if let Some(d) = kor_runtime::presence::dir() {
        desktop::remove(&d);
    }
    if shell().update_on_exit.load(Ordering::Relaxed) {
        crate::packaging::install_pending_on_exit();
    }
}

fn mark_dirty(app: &AppHandle) {
    shell().dirty.store(true, Ordering::Relaxed);
    let _ = app.emit("kor:shellui/changed", ());
}

fn refresh(app: &AppHandle) {
    update_tray(app);
    update_taskbar(app);
    pip_auto(app);
}

fn sessions_with_eta() -> Vec<Running> {
    let now = now_ms();
    let samples = lock(&shell().samples).clone();
    lock(&shell().tracker)
        .sessions
        .values()
        .map(|r| {
            let mut r = r.clone();
            r.eta = eta::estimate(&samples, &r.root, r.since, now);
            r
        })
        .collect()
}

fn desktop_approvals() -> Vec<Approval> {
    lock(&shell().tracker).approvals.clone()
}

fn write_snapshot() {
    let Some(dir) = kor_runtime::presence::dir() else {
        return;
    };
    let snap = Snapshot {
        pid: std::process::id(),
        updated: now_ms(),
        sessions: sessions_with_eta(),
        approvals: desktop_approvals(),
        paused: shell().paused.load(Ordering::Relaxed),
    };
    if let Err(e) = desktop::write(&dir, &snap) {
        eprintln!("kor: durum dosyası yazılamadı: {e}");
    }
}

// ---------------------------------------------------------------------------------------------
// Sohbet olayları
// ---------------------------------------------------------------------------------------------

/// Tur başladı (`chat::run_turn`).
pub fn turn_started(app: &AppHandle, session: &str, root: &std::path::Path, text: &str) {
    let custom = crate::meta::read()
        .ok()
        .and_then(|m| m.sessions.get(session).and_then(|s| s.custom_title.clone()));
    let title = custom.unwrap_or_else(|| {
        text.lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect()
    });
    let was_idle = {
        let mut t = lock(&shell().tracker);
        let idle = t.sessions.is_empty();
        t.start(session, &title, &root.to_string_lossy(), now_ms());
        idle
    };
    shell().failed.store(false, Ordering::Relaxed);
    if was_idle {
        shell().pip_hide_gen.fetch_add(1, Ordering::Relaxed);
    }
    mark_dirty(app);
}

/// Model çağrısının kullanımı (`chat::run_turn` pompası): katalog fiyatıyla ≈ maliyet.
pub fn usage(_app: &AppHandle, session: &str, model_ref: &str, u: &kor_core::Usage) {
    let Some((pin, pout)) = kor_runtime::cost::Prices::load().get(model_ref) else {
        return;
    };
    #[allow(clippy::cast_precision_loss)] // jeton sayıları 2^52'nin çok altında
    let usd = (u.input as f64 * pin + u.output as f64 * pout) / 1_000_000.0;
    // Yerel modeller ücretsizdir (fiyat 0); diğerleri katalog tahminidir (≈).
    let local = pin <= 0.0 && pout <= 0.0;
    lock(&shell().tracker).add_cost(session, usd, !local);
    shell().dirty.store(true, Ordering::Relaxed);
}

/// Her `kor:event` gövdesi (`chat::emit`).
pub fn observe(app: &AppHandle, session: &str, body: &Value) {
    let kind = body["type"].as_str().unwrap_or_default();
    if !matches!(
        kind,
        "status" | "tool" | "toolResult" | "confirm" | "fileChanged" | "error" | "done"
    ) {
        return;
    }
    let effects = lock(&shell().tracker).observe(session, body, now_ms());
    if kind == "status" {
        shell().dirty.store(true, Ordering::Relaxed);
    } else {
        mark_dirty(app);
    }
    for fx in effects {
        match fx {
            Effect::Waiting {
                session,
                approval,
                category,
                tool,
                summary,
            } => {
                let l = l();
                let title = lock(&shell().tracker)
                    .sessions
                    .get(&session)
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                notify(
                    app,
                    Notification {
                        category,
                        title: l.t(if category == Category::Question {
                            "presence-question-title"
                        } else {
                            "presence-waiting-title"
                        }),
                        body: l.t_args(
                            "presence-approval-body",
                            &args!("title" => title, "tool" => tool, "summary" => summary),
                        ),
                        session: Some(session),
                        approval: Some(approval),
                        ..Notification::default()
                    },
                );
            }
            Effect::Ended {
                session,
                title,
                secs,
                root,
                cost_usd,
                cost_estimated,
                files,
                error,
            } => ended(
                app,
                session,
                title,
                secs,
                &root,
                (cost_usd, cost_estimated),
                files,
                error,
            ),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ended(
    app: &AppHandle,
    session: String,
    title: String,
    secs: u64,
    root: &str,
    (cost_usd, cost_estimated): (f64, bool),
    files: u32,
    error: Option<String>,
) {
    if error.is_none()
        && let Some(h) = History::standard()
    {
        let _ = h.record(root, secs);
        *lock(&shell().samples) = h.samples();
    }
    let l = l();
    let cost = kor_runtime::cost::fmt_usd(cost_usd, cost_estimated);
    let n = match error {
        Some(e) => {
            shell().failed.store(true, Ordering::Relaxed);
            Notification {
                category: Category::Error,
                title: l.t("presence-error-title"),
                body: l.t_args(
                    "presence-error-body",
                    &args!("title" => title, "error" => e),
                ),
                session: Some(session),
                ..Notification::default()
            }
        }
        None => Notification {
            category: Category::Done,
            title: l.t("presence-done-title"),
            body: l.t_args(
                "presence-done-body",
                &args!(
                    "title" => title,
                    "time" => format!("{}:{:02}", secs / 60, secs % 60),
                    "cost" => cost,
                    "files" => i64::from(files),
                    "pending" => i64::try_from(desktop_approvals().len()).unwrap_or(0)
                ),
            ),
            session: Some(session),
            cost_usd: Some(cost_usd),
            cost_estimated,
            files: Some(files),
            ..Notification::default()
        },
    };
    notify(app, n);
    budget_check(app);
}

/// Bütçe uyarısı (günlük/aylık sınırın `alert_at_percent`'i), gün/ay başına bir kez.
fn budget_check(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(cfg) = state
        .root()
        .or_else(dirs::home_dir)
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map(|l| l.config)
    else {
        return;
    };
    let entries = kor_runtime::usage::file()
        .map(|p| kor_runtime::usage::read(&p))
        .unwrap_or_default();
    let r = kor_runtime::cost::report(
        &entries,
        &kor_runtime::cost::Prices::load(),
        now_ms(),
        1,
        &cfg.budget,
    );
    let Some(a) = attention::budget_alert(&r, cfg.budget.alert_at_percent) else {
        return;
    };
    let period = if a.scope == "daily" {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    } else {
        chrono::Local::now().format("%Y-%m").to_string()
    };
    let l = l();
    notify(
        app,
        Notification {
            category: Category::Budget,
            title: l.t("presence-budget-title"),
            body: l.t_args(
                "presence-budget-body",
                &args!(
                    "scope" => a.scope,
                    "spent" => kor_runtime::cost::fmt_usd(a.spent, a.estimated),
                    "limit" => kor_runtime::cost::fmt_usd(a.limit, false),
                    "pct" => i64::from(a.percent)
                ),
            ),
            key: Some(format!("budget:{}:{period}", a.scope)),
            ..Notification::default()
        },
    );
}

/// Yeni sürüm (packaging denetimi buldu).
pub fn update_available(app: &AppHandle, version: &str) {
    let l = l();
    notify(
        app,
        Notification {
            category: Category::Update,
            title: l.t_args(
                "presence-update-title",
                &args!("version" => version.to_owned()),
            ),
            body: l.t("presence-update-body"),
            key: Some(format!("update:{version}")),
            ..Notification::default()
        },
    );
    mark_dirty(app);
}

/// Onay arayüzde yanıtlandı (`Chats::approve`).
pub fn answered(app: &AppHandle, approval: &str) {
    lock(&shell().tracker).answered(approval);
    if let Some(s) = Store::standard() {
        let _ = s.resolve_approval(approval);
    }
    mark_dirty(app);
}

// ---------------------------------------------------------------------------------------------
// Bildirim ve dikkat
// ---------------------------------------------------------------------------------------------

fn window(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    app.get_webview_window(label)
}

/// Ana pencere görünür ve odakta mı.
fn main_active(app: &AppHandle) -> bool {
    window(app, MAIN).is_some_and(|w| {
        w.is_visible().unwrap_or(false)
            && !w.is_minimized().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
    })
}

fn context(app: &AppHandle) -> Context {
    Context {
        focused: main_active(app),
        idle_secs: now_ms().saturating_sub(shell().last_input.load(Ordering::Relaxed)) / 1000,
        hour: hour(),
        paused: shell().paused.load(Ordering::Relaxed),
    }
}

/// Bildirim merkezine ekler ve "beni çağır" kuralına göre OS bildirimi, ses, dikkat çekme uygular.
pub fn notify(app: &AppHandle, n: Notification) {
    let Some(store) = Store::standard() else {
        return;
    };
    let Ok(Some(saved)) = store.push(n) else {
        return;
    };
    let alert: Alert = attention::decide(&settings().policy, saved.category, context(app));
    let _ = app.emit(
        "kor:shellui/notification",
        json!({ "notification": saved, "alert": alert }),
    );
    if alert.toast {
        let app2 = app.clone();
        let target = (saved.session.clone(), saved.approval.clone());
        let toast = kor_platform::toast::Toast {
            title: saved.title.clone(),
            body: saved.body.clone(),
            app_id: (!cfg!(debug_assertions)).then(|| app.config().identifier.clone()),
        };
        if let Err(e) = kor_platform::toast::show(&toast, move || {
            open_target(&app2, target.0.as_deref(), target.1.is_some());
        }) {
            eprintln!("kor: OS bildirimi gösterilemedi: {e}");
        }
    }
    if alert.flash
        && let Some(w) = window(app, MAIN)
    {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Critical));
    }
}

/// Bildirime tıklanınca: ana pencere öne gelir, ilgili sohbet (ya da onay kuyruğu) açılır.
pub fn open_target(app: &AppHandle, session: Option<&str>, approval: bool) {
    show_main(app);
    if let Some(s) = session {
        let _ = app.emit("kor:shellui/openSession", s);
    } else if approval {
        let _ = app.emit("kor:shellui/openPanel", "approvals");
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = window(app, MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Arayüzde tuş/fare girişi (`shellui.activity`).
pub fn activity() {
    shell().last_input.store(now_ms(), Ordering::Relaxed);
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    shell().paused.store(paused, Ordering::Relaxed);
    mark_dirty(app);
}

pub fn paused() -> bool {
    shell().paused.load(Ordering::Relaxed)
}

pub fn set_update_on_exit(on: bool) {
    shell().update_on_exit.store(on, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------
// Onaylar
// ---------------------------------------------------------------------------------------------

/// `kor approve|deny` yanıtları (yalnızca şu an bekleyen masaüstü onaylarına uygulanır).
fn take_answers(app: &AppHandle) {
    let Some(dir) = kor_runtime::presence::dir() else {
        return;
    };
    for (id, approve) in desktop::take_answers(&dir) {
        let pending = desktop_approvals().iter().any(|a| a.id == id);
        if pending {
            let _ = answer_desktop(app, &id, approve);
        }
    }
}

fn answer_desktop(app: &AppHandle, id: &str, approve: bool) -> Result<(), String> {
    let session = desktop_approvals()
        .into_iter()
        .find(|a| a.id == id)
        .map(|a| a.owner);
    app.state::<AppState>()
        .chats
        .approve(id, if approve { "yes" } else { "no" }, None, "tray")?;
    answered(app, id);
    if let Some(s) = session {
        let _ = app.emit("kor:shellui/answered", json!({ "sessionId": s, "id": id }));
    }
    Ok(())
}

/// Herhangi bir kaynaktaki onayı yanıtlar (tepsi, PiP, zil, kuyruk paneli).
pub async fn answer(app: &AppHandle, id: &str, approve: bool) -> Result<&'static str, String> {
    if desktop_approvals().iter().any(|a| a.id == id) {
        answer_desktop(app, id, approve)?;
        return Ok(Source::Desktop.key());
    }
    let src = queue::answer(id, approve).await?;
    mark_dirty(app);
    Ok(src.key())
}

/// Bütün kaynaklardaki onay kuyruğu (masaüstü onayları bellekten, en güncel hâliyle).
pub async fn queue_items() -> Vec<queue::Item> {
    let mut all: Vec<Approval> = queue::collect()
        .await
        .into_iter()
        .map(|i| i.approval)
        .filter(|a| a.source != Source::Desktop)
        .collect();
    all.extend(desktop_approvals());
    queue::items(all)
}

/// `shellui.state`: tepsi açılır penceresi, PiP ve zil için her şey.
pub async fn state_json(app: &AppHandle) -> Value {
    let mut o = kor_runtime::presence::overview::collect().await;
    o.desktop = true;
    o.sessions = sessions_with_eta();
    o.approvals = queue_items().await;
    o.groups = queue::groups(&o.approvals);
    o.paused = paused();
    let unread = Store::standard().map_or(0, |s| s.unread());
    let update = crate::packaging::available_version();
    let state = app.state::<AppState>();
    let recent: Vec<String> = state.recent().into_iter().take(3).collect();
    json!({
        "overview": o,
        "unread": unread,
        "update": update,
        "recent": recent,
        "pip": { "opacity": settings().pip.opacity, "visible": window(app, PIP).is_some_and(|w| w.is_visible().unwrap_or(false)) },
        "progress": match lock(&shell().tracker).progress() {
            None => Value::Null,
            Some(None) => json!("indeterminate"),
            Some(Some(p)) => json!(p),
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Tepsi
// ---------------------------------------------------------------------------------------------

/// 3×5 piksel rakamlar (rozet sayısı).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Simgenin sağ altına renkli rozet (ve isteğe bağlı tek haneli sayı, 9'dan büyükse 9) çizer.
pub fn with_badge(rgba: &[u8], w: u32, h: u32, color: [u8; 4], count: Option<u32>) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let side = (w.min(h) * 11 / 20).max(6);
    let dot = kor_platform::taskbar::dot_rgba(side, color);
    let (ox, oy) = (w - side, h - side);
    for y in 0..side {
        for x in 0..side {
            let si = ((y * side + x) * 4) as usize;
            let a = u32::from(dot[si + 3]);
            if a == 0 {
                continue;
            }
            let di = (((oy + y) * w + ox + x) * 4) as usize;
            for c in 0..3 {
                let blended =
                    (u32::from(dot[si + c]) * a + u32::from(out[di + c]) * (255 - a)) / 255;
                out[di + c] = u8::try_from(blended).unwrap_or(255);
            }
            out[di + 3] = out[di + 3].max(dot[si + 3]);
        }
    }
    if let Some(n) = count.filter(|n| *n > 0) {
        let glyph = DIGITS[n.min(9) as usize];
        let scale = (side / 8).max(1);
        let (gw, gh) = (3 * scale, 5 * scale);
        let (gx, gy) = (ox + (side - gw) / 2, oy + (side - gh) / 2);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3u32 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = gx + col * scale + dx;
                        let py = gy + u32::try_from(row).unwrap_or(0) * scale + dy;
                        let i = ((py * w + px) * 4) as usize;
                        out[i..i + 4].copy_from_slice(&[0x1A, 0x14, 0x10, 0xFF]);
                    }
                }
            }
        }
    }
    out
}

/// §27.1: macOS menü çubuğunda tek renk şablon ikon (koyu/açık otomatik). Kaynak `assets/kor-mark-mono.svg`;
/// `assets/kor-mark-mono-tray.png` ondan üretilmiş 64 px'lik karşılığıdır (yalnız alfa kullanılır). Windows ve
/// Linux tepsisinde renkli pencere simgesi kalır (koyu görev çubuğunda siyah şablon görünmez).
const TEMPLATE_TRAY: bool = cfg!(target_os = "macos");
const MONO_TRAY: Image<'static> = tauri::include_image!("../../assets/kor-mark-mono-tray.png");

fn base_icon(app: &AppHandle) -> Option<(Vec<u8>, u32, u32)> {
    if TEMPLATE_TRAY {
        return Some((
            MONO_TRAY.rgba().to_vec(),
            MONO_TRAY.width(),
            MONO_TRAY.height(),
        ));
    }
    let i = app.default_window_icon()?;
    Some((i.rgba().to_vec(), i.width(), i.height()))
}

/// İkonu değiştirir; macOS'ta şablon bayrağı yeniden verilir (set_icon onu korumaz).
fn set_tray_icon(tray: &tauri::tray::TrayIcon<tauri::Wry>, icon: Option<Image<'static>>) {
    let _ = tray.set_icon(icon);
    let _ = tray.set_icon_as_template(TEMPLATE_TRAY);
}

fn tray_icon(app: &AppHandle, blink_off: bool) -> Option<Image<'static>> {
    let (rgba, w, h) = base_icon(app)?;
    let t = lock(&shell().tracker);
    let waiting = t.approvals.len();
    let rgba = if waiting > 0 && !blink_off {
        with_badge(&rgba, w, h, AMBER, u32::try_from(waiting).ok())
    } else if !t.sessions.is_empty() {
        with_badge(&rgba, w, h, EMBER, None)
    } else {
        rgba
    };
    Some(Image::new_owned(rgba, w, h))
}

fn apply_tray_setting(app: &AppHandle) {
    let enabled = settings().tray.enabled;
    match (enabled, app.tray_by_id(TRAY)) {
        (true, None) => {
            if let Err(e) = build_tray(app) {
                eprintln!("kor: tepsi simgesi kurulamadı: {e}");
            }
        }
        (false, Some(_)) => {
            let _ = app.remove_tray_by_id(TRAY);
        }
        _ => update_tray(app),
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app)?;
    let mut b = TrayIconBuilder::with_id(TRAY)
        .tooltip("Kor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| menu_event(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = ev
            {
                #[allow(clippy::cast_possible_truncation)] // ekran koordinatı
                toggle_popup(tray.app_handle(), (position.x as i32, position.y as i32));
            }
        });
    if let Some(icon) = tray_icon(app, false) {
        b = b.icon(icon).icon_as_template(TEMPLATE_TRAY);
    }
    b.build(app)?;
    Ok(())
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let l = l();
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        l.t("tray-open"),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "new",
        l.t("tray-new-chat"),
        true,
        None::<&str>,
    )?)?;
    let (sessions, approvals) = (sessions_with_eta(), desktop_approvals());
    let n = |v: usize| i64::try_from(v).unwrap_or(0);
    let running = Submenu::with_id(
        app,
        "running",
        l.t_args("tray-running", &args!("n" => n(sessions.len()))),
        !sessions.is_empty(),
    )?;
    for s in &sessions {
        // §28.5: tepside de geri sayım (≈, benzer turların süresinden).
        let left = s
            .eta
            .as_ref()
            .and_then(|e| e.remaining_secs.filter(|_| !e.overdue))
            .map(|r| format!(" · ≈{}:{:02}", r / 60, r % 60))
            .unwrap_or_default();
        let label = format!("{} — {}{left}", short(&s.title, 40), s.verb);
        let sub = Submenu::with_id(app, format!("run:{}", s.id), label, true)?;
        sub.append(&MenuItem::with_id(
            app,
            format!("show:{}", s.id),
            l.t("tray-open"),
            true,
            None::<&str>,
        )?)?;
        sub.append(&MenuItem::with_id(
            app,
            format!("stop:{}", s.id),
            l.t("tray-stop"),
            true,
            None::<&str>,
        )?)?;
        running.append(&sub)?;
    }
    menu.append(&running)?;
    let pending = Submenu::with_id(
        app,
        "approvals",
        l.t_args("tray-approvals", &args!("n" => n(approvals.len()))),
        !approvals.is_empty(),
    )?;
    for a in &approvals {
        let label = format!(
            "{}: {} {}",
            short(&a.title, 24),
            a.tool,
            short(&a.summary, 32)
        );
        let sub = Submenu::with_id(app, format!("q:{}", a.id), label, true)?;
        sub.append(&MenuItem::with_id(
            app,
            format!("ap:{}", a.id),
            l.t("tray-approve"),
            true,
            None::<&str>,
        )?)?;
        sub.append(&MenuItem::with_id(
            app,
            format!("dn:{}", a.id),
            l.t("tray-deny"),
            true,
            None::<&str>,
        )?)?;
        sub.append(&MenuItem::with_id(
            app,
            format!("show:{}", a.owner),
            l.t("tray-details"),
            true,
            None::<&str>,
        )?)?;
        pending.append(&sub)?;
    }
    menu.append(&pending)?;
    let recent: Vec<String> = app
        .state::<AppState>()
        .recent()
        .into_iter()
        .take(3)
        .collect();
    let rec = Submenu::with_id(app, "recent", l.t("tray-recent"), !recent.is_empty())?;
    for (i, r) in recent.iter().enumerate() {
        rec.append(&MenuItem::with_id(
            app,
            format!("recent:{i}"),
            short(r, 60),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&rec)?;
    // §31.1: eşleşmiş makineler ve yükleri (ADR 0057), dakikada bir yenilenir.
    let peers = lock(peer_loads()).clone();
    if !peers.is_empty() {
        let sub = Submenu::with_id(
            app,
            "peers",
            l.t_args("tray-peers", &args!("n" => n(peers.len()))),
            true,
        )?;
        for (i, label) in peers.iter().enumerate() {
            sub.append(&MenuItem::with_id(
                app,
                format!("peer:{i}"),
                label,
                true,
                None::<&str>,
            )?)?;
        }
        menu.append(&sub)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let pip_visible = window(app, PIP).is_some_and(|w| w.is_visible().unwrap_or(false));
    menu.append(&CheckMenuItem::with_id(
        app,
        "pip",
        l.t("tray-pip"),
        true,
        pip_visible,
        None::<&str>,
    )?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        "pause",
        l.t("tray-pause"),
        true,
        paused(),
        None::<&str>,
    )?)?;
    if let Some(v) = crate::packaging::available_version() {
        menu.append(&MenuItem::with_id(
            app,
            "update",
            l.t_args("tray-update", &args!("version" => v)),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        l.t("tray-quit"),
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn peer_loads() -> &'static Mutex<Vec<String>> {
    static P: std::sync::OnceLock<Mutex<Vec<String>>> = std::sync::OnceLock::new();
    P.get_or_init(Mutex::default)
}

/// §31.1 tepsideki makine listesi: yalnızca eşleşmiş makine varsa ve anahtar sorusuz okunabiliyorsa.
async fn refresh_peers(app: &AppHandle) {
    let has = kor_runtime::peer::home()
        .and_then(|d| kor_runtime::peer::Book::load(&d).ok())
        .is_some_and(|b| !b.peers.is_empty());
    if !has {
        if !lock(peer_loads()).is_empty() {
            lock(peer_loads()).clear();
            refresh(app);
        }
        return;
    }
    let cfg = crate::api::root(&app.state::<AppState>())
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map(|l| l.config.vault)
        .unwrap_or_default();
    let Ok(store) = kor_runtime::secure::vault_store(&cfg, None) else {
        return;
    };
    let Ok(node) = kor_runtime::peer::local_node(store.as_ref()) else {
        return;
    };
    let l = l();
    let rows: Vec<String> = kor_runtime::peer::client::loads(&node)
        .await
        .into_iter()
        .map(|(p, r)| match r {
            Ok(ld) => l.t_args(
                "tray-peer-load",
                &args!("name" => p.name, "tasks" => ld.tasks, "cpus" => ld.cpus,
                       "busy" => ld.busy.map_or_else(|| "-".to_owned(), |b| format!("{:.0}%", b * 100.0))),
            ),
            Err(_) => l.t_args("tray-peer-unreachable", &args!("name" => p.name)),
        })
        .collect();
    if *lock(peer_loads()) != rows {
        *lock(peer_loads()) = rows;
        refresh(app);
    }
}

fn short(s: &str, n: usize) -> String {
    kor_platform::toast::clip(s.lines().next().unwrap_or_default(), n)
}

/// One tooltip line per provider with the most used key's share, from the rate-limit headers
/// this process has seen (nothing until the first request).
fn quota_lines(l: &Localizer) -> Vec<String> {
    let mut worst: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for k in kor_providers::quota::snapshot() {
        if let Some(s) = k.used_share() {
            let e = worst.entry(k.provider.clone()).or_insert(0.0);
            *e = e.max(s);
        }
    }
    worst
        .into_iter()
        .map(|(provider, s)| {
            #[allow(clippy::cast_possible_truncation)] // a 0–100 percentage
            let percent = (s * 100.0).round().clamp(0.0, 100.0) as i64;
            l.t_args(
                "tray-tooltip-quota",
                &args!("provider" => provider, "percent" => percent),
            )
        })
        .collect()
}

fn update_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    let l = l();
    let (running, waiting) = {
        let t = lock(&shell().tracker);
        (t.sessions.len(), t.approvals.len())
    };
    let n = |v: usize| i64::try_from(v).unwrap_or(0);
    let tip = if waiting > 0 {
        l.t_args("tray-tooltip-waiting", &args!("n" => n(waiting)))
    } else if running > 0 {
        l.t_args("tray-tooltip-working", &args!("n" => n(running)))
    } else {
        l.t("tray-tooltip-idle")
    };
    let tip = [tip]
        .into_iter()
        .chain(quota_lines(&l))
        .collect::<Vec<_>>()
        .join("\n");
    let _ = tray.set_tooltip(Some(&tip));
    set_tray_icon(&tray, tray_icon(app, false));
    if let Ok(menu) = tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Onay bekliyor ve kullanıcı pencerede değil: rozet saniyede bir yanıp söner.
fn blink_tray(app: &AppHandle) {
    let waiting = !lock(&shell().tracker).approvals.is_empty();
    let s = shell();
    if !waiting || main_active(app) {
        if s.blink.swap(false, Ordering::Relaxed)
            && let Some(t) = app.tray_by_id(TRAY)
        {
            set_tray_icon(&t, tray_icon(app, false));
        }
        return;
    }
    let off = !s.blink.fetch_xor(true, Ordering::Relaxed);
    if let Some(t) = app.tray_by_id(TRAY) {
        set_tray_icon(&t, tray_icon(app, off));
    }
}

fn menu_event(app: &AppHandle, id: &str) {
    let app = app.clone();
    match id {
        "open" => show_main(&app),
        "new" => {
            show_main(&app);
            let _ = app.emit("kor:shellui/newChat", ());
        }
        "pip" => toggle_pip(&app),
        "pause" => set_paused(&app, !paused()),
        "update" => {
            show_main(&app);
            let _ = app.emit("kor:shellui/openUpdate", ());
        }
        "quit" => app.exit(0),
        _ => {
            if id.starts_with("peer:") {
                show_main(&app);
            } else if let Some(s) = id.strip_prefix("show:") {
                open_target(&app, Some(s), false);
            } else if let Some(s) = id.strip_prefix("stop:") {
                app.state::<AppState>().chats.abort(s);
                mark_dirty(&app);
            } else if let Some(i) = id
                .strip_prefix("recent:")
                .and_then(|i| i.parse::<usize>().ok())
            {
                if let Some(path) = app.state::<AppState>().recent().into_iter().nth(i) {
                    show_main(&app);
                    let _ = app.emit("kor:shellui/openProject", path);
                }
            } else if let Some((approve, aid)) = id
                .strip_prefix("ap:")
                .map(|a| (true, a))
                .or_else(|| id.strip_prefix("dn:").map(|a| (false, a)))
            {
                let aid = aid.to_owned();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = answer(&app, &aid, approve).await {
                        eprintln!("kor: onay yanıtlanamadı: {e}");
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Görev çubuğu
// ---------------------------------------------------------------------------------------------

fn update_taskbar(app: &AppHandle) {
    let Some(w) = window(app, MAIN) else {
        return;
    };
    let (progress, waiting) = {
        let t = lock(&shell().tracker);
        (t.progress(), t.approvals.len())
    };
    let failed = shell().failed.load(Ordering::Relaxed);
    let state = match (progress, waiting > 0, failed) {
        (_, true, _) => ProgressBarState {
            status: Some(ProgressBarStatus::Paused),
            progress: Some(progress.flatten().map_or(100, u64::from)),
        },
        (Some(Some(p)), false, _) => ProgressBarState {
            status: Some(ProgressBarStatus::Normal),
            progress: Some(u64::from(p)),
        },
        (Some(None), false, _) => ProgressBarState {
            status: Some(ProgressBarStatus::Indeterminate),
            progress: None,
        },
        (None, false, true) => ProgressBarState {
            status: Some(ProgressBarStatus::Error),
            progress: Some(100),
        },
        (None, false, false) => ProgressBarState {
            status: Some(ProgressBarStatus::None),
            progress: None,
        },
    };
    let _ = w.set_progress_bar(state);
    // macOS Dock / Linux başlatıcı sayı rozeti; Windows'ta desteklenmez, orada bindirme noktası çizilir.
    let _ = w.set_badge_count((waiting > 0).then(|| i64::try_from(waiting).unwrap_or(i64::MAX)));
    let running = progress.is_some();
    let color = if waiting > 0 {
        Some(AMBER)
    } else if running {
        Some(EMBER)
    } else {
        None
    };
    if let Some(hwnd) = hwnd(&w) {
        let desc = if waiting > 0 {
            "waiting"
        } else if running {
            "working"
        } else {
            ""
        };
        let desc = desc.to_owned();
        let _ = app.run_on_main_thread(move || {
            let _ = kor_platform::taskbar::set_overlay(hwnd, color, &desc);
        });
    }
}

/// Win32 HWND (yalnızca Windows'ta vardır; diğer sistemlerde `None`).
fn hwnd(w: &WebviewWindow) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// PiP ve tepsi açılır penceresi
// ---------------------------------------------------------------------------------------------

fn monitors(app: &AppHandle) -> Vec<Monitor> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Monitor {
            name: m.name().cloned().unwrap_or_default(),
            x: m.position().x,
            y: m.position().y,
            w: m.size().width,
            h: m.size().height,
            scale: m.scale_factor(),
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)] // ekran koordinatı
fn cursor(app: &AppHandle) -> (i32, i32) {
    app.cursor_position()
        .map(|p| (p.x as i32, p.y as i32))
        .unwrap_or((0, 0))
}

fn scale_at(mons: &[Monitor], p: (i32, i32)) -> f64 {
    layout::monitor_at(mons, p.0, p.1).map_or(1.0, |m| m.scale.max(1.0))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // küçük pencere boyutu
fn physical(size: (f64, f64), scale: f64) -> (u32, u32) {
    (
        (size.0 * scale).round() as u32,
        (size.1 * scale).round() as u32,
    )
}

fn ensure_window(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = window(app, label) {
        return Some(w);
    }
    let (w, h) = if label == PIP { PIP_SIZE } else { POPUP_SIZE };
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Kor")
        .inner_size(w, h)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .transparent(true)
        .shadow(label == POPUP)
        .focused(label == POPUP)
        .visible(false)
        .build();
    match built {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("kor: {label} penceresi açılamadı: {e}");
            None
        }
    }
}

fn show_pip(app: &AppHandle) {
    let Some(w) = ensure_window(app, PIP) else {
        return;
    };
    let mons = monitors(app);
    let cur = cursor(app);
    let size = physical(PIP_SIZE, scale_at(&mons, cur));
    let saved = Layouts::standard().and_then(|l| l.get(&mons, PIP));
    if let Some((x, y)) = layout::pip_position(saved, &mons, cur, size) {
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = w.show();
    let _ = w.set_always_on_top(true);
    shell().pip_suppressed.store(false, Ordering::Relaxed);
    mark_dirty(app);
}

fn hide_pip(app: &AppHandle) {
    if let Some(w) = window(app, PIP) {
        let _ = w.hide();
    }
    mark_dirty(app);
}

/// Tepsi menüsü, komut paleti, `shellui.pip`.
pub fn toggle_pip(app: &AppHandle) {
    let visible = window(app, PIP).is_some_and(|w| w.is_visible().unwrap_or(false));
    shell().pip_manual.store(!visible, Ordering::Relaxed);
    if visible {
        hide_pip(app);
    } else {
        show_pip(app);
    }
}

pub fn set_pip(app: &AppHandle, show: bool) {
    shell().pip_manual.store(show, Ordering::Relaxed);
    if show {
        show_pip(app);
    } else {
        hide_pip(app);
    }
}

/// `pip.auto_show`: görev başlayınca çıkar, son görev bittikten 5 sn sonra kaybolur.
fn pip_auto(app: &AppHandle) {
    if !settings().pip.auto_show || shell().pip_manual.load(Ordering::Relaxed) {
        return;
    }
    let busy = !lock(&shell().tracker).sessions.is_empty();
    let visible = window(app, PIP).is_some_and(|w| w.is_visible().unwrap_or(false));
    if busy && !visible && !shell().pip_suppressed.load(Ordering::Relaxed) {
        show_pip(app);
    } else if !busy && visible {
        let g = shell().pip_hide_gen.fetch_add(1, Ordering::Relaxed) + 1;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let still_idle = lock(&shell().tracker).sessions.is_empty();
            if still_idle
                && shell().pip_hide_gen.load(Ordering::Relaxed) == g
                && !shell().pip_manual.load(Ordering::Relaxed)
            {
                hide_pip(&app);
            }
        });
    }
}

/// `pip.hide_fullscreen`: başka bir uygulama tam ekranken PiP gizlenir, çıkınca geri gelir.
async fn fullscreen_check(app: &AppHandle) {
    if !settings().pip.hide_fullscreen {
        return;
    }
    let Some(w) = window(app, PIP) else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false);
    let suppressed = shell().pip_suppressed.load(Ordering::Relaxed);
    if !visible && !suppressed {
        return;
    }
    let full =
        tauri::async_runtime::spawn_blocking(kor_platform::foreground::fullscreen_app_active)
            .await
            .ok()
            .flatten();
    match full {
        Some(true) if visible => {
            let _ = w.hide();
            shell().pip_suppressed.store(true, Ordering::Relaxed);
        }
        Some(false) if suppressed => {
            shell().pip_suppressed.store(false, Ordering::Relaxed);
            let _ = w.show();
        }
        _ => {}
    }
}

/// Sol tık: tepsinin yanında açılır pencere (çalışanlar, onaylar, hızlı görev, son projeler).
fn toggle_popup(app: &AppHandle, at: (i32, i32)) {
    let Some(w) = ensure_window(app, POPUP) else {
        return;
    };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    let mons = monitors(app);
    let (pw, ph) = physical(POPUP_SIZE, scale_at(&mons, at));
    if let Some(m) = layout::monitor_at(&mons, at.0, at.1) {
        let max_x = i64::from(m.x) + i64::from(m.w) - i64::from(pw);
        let max_y = i64::from(m.y) + i64::from(m.h) - i64::from(ph);
        let x =
            (i64::from(at.0) - i64::from(pw) / 2).clamp(i64::from(m.x), max_x.max(i64::from(m.x)));
        // Görev çubuğu altta: imlecin üstüne; üstte: altına.
        let y = if i64::from(at.1) > i64::from(m.y) + i64::from(m.h) / 2 {
            i64::from(at.1) - i64::from(ph) - 8
        } else {
            i64::from(at.1) + 8
        };
        let y = y.clamp(i64::from(m.y), max_y.max(i64::from(m.y)));
        let _ = w.set_position(tauri::PhysicalPosition::new(
            i32::try_from(x).unwrap_or(m.x),
            i32::try_from(y).unwrap_or(m.y),
        ));
    }
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit("kor:shellui/changed", ());
}

pub fn hide_popup(app: &AppHandle) {
    if let Some(w) = window(app, POPUP) {
        let _ = w.hide();
    }
}

// ---------------------------------------------------------------------------------------------
// Pencere olayları ve konumlar
// ---------------------------------------------------------------------------------------------

/// `lib.rs` `on_window_event`.
pub fn window_event(w: &tauri::Window, ev: &tauri::WindowEvent) {
    let app = w.app_handle();
    match (w.label(), ev) {
        (MAIN, tauri::WindowEvent::CloseRequested { api, .. }) => {
            let s = settings();
            if s.tray.enabled && s.tray.close_to_tray && app.tray_by_id(TRAY).is_some() {
                api.prevent_close();
                let _ = w.hide();
            }
        }
        (MAIN, tauri::WindowEvent::Focused(f)) => {
            shell().main_focused.store(*f, Ordering::Relaxed);
            if *f {
                activity();
                shell().failed.store(false, Ordering::Relaxed);
                update_taskbar(app);
            }
        }
        (MAIN | PIP, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) => {
            lock(&shell().layouts_pending).insert(w.label().to_owned());
        }
        (POPUP, tauri::WindowEvent::Focused(false)) => {
            let _ = w.hide();
        }
        _ => {}
    }
}

fn save_layouts(app: &AppHandle) {
    let labels: Vec<String> = std::mem::take(&mut *lock(&shell().layouts_pending))
        .into_iter()
        .collect();
    if labels.is_empty() {
        return;
    }
    let Some(store) = Layouts::standard() else {
        return;
    };
    let mons = monitors(app);
    for label in labels {
        let Some(w) = window(app, &label) else {
            continue;
        };
        if w.is_maximized().unwrap_or(false)
            || w.is_minimized().unwrap_or(false)
            || !w.is_visible().unwrap_or(false)
        {
            continue;
        }
        let (Ok(p), Ok(s)) = (w.outer_position(), w.outer_size()) else {
            continue;
        };
        let _ = store.save(
            &mons,
            &label,
            Rect {
                x: p.x,
                y: p.y,
                w: s.width,
                h: s.height,
            },
        );
    }
}

/// Ana pencere bu monitör düzeninde son kapatıldığı yerde açılır; o monitör yoksa varsayılan konum.
fn restore_main(app: &AppHandle) {
    let Some(w) = window(app, MAIN) else {
        return;
    };
    let mons = monitors(app);
    let Some(saved) = Layouts::standard().and_then(|l| l.get(&mons, MAIN)) else {
        return;
    };
    if let Some(r) = layout::restore(saved, &mons) {
        let _ = w.set_size(tauri::PhysicalSize::new(r.w, r.h));
        let _ = w.set_position(tauri::PhysicalPosition::new(r.x, r.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_is_drawn_bottom_right_with_a_digit() {
        let (w, h) = (32u32, 32u32);
        let base = vec![0u8; (w * h * 4) as usize];
        let out = with_badge(&base, w, h, AMBER, Some(3));
        let px = |x: u32, y: u32| &out[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(px(0, 0)[3], 0, "top-left untouched");
        assert!(px(w - 4, h - 9)[3] > 0, "badge area painted");
        // Rakam koyu renkle çizilir: rozetin içinde en az bir koyu piksel var.
        let side = w * 11 / 20;
        let dark =
            (h - side..h).any(|y| (w - side..w).any(|x| px(x, y)[..3] == [0x1A, 0x14, 0x10]));
        assert!(dark);
        let plain = with_badge(&base, w, h, EMBER, None);
        let dark =
            (h - side..h).any(|y| (w - side..w).any(|x| plain[((y * w + x) * 4) as usize] == 0x1A));
        assert!(!dark, "no digit without a count");
    }

    #[test]
    fn short_takes_the_first_line() {
        assert_eq!(short("a\nb", 10), "a");
        assert_eq!(short("abcdef", 4), "abc…");
    }

    #[test]
    fn physical_size_scales() {
        assert_eq!(physical(PIP_SIZE, 1.5), (360, 108));
    }
}
