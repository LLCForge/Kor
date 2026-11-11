//! The Kor browser extension bridge (§18, ADR 0058): Kor drives the user's OWN Chrome / Edge /
//! Brave / Firefox (their tabs and logins) through the extension in `frontend/src/extension`.
//!
//! - [`server`]: the local WebSocket the extension (or the native host relaying for it) connects
//!   to, with pairing ([`pairing`]) and the side-panel channels.
//! - [`policy`]: the user's domain allow/deny lists (`kor chrome allow|deny`).
//! - [`native_host`]: `kor chrome-native-host`, the native messaging relay.
//! - The `browser_*` tools use the extension as their backend when `[browser] backend` says so
//!   (`tools/browser/extension.rs`): each task gets its own tab group and Kor never touches tabs
//!   outside it unless the user hands a tab over from the side panel.

pub mod native_host;
pub mod pairing;
pub mod policy;
pub(crate) mod server;

pub use server::{Auth, Bridge, ClientHandler, PORT, PanelHandler, set_client_handler, set_panel_handler};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, lock};

static BRIDGE: Mutex<Option<Arc<Bridge>>> = Mutex::new(None);

/// Which browser the `browser_*` tools drive (`[browser] backend`, set by the runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    Auto,
    Extension,
    Headless,
}

static BACKEND: AtomicU8 = AtomicU8::new(0);

/// Set from `[browser] backend` (runtime start-up).
pub fn set_backend(b: Backend) {
    BACKEND.store(b as u8, Ordering::Relaxed);
}

pub fn backend() -> Backend {
    match BACKEND.load(Ordering::Relaxed) {
        1 => Backend::Extension,
        2 => Backend::Headless,
        _ => Backend::Auto,
    }
}

/// Legacy switch: `KOR_COMPUTER_CONTROL=1` starts the bridge in CLI sessions even when the backend
/// is `auto`.
pub fn enabled() -> bool {
    matches!(
        std::env::var("KOR_COMPUTER_CONTROL").ok().as_deref(),
        Some("1" | "true")
    )
}

/// The running bridge of this process, if started.
pub fn global() -> Option<Arc<Bridge>> {
    lock(&BRIDGE).clone()
}

/// `<data dir>/chrome`: the one-time pairing code.
pub fn state_dir() -> Option<PathBuf> {
    kor_platform::data_dir().map(|d| d.join("chrome"))
}

/// Start the process-wide bridge on 127.0.0.1:[`PORT`] once (keys in the OS keychain); later
/// calls return it.
///
/// # Errors
/// The port is taken (another Kor process runs the bridge) or no data directory.
pub async fn start() -> Result<Arc<Bridge>, String> {
    static STARTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = STARTING.lock().await;
    if let Some(b) = global() {
        return Ok(b);
    }
    let dir = state_dir().ok_or("no data directory for the browser bridge")?;
    let auth = Auth { dir, store: Arc::new(kor_secrets::KeychainStore) };
    let bridge = Bridge::bind(PORT, auth).await?;
    *lock(&BRIDGE) = Some(bridge.clone());
    Ok(bridge)
}

/// [`start`] in the background from sync code; no-op outside a tokio runtime.
pub fn spawn_start() {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async { start().await.ok() });
    }
}

/// Mirror one agent event to the side panels (`{"type":"event","event":…}`).
pub fn broadcast_event(event: &Value) {
    if let Some(b) = global() {
        b.broadcast(&json!({"type": "event", "event": event}));
    }
}

/// Browser actions that always ask in the user's own browser, whatever the mode or rules.
pub fn is_action(name: &str) -> bool {
    backend_in_use()
        && matches!(
            name,
            "browser_open"
                | "browser_click"
                | "browser_fill"
                | "browser_press"
                | "browser_select"
                | "browser_hover"
                | "browser_back"
                | "browser_tabs"
                | "browser_eval"
                | "browser_upload"
        )
}

/// Whether the `browser_*` tools go to the extension now.
pub fn backend_in_use() -> bool {
    match backend() {
        Backend::Headless => false,
        Backend::Extension => true,
        Backend::Auto => global().is_some_and(|b| b.connected()),
    }
}

/// `chrome_bridge_info`: status and setup steps. Offered when the bridge runs.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(InfoTool)]
}

#[derive(Debug)]
struct InfoTool;

#[async_trait]
impl Tool for InfoTool {
    fn name(&self) -> &'static str {
        "chrome_bridge_info"
    }
    fn rule_name(&self) -> &'static str {
        "chrome_bridge_info"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Status of the Kor browser extension (the user's own Chrome/Edge/Brave/Firefox): connected or not, which backend the browser_* tools use, and how the user installs and pairs it. Pairing codes and keys are never shown here.".into(),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        Some("chrome_bridge_info".into())
    }
    async fn call(&self, _input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let (status, port) = match global() {
            Some(b) if b.connected() => (format!("extension connected ({} browser(s))", b.connections()), b.port()),
            Some(b) => ("extension not connected".to_owned(), b.port()),
            None => ("bridge not running in this Kor process".to_owned(), PORT),
        };
        let backend = match backend() {
            Backend::Auto => "auto (extension when connected, else headless)",
            Backend::Extension => "chrome (the user's browser through the extension)",
            Backend::Headless => "headless (Kor's isolated browser)",
        };
        ToolOutput::ok(format!(
            "Status: {status}\nPort: {port} (127.0.0.1 only)\nbrowser_* backend: {backend}\nSetup (the user does this): `kor chrome install`, load the extension, then `kor chrome pair` and enter the code in the Kor side panel. Domains: `kor chrome allow|deny <domain>`. Kor works only in its own tab group."
        ))
    }
}

/// Read one page in the user's own browser (their logins) through the extension: a background
/// tab in Kor's group, its text, then the tab is closed. Used by `kor docs add --browser` and
/// `kor docs use --browser`. Domain lists and privacy/offline mode apply.
///
/// # Errors
/// Refused URL, no paired extension within 10 s, or the page could not be read.
pub async fn read_page(url: &str) -> Result<(String, String), String> {
    if kor_core::net::policy() != kor_core::net::Policy::Open {
        return Err("privacy or offline mode is on: Kor does not use the user's browser".into());
    }
    policy::current().check(url)?;
    let bridge = match global() {
        Some(b) => b,
        None => start().await?,
    };
    for _ in 0..20 {
        if bridge.connected() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let task = json!({"key": "docs", "title": "Kor · docs"});
    let tab = bridge
        .request(json!({"cmd": "tabs.create", "task": task, "url": url, "active": false}), std::time::Duration::from_secs(40))
        .await?;
    let id = tab["tabId"].as_i64().ok_or("the extension returned no tab id")?;
    let snap = bridge
        .request(json!({"cmd": "snapshot", "task": task, "tabId": id}), std::time::Duration::from_secs(20))
        .await;
    let _ = bridge
        .request(json!({"cmd": "tabs.close", "task": task, "tabId": id}), std::time::Duration::from_secs(10))
        .await;
    let snap = snap?;
    Ok((
        snap["title"].as_str().unwrap_or_default().to_owned(),
        snap["text"].as_str().unwrap_or_default().to_owned(),
    ))
}
