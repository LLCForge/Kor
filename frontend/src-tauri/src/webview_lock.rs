//! WebView kilidi, Rust tarafı (spec §21, ADR 0053). Arayüz tarafı `renderer/webview-lock.ts`.
//!
//! - Ana pencere `tauri.conf.json`'dan (`create: false`) burada kurulur: devtools yalnızca geliştirici build'inde ve
//!   `KOR_DEVTOOLS=1` / `--devtools` ile açılır (release'de Tauri `devtools` özelliği derlenmez; WebView2
//!   `AreDevToolsEnabled`, WKWebView `developerExtrasEnabled`, WebKitGTK `enable-developer-extras` bu bayrakla kapanır).
//! - Gezinme: yalnızca uygulamanın kendi sayfası ve gömülü şemalar; http(s)/mailto sistem tarayıcısında açılır,
//!   view-source:/about:/file:/javascript: engellenir. `window.open` aynı kurala tabidir.
//! - Kapatma: çalışan görev varken pencere kapanmaz, arayüz Kor'un kendi "kapatılsın mı?" diyaloğunu gösterir
//!   (`kor:window/closeRequested`); onay `webview.closeNow` ile gelir. Tarayıcının beforeunload kutusu hiç yoktur.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, Url, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_opener::OpenerExt;

use crate::state::AppState;

/// `webview.closeNow` kurar: bir sonraki kapatma isteği soru sormadan geçer.
pub static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Allow,
    External,
    Block,
}

/// Bir gezinme hedefinin kararı (testli; arayüzdeki `navDecision` ile aynı liste).
pub fn nav_policy(url: &Url) -> Nav {
    match url.scheme() {
        // Uygulamanın kendi sayfası (Windows/Android: http(s)://tauri.localhost), gömülü önizleme ve varlık şemaları.
        "tauri" | "kor-sandbox" | "asset" | "ipc" => Nav::Allow,
        "http" | "https" => match url.host_str() {
            Some(
                "tauri.localhost" | "kor-sandbox.localhost" | "asset.localhost" | "ipc.localhost",
            ) => Nav::Allow,
            // `tauri dev` serves the renderer from its own local server; release builds never do.
            Some("localhost" | "127.0.0.1") if tauri::is_dev() => Nav::Allow,
            _ => Nav::External,
        },
        "mailto" => Nav::External,
        // iframe srcdoc ve boş çerçeveler sayfa içidir; diğer about: sayfaları (about:config…) değildir.
        "about" if matches!(url.as_str(), "about:blank" | "about:srcdoc") => Nav::Allow,
        "blob" | "data" if is_frame_content(url) => Nav::Allow,
        _ => Nav::Block,
    }
}

/// `blob:`/`data:` yalnızca uygulamanın kendi ürettiği çerçeve içeriğiyse (örn. izole REPL, pdf.js) geçer.
fn is_frame_content(url: &Url) -> bool {
    match url.scheme() {
        "blob" => {
            url.as_str().starts_with("blob:http://tauri.localhost/")
                || url.as_str().starts_with("blob:tauri://localhost/")
        }
        // Üst düzey data: belgesine gezinme (data:text/html…) kimlik avı yüzeyidir; yalnızca görseller.
        "data" => url.as_str().starts_with("data:image/"),
        _ => false,
    }
}

/// Geliştirici araçları: yalnızca debug build + `KOR_DEVTOOLS=1` veya `--devtools`.
pub fn devtools_allowed() -> bool {
    devtools_requested(
        std::env::var("KOR_DEVTOOLS").ok().as_deref(),
        std::env::args(),
    ) && cfg!(debug_assertions)
}

fn devtools_requested(env: Option<&str>, mut args: impl Iterator<Item = String>) -> bool {
    matches!(env, Some("1" | "true")) || args.any(|a| a == "--devtools")
}

/// Ana pencereyi yapılandırmadan, kilitlerle kurar (`setup` içinde çağrılır).
pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let Some(conf) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
    else {
        return Ok(());
    };
    let opener = app.clone();
    let popup = app.clone();
    WebviewWindowBuilder::from_config(app, &conf)?
        .devtools(devtools_allowed())
        .zoom_hotkeys_enabled(false)
        .on_navigation(move |url| match nav_policy(url) {
            Nav::Allow => true,
            Nav::External => {
                let _ = opener.opener().open_url(url.as_str(), None::<&str>);
                false
            }
            Nav::Block => false,
        })
        .on_new_window(move |url, _| {
            if nav_policy(&url) == Nav::External {
                let _ = popup.opener().open_url(url.as_str(), None::<&str>);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

/// Ana pencere kapatılırken: çalışan görev varsa kapanmayı durdurur ve arayüze sorar.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != "main" || FORCE_CLOSE.load(Ordering::SeqCst) {
        return;
    }
    // Tepsiye küçültme açıksa presence.rs pencereyi gizler; görevler sürer, sorulacak bir şey yoktur.
    let tray = crate::presence::settings_json();
    let to_tray = tray["tray"]["enabled"] == true
        && tray["tray"]["closeToTray"] == true
        && window.app_handle().tray_by_id("kor-tray").is_some();
    let running = window.app_handle().state::<AppState>().chats.running_ids();
    if !to_tray && should_ask(running.len()) {
        api.prevent_close();
        let _ = window.emit(
            "kor:window/closeRequested",
            serde_json::json!({ "running": running.len() }),
        );
    }
}

fn should_ask(running: usize) -> bool {
    running > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(s: &str) -> Nav {
        nav_policy(&Url::parse(s).expect("url"))
    }

    #[test]
    fn app_pages_and_embedded_schemes_stay() {
        for u in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html#x",
            "https://tauri.localhost/",
            "kor-sandbox://localhost/p",
            "http://kor-sandbox.localhost/p",
            "http://asset.localhost/C%3A/x.png",
            "about:blank",
            "about:srcdoc",
            "blob:http://tauri.localhost/1234",
            "data:image/png;base64,AAAA",
        ] {
            assert_eq!(nav(u), Nav::Allow, "{u}");
        }
    }

    #[test]
    fn web_links_go_to_the_system_browser() {
        for u in [
            "https://github.com/kor",
            "http://example.com",
            "mailto:a@b.c",
        ] {
            assert_eq!(nav(u), Nav::External, "{u}");
        }
    }

    #[test]
    fn browser_internals_and_local_files_are_blocked() {
        for u in [
            "view-source:https://x.com",
            "about:config",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "blob:https://evil.example/1",
            "chrome://settings",
            "edge://flags",
            "ftp://x.org",
        ] {
            assert_eq!(nav(u), Nav::Block, "{u}");
        }
    }

    #[test]
    fn devtools_only_on_request() {
        assert!(!devtools_requested(None, std::iter::empty()));
        assert!(!devtools_requested(
            Some("0"),
            ["kor".to_owned()].into_iter()
        ));
        assert!(devtools_requested(Some("1"), std::iter::empty()));
        assert!(devtools_requested(
            None,
            ["kor".to_owned(), "--devtools".to_owned()].into_iter()
        ));
    }

    #[test]
    fn close_asks_only_while_tasks_run() {
        assert!(!should_ask(0));
        assert!(should_ask(2));
    }

    /// §21.3 "release build'de devtools sembolü yok": Tauri `devtools` özelliği hiçbir profilde açılmaz.
    #[test]
    fn devtools_feature_not_compiled() {
        let toml = include_str!("../Cargo.toml");
        let tauri_line = toml
            .lines()
            .find(|l| l.trim_start().starts_with("tauri = "))
            .expect("tauri dependency");
        assert!(!tauri_line.contains("devtools"), "{tauri_line}");
        assert!(!toml.contains("\"tauri/devtools\""));
    }

    #[test]
    fn main_window_is_built_by_the_lock() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let main = &conf["app"]["windows"][0];
        assert_eq!(main["label"], "main");
        assert_eq!(
            main["create"], false,
            "ana pencere webview_lock::create_main_window ile kurulur"
        );
        assert_eq!(conf["app"]["security"]["csp"]["default-src"], "'none'");
    }
}
