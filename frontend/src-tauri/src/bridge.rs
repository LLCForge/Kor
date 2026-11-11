//! `window.kor` köprüsünün Rust tarafı. Arayüzdeki her çağrı tek `kor_call` komutuna gelir
//! (`kor.git.status(a, b)` → `method = "git.status"`, `args = [a, b]`) ve buradan yönlendirilir.
//! Olaylar `kor:<yol>` adıyla yayılır: `kor:event`, `kor:g4/event`… (bkz. renderer/kor-bridge.ts).

use std::path::PathBuf;

use kor_core::PermissionMode;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::state::AppState;
use crate::store;

type Res = Result<Value, String>;

/// Every call goes through here; with `KOR_IPC_TRACE` / `core.ipcTrace` on, the call, its result or
/// error and its duration are written to the IPC trace (§17.2, ADR 0061).
#[tauri::command]
pub async fn kor_call(
    method: String,
    args: Vec<Value>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res {
    use kor_core::ipc_trace::{self as trace, Kind, Surface};
    if !trace::enabled() {
        return dispatch(method, args, app, state).await;
    }
    trace::record(Surface::Desktop, Kind::Call, &method, None, &Value::Array(args.clone()));
    let started = std::time::Instant::now();
    let name = method.clone();
    let out = dispatch(method, args, app, state).await;
    match &out {
        Ok(v) => trace::record(Surface::Desktop, Kind::Result, &name, Some(started.elapsed()), v),
        Err(e) => trace::record(Surface::Desktop, Kind::Error, &name, Some(started.elapsed()), &Value::String(e.clone())),
    }
    out
}

/// Every `kor:*` event in the IPC schema is also written to the trace (`kor:event` is traced where the
/// chat emits it, with its session). Listeners are cheap no-ops while tracing is off.
pub fn trace_events(app: &AppHandle) {
    use tauri::Listener;
    for name in kor_core::ipc::events() {
        if name == "kor:event" {
            continue;
        }
        let traced = name.clone();
        app.listen_any(name, move |e| {
            if kor_core::ipc_trace::enabled() {
                let payload = serde_json::from_str(e.payload()).unwrap_or(Value::Null);
                kor_core::ipc_trace::event(kor_core::ipc_trace::Surface::Desktop, &traced, &payload);
            }
        });
    }
}

async fn dispatch(
    method: String,
    args: Vec<Value>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res {
    let arg = |i: usize| -> Result<&str, String> {
        args.get(i)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("`{method}`: {}. bağımsız değişken metin olmalı", i + 1))
    };
    match method.as_str() {
        "getState" => {
            // Açılışta: açık (ya da varsayılan) klasördeki eski sohbetlerin medyası da görünsün.
            if let Ok(ws) = state.workspace() {
                allow_media(&app, &ws);
            }
            get_state(&state)
        }
        "listSessions" => Ok(store::sessions_dir().map_or(json!([]), |d| store::list_sessions(&d))),
        "openSession" | "loadSessionView" => match state.chats.transcript(arg(0)?) {
            Some(t) => Ok(t),
            None => store::open_session(&sessions_dir()?, arg(0)?),
        },
        "deleteSession" => {
            state.chats.forget(arg(0)?);
            store::delete_session(&sessions_dir()?, arg(0)?).map(|()| Value::Null)
        }
        // shortcut: spinOff ("dal") da düz kopyadır; dökümde başlık alanı yok, ad ilk mesajdan gelir.
        "duplicateSession" => {
            store::duplicate_session(&sessions_dir()?, arg(0)?).map(Value::String)
        }
        "liveSessions" => Ok(state.chats.live()),
        // Bildirim yönlendirme ipucu; masaüstü bildirimleri gelene kadar tutulacak bir şey yok.
        "setActiveSession" => Ok(Value::Null),
        "listModels" => Ok(list_models()),
        "listDir" => store::list_dir(&root(&state)?, arg(0)?),
        "readFile" => store::read_file(&root(&state)?, arg(0)?).map(Value::String),
        "setWorkspacePath" => {
            let shown = set_workspace(&state, PathBuf::from(arg(0)?))?;
            if let Some(root) = state.root() {
                allow_media(&app, &root);
            }
            Ok(shown)
        }
        "openWorkspace" => open_workspace(&app, &state).await,
        "clearWorkspace" => {
            state.close();
            Ok(Value::Null)
        }
        "openExternal" => open_external(&app, arg(0)?),
        "send" => {
            let strings = |v: Option<&Value>| -> Vec<String> {
                v.and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            crate::chat::send(
                &app,
                crate::chat::SendArgs {
                    session_id: arg(0)?.to_owned(),
                    text: arg(1)?.to_owned(),
                    attachments: strings(args.get(2)),
                    open_file: args.get(3).and_then(Value::as_str).map(str::to_owned),
                },
            )
            .map(|()| Value::Null)
        }
        "g2Send" => {
            let payload = args.get(1).ok_or("`g2Send`: içerik eksik")?;
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // §10 (ADR 0048): görseller yerel OCR ile metne çevrilip mesaja eklenir (LLM'siz); model
            // görüntünün kendisini almaz. Okunamayan görsel gönderimi durdurur, taslak geri konur.
            let images: Vec<(String, String)> = payload
                .get("images")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|i| {
                            let field = |k: &str| i.get(k).and_then(Value::as_str).unwrap_or_default();
                            (field("name").to_owned(), field("dataUrl").to_owned())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let text = kor_runtime::visual::with_pasted_images(
                text,
                &images,
                &kor_runtime::visual::SystemOcr,
                &kor_providers::CancellationToken::new(),
            )
            .await
            .map_err(|e| format!("Görsel metne çevrilemedi: {e}"))?;
            let text = text.as_str();
            let attachments = payload
                .get("attachments")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let interrupt = payload
                .get("interrupt")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            crate::chat::g2_send(&app, arg(0)?, text, attachments, interrupt)
        }
        "g2QueueRemove" => Ok(Value::Bool(crate::chat::queue_remove(
            &app,
            arg(0)?,
            arg(1)?,
        ))),
        "g2QueueClear" => Ok(json!(crate::chat::queue_clear(&app, arg(0)?))),
        "g2QueueFlush" => crate::chat::queue_flush(&app, arg(0)?).map(|()| Value::Null),
        "abort" => {
            state.chats.abort(arg(0)?);
            Ok(Value::Null)
        }
        "approve" => state
            .chats
            .approve(arg(0)?, arg(1)?, args.get(2))
            .map(|()| Value::Null),
        // Interactive cards: the event draft's Save/Cancel answers the tool's permission request.
        "g3Answer" => {
            let answer = args.get(2).ok_or("Yanıt eksik")?;
            match answer["kind"].as_str() {
                Some("draft") => {
                    let yes = answer["confirmed"].as_bool().unwrap_or(false);
                    state
                        .chats
                        .approve(arg(1)?, if yes { "yes" } else { "no" }, None)
                        .map(|()| Value::Null)
                }
                _ => Err("Bu etkileşim türü henüz desteklenmiyor".into()),
            }
        }
        "setModel" => {
            let model = arg(0)?;
            model
                .parse::<kor_core::ModelRef>()
                .map_err(|_| format!("Model `sağlayıcı/model` biçiminde olmalı: {model}"))?;
            state.set_model(model.to_owned());
            Ok(Value::Null)
        }
        // shortcut: varsayılan mod da yalnızca bu oturum için tutulur; kor-config'e yazma ayarlar fazında gelecek.
        "setPermissionMode" | "setDefaultPermissionMode" => {
            state.set_mode(from_ui_mode(arg(0)?)?);
            Ok(Value::Null)
        }
        "setUiLanguage" => set_ui_language(arg(0)?),
        "g27ListRuntimes" => tauri::async_runtime::spawn_blocking(crate::runtimes::list)
            .await
            .map_err(|e| e.to_string()),
        "g27InstallRuntime" => {
            let (app, id) = (app.clone(), arg(0)?.to_owned());
            tauri::async_runtime::spawn_blocking(move || crate::runtimes::install(&app, &id))
                .await
                .map_err(|e| e.to_string())?
        }
        _ => crate::api::call(&method, &args, &app, &state).await,
    }
}

fn sessions_dir() -> Result<PathBuf, String> {
    store::sessions_dir().ok_or_else(|| "Kullanıcı veri klasörü bulunamadı".into())
}

fn root(state: &AppState) -> Result<PathBuf, String> {
    state.root().ok_or_else(|| "Önce bir klasör aç".into())
}

/// Arka uçtaki adlar arayüzün `PERMISSION_MODES` kümesine çevrilir.
pub(crate) fn ui_mode(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Default => "default",
        PermissionMode::Plan => "plan",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Auto => "auto",
        PermissionMode::BypassPermissions => "bypass",
        // Arayüzde karşılığı yok; en yakın güvenli gösterim, her işlemde soran varsayılan mod.
        PermissionMode::DontAsk => "default",
    }
}

pub(crate) fn from_ui_mode(m: &str) -> Result<PermissionMode, String> {
    Ok(match m {
        "default" => PermissionMode::Default,
        "plan" => PermissionMode::Plan,
        "acceptEdits" => PermissionMode::AcceptEdits,
        "auto" => PermissionMode::Auto,
        "bypass" => PermissionMode::BypassPermissions,
        other => return Err(format!("Bilinmeyen izin modu: {other}")),
    })
}

/// Kökteki proje ayarlarıyla birlikte etkin yapılandırma (CLI ile aynı katmanlar).
pub fn load_config(root: &std::path::Path) -> Result<kor_config::Loaded, String> {
    let loaded = kor_config::load(&kor_config::Paths::discover(root), std::env::vars(), &[])
        .map_err(|e| format!("Yapılandırma okunamadı: {e}"))?;
    // Gizlilik/çevrimdışı değişikliği bir sonraki okumada tüm HTTP istemcilerine yansır (ADR 0035).
    kor_runtime::apply_network_policy(
        &kor_i18n::Localizer::new(loaded.config.ui.language.resolve()),
        &loaded.config,
    );
    Ok(loaded)
}

fn get_state(state: &AppState) -> Res {
    let root = state.root();
    let cwd = root
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let loaded = load_config(&cwd)?;
    let default_mode = ui_mode(loaded.config.permissions.mode);
    let mode = state.mode().map_or(default_mode, ui_mode);
    Ok(json!({
        "root": root.map(|r| r.to_string_lossy().into_owned()),
        "model": state.model().unwrap_or(loaded.config.agent.model),
        // Anahtarlar sağlayıcı başına `kor auth` ile işletim sistemi anahtarlığında tutulur; tek anahtarlık
        // "API anahtarı gir" ekranı Kor'un çoklu sağlayıcı modeline uymaz, bu yüzden gösterilmez.
        "hasApiKey": true,
        "permissionMode": mode,
        "defaultPermissionMode": default_mode,
        "runningSessionIds": state.chats.running_ids(),
        // Arayüz dili: etkin olan (otomatikse işletim sisteminden) ve kullanıcının seçimi.
        "uiLanguage": loaded.config.ui.language.resolve().code(),
        "uiLanguagePref": serde_json::to_value(loaded.config.ui.language).unwrap_or(Value::Null),
        "recentWorkspaces": state.recent(),
    }))
}

/// `kor models` önbelleğindeki modeller (`sağlayıcı/model`). Ağ isteği yapmaz.
fn list_models() -> Value {
    let Some(path) = dirs::cache_dir().map(|d| d.join("kor").join("models.json")) else {
        return json!([]);
    };
    let catalog = kor_providers::catalog::Catalog::load(&path);
    let models: Vec<Value> = catalog
        .providers
        .iter()
        .flat_map(|(provider, entry)| {
            entry.models.iter().map(move |m| {
                json!({
                    "id": format!("{provider}/{}", m.id),
                    "name": m.display_name.clone().unwrap_or_else(|| m.id.clone()),
                    // shortcut: katalog araç desteğini taşımıyor; ajan araçsız modeli metin protokolüne düşürür.
                    "supportsTools": true,
                    "uncensored": false,
                })
            })
        })
        .collect();
    Value::Array(models)
}

fn set_workspace(state: &AppState, dir: PathBuf) -> Res {
    if !dir.is_dir() {
        return Err(format!("Klasör bulunamadı: {}", dir.display()));
    }
    // Araçlar kökü kanonik yolla karşılaştırır; göreli/kısa yol kökten kaçış denetimini yanıltmasın.
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let shown = dir.to_string_lossy().into_owned();
    state.open(dir);
    Ok(Value::String(shown))
}

async fn open_workspace(app: &AppHandle, state: &AppState) -> Res {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    // Diyalog kendi iş parçacığında döner; beklemek eşzamansız çalışanı bloklamasın diye ayrı iş parçacığında.
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| e.to_string())?;
    match picked.and_then(|p| p.into_path().ok()) {
        Some(dir) => {
            let shown = set_workspace(state, dir)?;
            if let Some(root) = state.root() {
                allow_media(app, &root);
            }
            Ok(shown)
        }
        None => Ok(Value::Null),
    }
}

/// Yalnızca web bağlantıları açılır; `file:`/özel şemalar arayüzden program çalıştırmaya dönüşmesin.
fn open_external(app: &AppHandle, url: &str) -> Res {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Yalnızca http(s) bağlantıları açılabilir".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map(|()| Value::Null)
        .map_err(|e| e.to_string())
}

/// Arayüz ve arka uç mesajlarının dili; CLI ile aynı ayar (`ui.language` kullanıcı `config.toml`).
/// `auto`: işletim sisteminin dili. Yorumlar ve diğer ayarlar korunur.
fn set_ui_language(code: &str) -> Res {
    if !matches!(code, "auto" | "tr" | "en" | "fr") {
        return Err(format!("Desteklenmeyen dil: {code}"));
    }
    let file = kor_config::user_dir()
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?
        .join("config.toml");
    kor_config::set_string(&file, "ui.language", code).map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

/// Arayüzün üretilen medyayı gösterebilmesi için asset protokolüne yalnızca `<kök>/kor-media` açılır
/// (tauri.conf.json'da kapsam boş başlar). Proje dosyalarının geri kalanı bu yoldan okunamaz.
pub fn allow_media(app: &AppHandle, root: &std::path::Path) {
    let dir = root.join(kor_agent::media::MEDIA_DIR);
    if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
        eprintln!(
            "kor: medya klasörüne izin verilemedi ({}): {e}",
            dir.display()
        );
    }
}
