//! Ad alanları: bkz. `ns/mod.rs`. `chromeExtension`: Kor tarayıcı eklentisi (§18, ADR 0058).
//!
//! - Köprü ve yan panel istemcisi: `crate::chrome_bridge`.
//! - Arayüz yöntemleri: `info`, `openFolder`, `pair`, `revoke`, `domains`, `allow`, `deny`,
//!   `forget`, `install`, `uninstall`, `profiles`, `profileGet`, `profileSet`, `profileRemove`,
//!   `docsCache`, `docsCacheClear`, `setBackend`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kor_platform::native_messaging::{self as nm, Browser};
use kor_runtime::browser_assist::{docs, forms};
use kor_tools::chrome::{pairing, policy};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::api::Res;
use crate::state::AppState;


/// Yüklü uygulamada kaynak klasöründeki `extension/<chrome|firefox>`, geliştirmede depodaki
/// `frontend/extension/dist/<chrome|firefox>`.
fn extension_dir(app: &AppHandle, flavour: &str) -> PathBuf {
    let dev = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../extension/dist")).join(flavour);
    app.path()
        .resource_dir()
        .ok()
        .map(|d| d.join("extension").join(flavour))
        .into_iter()
        .chain([dev])
        .find(|d| d.join("manifest.json").is_file())
        .map(|d| dunce_path(&d))
        .unwrap_or_default()
}

/// Kullanıcıya gösterilecek yol: `..` çözülmüş, `\\?\` öneki yok.
fn dunce_path(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    PathBuf::from(kor_tools::display_path(&c))
}

fn state_dir() -> Result<PathBuf, String> {
    kor_tools::chrome::state_dir().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn info(app: &AppHandle) -> Res {
    let bridge = kor_tools::chrome::global();
    let store = kor_secrets::KeychainStore;
    let hosts: BTreeMap<&str, bool> = nm::Env::system()
        .map(|env| nm::ALL.iter().map(|b| (b.id(), nm::installed(&env, *b))).collect())
        .unwrap_or_default();
    let backend = crate::bridge::load_config(&app.state::<AppState>().workspace()?)
        .map(|l| format!("{:?}", l.config.browser.backend).to_lowercase())
        .unwrap_or_else(|_| "auto".into());
    Ok(json!({
        "running": bridge.is_some(),
        "connected": bridge.as_ref().is_some_and(|b| b.connected()),
        "connections": bridge.as_ref().map_or(0, |b| b.connections()),
        "port": kor_tools::chrome::PORT,
        "paired": pairing::paired_count(&store),
        "codeWaiting": state_dir().map(|d| pairing::code_pending(&d)).unwrap_or(false),
        "hosts": hosts,
        "domains": policy::current(),
        "backend": backend,
        "extensionPath": extension_dir(app, "chrome").to_string_lossy(),
        "firefoxPath": extension_dir(app, "firefox").to_string_lossy(),
        "extensionId": nm::CHROME_EXTENSION_ID,
    }))
}

fn arg<'a>(args: &'a [Value], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("{what} eksik"))
}

fn domains(f: impl FnOnce(&mut policy::Domains) -> Result<(), String>) -> Res {
    let file = policy::file().ok_or("Ayar klasörü bulunamadı")?;
    let mut d = policy::Domains::load(&file);
    f(&mut d)?;
    d.save(&file)?;
    Ok(json!(d))
}

/// Paketlenmiş `kor` (masaüstünün yanında), yoksa PATH'teki.
fn cli_exe() -> Option<PathBuf> {
    crate::kor_cli::find()
}

fn install(app: &AppHandle, args: &[Value]) -> Res {
    let env = nm::Env::system().ok_or("Kor veri klasörü bulunamadı")?;
    let kor = cli_exe().ok_or("Kor CLI bulunamadı; önce Ayarlar > CLI'yı PATH'e ekle")?;
    let list: Vec<Browser> = match args.first().and_then(Value::as_str) {
        None | Some("all") => nm::ALL.to_vec(),
        Some(b) => vec![Browser::parse(b).ok_or_else(|| format!("Bilinmeyen tarayıcı: {b}"))?],
    };
    let mut done = Vec::new();
    for b in list {
        let i = nm::install(&env, b, &kor, &[]).map_err(|e| format!("{}: {e}", b.id()))?;
        done.push(json!({ "browser": b.id(), "manifest": i.manifest }));
    }
    if args.get(1).and_then(Value::as_bool) == Some(true) {
        let dir = extension_dir(app, "chrome");
        if !dir.as_os_str().is_empty() {
            let _ = app.opener().open_path(dir.to_string_lossy(), None::<&str>);
        }
    }
    Ok(json!({ "installed": done }))
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(_ns: &str, name: &str, args: &[Value], app: &AppHandle, _state: &AppState) -> Option<Res> {
    let store = kor_secrets::KeychainStore;
    Some(match name {
        "info" => info(app),
        "openFolder" => {
            let dir = extension_dir(app, args.first().and_then(Value::as_str).unwrap_or("chrome"));
            if dir.as_os_str().is_empty() {
                return Some(Err("Eklenti derlemesi bulunamadı (npm run build:extension).".into()));
            }
            app.opener()
                .open_path(dir.to_string_lossy(), None::<&str>)
                .map(|()| Value::Null)
                .map_err(|e| e.to_string())
        }
        "pair" => state_dir()
            .and_then(|d| pairing::new_code(&d))
            .map(|(code, expires)| json!({ "code": code, "expires": expires })),
        "revoke" => state_dir().and_then(|d| pairing::revoke_all(&d, &store)).map(|()| Value::Null),
        "domains" => Ok(json!(policy::current())),
        "allow" => match arg(args, 0, "Alan adı") {
            Ok(d) => domains(|x| x.allow(d).map(drop)),
            Err(e) => Err(e),
        },
        "deny" => match arg(args, 0, "Alan adı") {
            Ok(d) => domains(|x| x.deny(d).map(drop)),
            Err(e) => Err(e),
        },
        "forget" => match arg(args, 0, "Alan adı") {
            Ok(d) => domains(|x| x.forget(d).map(drop)),
            Err(e) => Err(e),
        },
        "install" => install(app, args),
        "uninstall" => nm::Env::system().ok_or_else(|| "Kor veri klasörü bulunamadı".to_owned()).and_then(|env| {
            let mut removed = Vec::new();
            for b in nm::ALL {
                if nm::uninstall(&env, *b).map_err(|e| e.to_string())? {
                    removed.push(b.id());
                }
            }
            Ok(json!({ "removed": removed }))
        }),
        "setBackend" => match arg(args, 0, "Arka uç") {
            Ok(b @ ("auto" | "chrome" | "headless")) => kor_config::user_dir()
                .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".to_owned())
                .and_then(|d| kor_config::set_string(&d.join("config.toml"), "browser.backend", b).map_err(|e| e.to_string()))
                .map(|()| Value::Null),
            Ok(other) => Err(format!("Bilinmeyen arka uç: {other}")),
            Err(e) => Err(e),
        },
        "profiles" => Ok(json!(forms::list(&store))),
        "profileGet" => arg(args, 0, "Profil").and_then(|n| forms::get(&store, n)).map(|p| json!(p)),
        "profileSet" => arg(args, 0, "Profil").and_then(|n| {
            let values: BTreeMap<String, String> =
                serde_json::from_value(args.get(1).cloned().unwrap_or_default()).map_err(|_| "Profil değerleri geçersiz".to_owned())?;
            forms::set(&store, n, &values).map(|()| Value::Null)
        }),
        "profileRemove" => arg(args, 0, "Profil").and_then(|n| forms::remove(&store, n)).map(|b| json!(b)),
        "docsCache" => Ok(docs::cache_dir()
            .map(|d| {
                docs::cache_list(&d)
                    .into_iter()
                    .map(|c| json!({ "url": c.page.url, "title": c.page.title, "fetchedAt": c.fetched_at }))
                    .collect::<Vec<_>>()
            })
            .map_or(Value::Array(Vec::new()), Value::Array)),
        "docsCacheClear" => Ok(json!(docs::cache_dir().map_or(0, |d| docs::cache_clear(&d)))),
        _ => return None,
    })
}
