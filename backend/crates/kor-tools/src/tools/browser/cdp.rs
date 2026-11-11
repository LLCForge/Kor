//! A small Chrome `DevTools` Protocol client: one WebSocket to the browser, flattened target
//! sessions, responses routed by id. Console messages, dialogs and network requests are collected
//! per session.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::lock;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LOGS: usize = 300;
const MAX_REQUESTS: usize = 300;
/// The Chrome for Testing headless shell (`kor component add browser`).
pub(crate) const COMPONENT: &str = "chromium";
/// Distinguishes profiles of several browsers started by one process.
static SEQ: AtomicU64 = AtomicU64::new(0);

type Pending = HashMap<u64, oneshot::Sender<Result<Value, String>>>;
pub(crate) type Logs = std::sync::Mutex<HashMap<String, VecDeque<String>>>;
pub(crate) type Requests = std::sync::Mutex<HashMap<String, VecDeque<Request>>>;

/// One network request of a tab, as `browser_network` reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Request {
    pub id: String,
    pub method: String,
    pub url: String,
    /// `Document`, `Script`, `XHR`, `Fetch`…
    pub kind: String,
    pub status: Option<u16>,
    pub mime: String,
    pub bytes: Option<u64>,
    /// `net::ERR_…` text, when the request failed.
    pub failed: Option<String>,
}

/// Which hosts the agent browser may reach (sandbox allowlist + loopback, see ADR 0042).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetRule {
    Open,
    /// Loopback plus these host patterns (`example.com`, `*.example.com`).
    Only(Vec<String>),
}

impl NetRule {
    pub(crate) fn allows(&self, host: &str) -> bool {
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        match self {
            Self::Open => true,
            Self::Only(list) => {
                kor_core::net::is_loopback_host(&host)
                    || list.iter().any(|p| match p.strip_prefix("*.") {
                        Some(base) => host == base || host.ends_with(&format!(".{base}")),
                        None => p == "*" || *p == host,
                    })
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Arc<std::sync::Mutex<Pending>>,
    next: AtomicU64,
    /// Console lines per session id.
    pub(crate) logs: Arc<Logs>,
    /// Network requests per session id.
    pub(crate) requests: Arc<Requests>,
    child: tokio::process::Child,
    owner: Option<std::fs::File>,
    profile: PathBuf,
    /// Closed proxy that every refused connection goes to.
    trap: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Cdp {
    fn drop(&mut self) {
        // Stop the browser, release the owner lock, then try to remove the profile. Windows keeps
        // the files locked for a moment and the process may be exiting, so a profile that stays
        // behind is removed by the next launch (its owner lock is then free).
        let _ = self.child.start_kill();
        if let Some(t) = self.trap.take() {
            t.abort();
        }
        drop(self.owner.take());
        for _ in 0..30 {
            if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

const OWNER_LOCK: &str = "kor-owner.lock";

/// Held open while the profile is in use. On Windows it is opened without delete sharing, so
/// nobody can delete it until this process closes it or dies.
fn owner_lock(profile: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        o.share_mode(1); // FILE_SHARE_READ only
    }
    o.open(profile.join(OWNER_LOCK))
}

/// Remove profiles whose owner is gone: on Windows a deletable owner lock means its process
/// ended; elsewhere, profiles older than a day.
fn sweep_stale_profiles() {
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in dir.filter_map(Result::ok) {
        if !e.file_name().to_string_lossy().starts_with("kor-browser-") {
            continue;
        }
        let path = e.path();
        let older_than = |secs: u64| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(secs))
        };
        let stale = if cfg!(windows) {
            match std::fs::remove_file(path.join(OWNER_LOCK)) {
                Ok(()) => true,
                // No lock yet: another Kor may be creating it right now.
                Err(err) => err.kind() == std::io::ErrorKind::NotFound && older_than(60),
            }
        } else {
            older_than(86_400)
        };
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Chrome, then Edge (`KOR_BROWSER` overrides): any Chromium browser speaks CDP.
pub(crate) fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KOR_BROWSER").map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
                candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
                candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        candidates.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        candidates.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
        candidates.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
    } else {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
        ] {
            if let Some(path) = std::env::var_os("PATH") {
                candidates.extend(std::env::split_paths(&path).map(|d| d.join(name)));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The browser for the agent: `KOR_BROWSER`, then the verified headless component when no window
/// is wanted, then an installed Chrome or Edge.
fn agent_browser(visible: bool) -> Option<PathBuf> {
    let component = || kor_components::path(COMPONENT).filter(|p| p.is_file());
    if std::env::var_os("KOR_BROWSER").is_some() || visible {
        find_browser().or_else(component)
    } else {
        component().or_else(find_browser)
    }
}

fn is_headless_shell(exe: &Path) -> bool {
    exe.file_name()
        .is_some_and(|n| n.to_string_lossy().contains("headless-shell"))
}

/// Command-line flags of the agent browser. `proxy` is the closed port refused traffic goes to.
pub(crate) fn launch_args(
    profile: &Path,
    headless: Option<&str>,
    proxy: Option<u16>,
    rule: &NetRule,
) -> Vec<String> {
    let mut a = vec![
        "--remote-debugging-port=0".to_owned(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-extensions".into(),
        "--disable-sync".into(),
        "--disable-background-networking".into(),
        "--disable-component-update".into(),
        "--password-store=basic".into(),
        "--window-size=1280,900".into(),
    ];
    if let Some(h) = headless {
        a.push(h.to_owned());
    }
    if let Some(port) = proxy {
        restrict(&mut a, port, rule);
    }
    a.push("about:blank".into());
    a
}

/// [`launch_args`] that loads one unpacked extension (the Kor extension's integration test,
/// ADR 0058) instead of disabling all of them. Branded Chrome 137+ ignores `--load-extension`;
/// Chromium and Chrome for Testing honour it.
pub(crate) fn launch_args_with_extension(profile: &Path, headless: Option<&str>, extension: &Path) -> Vec<String> {
    let dir = extension.display().to_string();
    let mut a: Vec<String> = launch_args(profile, headless, None, &NetRule::Open)
        .into_iter()
        .filter(|x| x != "--disable-extensions" && x != "about:blank")
        .collect();
    a.push(format!("--disable-extensions-except={dir}"));
    a.push(format!("--load-extension={dir}"));
    a.push("--disable-features=DisableLoadExtensionCommandLineSwitch".into());
    a.push("about:blank".into());
    a
}

fn restrict(a: &mut Vec<String>, port: u16, rule: &NetRule) {
    if let NetRule::Only(hosts) = rule {
        // Everything not bypassed goes to a proxy that closes each connection: refused hosts fail
        // at once, IP literals and WebSockets included. Loopback always bypasses proxies.
        a.push(format!("--proxy-server=http://127.0.0.1:{port}"));
        let mut bypass = vec!["localhost", "127.0.0.1", "[::1]", "*.localhost"];
        bypass.extend(hosts.iter().map(String::as_str));
        a.push(format!("--proxy-bypass-list={}", bypass.join(";")));
        // UDP (WebRTC) does not use the proxy.
        a.push("--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into());
    }
}

/// A loopback port that accepts and closes every connection.
async fn closed_proxy() -> Result<(u16, tokio::task::JoinHandle<()>), String> {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = l.local_addr().map_err(|e| e.to_string())?.port();
    let task = tokio::spawn(async move {
        while let Ok((sock, _)) = l.accept().await {
            drop(sock);
        }
    });
    Ok((port, task))
}

async fn read_port_file(profile: &Path) -> Result<(u16, String), String> {
    let file = profile.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(text) = tokio::fs::read_to_string(&file).await {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next())
                && let Ok(port) = port.trim().parse::<u16>()
            {
                return Ok((port, path.trim().to_owned()));
            }
        }
        if tokio::time::Instant::now() > deadline {
            return Err("the browser did not open its DevTools port in time".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

impl Cdp {
    /// Start an isolated browser (fresh temporary profile, no extensions or sync) and connect.
    pub(crate) async fn launch(visible: bool, rule: &NetRule) -> Result<Self, String> {
        Self::launch_with(visible, rule, None).await
    }

    /// [`Cdp::launch`], optionally with one unpacked extension loaded (tests only use it).
    pub(crate) async fn launch_with(visible: bool, rule: &NetRule, extension: Option<&Path>) -> Result<Self, String> {
        let exe = agent_browser(visible).ok_or(
            "No browser found. Install Kor's headless browser with `kor component add browser`, install Chrome or Edge, or set KOR_BROWSER to a Chromium-based browser.",
        )?;
        let profile = std::env::temp_dir().join(format!(
            "kor-browser-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        sweep_stale_profiles();
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
        let owner = owner_lock(&profile).map_err(|e| e.to_string())?;
        let (proxy, trap) = match rule {
            NetRule::Open => (None, None),
            NetRule::Only(_) => {
                let (port, task) = closed_proxy().await?;
                (Some(port), Some(task))
            }
        };
        // The headless shell is always headless and does not know `--headless=new`.
        let headless = (!visible && !is_headless_shell(&exe)).then_some("--headless=new");
        let mut cmd = tokio::process::Command::new(&exe);
        let args = match extension {
            Some(ext) => launch_args_with_extension(&profile, headless, ext),
            None => launch_args(&profile, headless, proxy, rule),
        };
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
        let (port, path) = read_port_file(&profile).await?;
        // The port is local; only 127.0.0.1 is ever contacted.
        let url = format!("ws://127.0.0.1:{port}{path}");
        let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| format!("could not connect to the browser: {e}"))?;
        let (mut sink, mut stream) = ws.split();
        let (out, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        let pending: Arc<std::sync::Mutex<Pending>> = Arc::default();
        let logs: Arc<Logs> = Arc::default();
        let requests: Arc<Requests> = Arc::default();
        {
            let (pending, logs, requests, out) =
                (pending.clone(), logs.clone(), requests.clone(), out.clone());
            tokio::spawn(async move {
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        dispatch(text.as_str(), &pending, &logs, &requests, &out);
                    }
                }
                // Connection gone: fail everything still waiting.
                for (_, tx) in lock(&pending).drain() {
                    let _ = tx.send(Err("the browser closed".into()));
                }
            });
        }
        let cdp = Self {
            out,
            pending,
            next: AtomicU64::new(1),
            logs,
            requests,
            child,
            owner: Some(owner),
            profile,
            trap,
        };
        // Downloads would write outside the throwaway profile.
        let _ = cdp
            .call(
                None,
                "Browser.setDownloadBehavior",
                json!({"behavior": "deny"}),
            )
            .await;
        Ok(cdp)
    }

    /// One command; `session` targets a page, `None` the browser.
    pub(crate) async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        if self.out.send(msg.to_string()).is_err() {
            lock(&self.pending).remove(&id);
            return Err("the browser closed".into());
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("the browser closed".into()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(format!("{method} timed out"))
            }
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.out.is_closed()
    }
}

/// One message from the browser: a response goes to its waiting call, anything else is an event.
fn dispatch(
    text: &str,
    pending: &std::sync::Mutex<Pending>,
    logs: &Logs,
    requests: &Requests,
    out: &mpsc::UnboundedSender<String>,
) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        if let Some(tx) = lock(pending).remove(&id) {
            let res = match v.get("error") {
                Some(e) => Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("CDP error")
                    .to_owned()),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(res);
        }
    } else {
        on_event(&v, logs, requests, out);
    }
}

fn push_log(logs: &Logs, session: &str, line: String) {
    let mut all = lock(logs);
    let q = all.entry(session.to_owned()).or_default();
    if q.len() >= MAX_LOGS {
        q.pop_front();
    }
    q.push_back(line);
}

/// The request `id` of `session`, created at the end of its list when `create` is set.
fn with_request(
    requests: &Requests,
    session: &str,
    id: &str,
    create: bool,
    f: impl FnOnce(&mut Request),
) {
    let mut all = lock(requests);
    let q = all.entry(session.to_owned()).or_default();
    // Redirects reuse the id: the newest entry is the live one.
    if let Some(r) = q.iter_mut().rev().find(|r| r.id == id) {
        if !create {
            f(r);
            return;
        }
    } else if !create {
        return;
    }
    if q.len() >= MAX_REQUESTS {
        q.pop_front();
    }
    let mut r = Request {
        id: id.to_owned(),
        ..Request::default()
    };
    f(&mut r);
    q.push_back(r);
}

fn on_event(v: &Value, logs: &Logs, requests: &Requests, out: &mpsc::UnboundedSender<String>) {
    let Some(session) = v.get("sessionId").and_then(Value::as_str) else {
        return;
    };
    let params = &v["params"];
    let rid = params["requestId"].as_str().unwrap_or("");
    match v.get("method").and_then(Value::as_str) {
        Some("Runtime.consoleAPICalled") => {
            let kind = params["type"].as_str().unwrap_or("log");
            let text: Vec<String> = params["args"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| match a.get("value") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => a["description"].as_str().unwrap_or("").to_owned(),
                })
                .collect();
            let line: String = format!("[{kind}] {}", text.join(" "))
                .chars()
                .take(2000)
                .collect();
            push_log(logs, session, line);
        }
        Some("Runtime.exceptionThrown") => {
            let d = &params["exceptionDetails"];
            let text = d["exception"]["description"]
                .as_str()
                .or_else(|| d["text"].as_str())
                .unwrap_or("exception");
            let line: String = format!("[exception] {text}").chars().take(2000).collect();
            push_log(logs, session, line);
        }
        Some("Page.javascriptDialogOpening") => {
            // Dialogs would block the page; dismiss them and record what they said.
            let kind = params["type"].as_str().unwrap_or("dialog");
            let message = params["message"].as_str().unwrap_or("");
            push_log(logs, session, format!("[{kind} dismissed] {message}"));
            let cmd = json!({
                "id": 0,
                "sessionId": session,
                "method": "Page.handleJavaScriptDialog",
                "params": {"accept": kind == "alert" || kind == "beforeunload"}
            });
            let _ = out.send(cmd.to_string());
        }
        Some("Network.requestWillBeSent") if !rid.is_empty() => {
            let req = &params["request"];
            with_request(requests, session, rid, true, |r| {
                req["method"]
                    .as_str()
                    .unwrap_or("GET")
                    .clone_into(&mut r.method);
                r.url = req["url"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(500)
                    .collect();
                params["type"].as_str().unwrap_or("").clone_into(&mut r.kind);
            });
        }
        Some("Network.responseReceived") if !rid.is_empty() => {
            let resp = &params["response"];
            with_request(requests, session, rid, false, |r| {
                r.status = resp["status"].as_u64().and_then(|s| u16::try_from(s).ok());
                resp["mimeType"]
                    .as_str()
                    .unwrap_or("")
                    .clone_into(&mut r.mime);
            });
        }
        Some("Network.loadingFinished") if !rid.is_empty() => {
            with_request(requests, session, rid, false, |r| {
                r.bytes = params["encodedDataLength"].as_f64().map(byte_count);
            });
        }
        Some("Network.loadingFailed") if !rid.is_empty() => {
            with_request(requests, session, rid, false, |r| {
                let text = params["errorText"].as_str().unwrap_or("failed");
                r.failed = Some(match params["blockedReason"].as_str() {
                    Some(why) => format!("{text} ({why})"),
                    None => text.to_owned(),
                });
            });
        }
        _ => {}
    }
}

/// CDP reports byte counts as JSON floats. Clamped to `[0, u64::MAX]` first (NaN -> 0),
/// so the saturating `as` only drops the fractional part.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn byte_count(b: f64) -> u64 {
    b.max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(method: &str, params: &Value) -> String {
        json!({"method": method, "sessionId": "S", "params": params}).to_string()
    }

    #[test]
    fn responses_reach_their_call_and_errors_carry_the_message() {
        let pending = std::sync::Mutex::new(Pending::new());
        let (logs, requests) = (Logs::default(), Requests::default());
        let (out, _rx) = mpsc::unbounded_channel();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        lock(&pending).insert(1, tx1);
        lock(&pending).insert(2, tx2);
        dispatch(
            r#"{"id":2,"error":{"code":-32000,"message":"No node"}}"#,
            &pending,
            &logs,
            &requests,
            &out,
        );
        dispatch(
            r#"{"id":1,"result":{"frameId":"F"}}"#,
            &pending,
            &logs,
            &requests,
            &out,
        );
        dispatch("not json", &pending, &logs, &requests, &out);
        assert_eq!(rx1.try_recv().unwrap().unwrap()["frameId"], "F");
        assert_eq!(rx2.try_recv().unwrap().unwrap_err(), "No node");
        assert!(lock(&pending).is_empty());
    }

    #[test]
    fn console_dialogs_and_exceptions_are_logged_per_session() {
        let pending = std::sync::Mutex::new(Pending::new());
        let (logs, requests) = (Logs::default(), Requests::default());
        let (out, mut rx) = mpsc::unbounded_channel();
        let d = |t: String| dispatch(&t, &pending, &logs, &requests, &out);
        d(ev(
            "Runtime.consoleAPICalled",
            &json!({"type": "error", "args": [{"value": "boom"}, {"value": 3}]}),
        ));
        d(ev(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails": {"exception": {"description": "TypeError: x"}}}),
        ));
        d(ev(
            "Page.javascriptDialogOpening",
            &json!({"type": "confirm", "message": "Sure?"}),
        ));
        let lines: Vec<String> = lock(&logs)["S"].iter().cloned().collect();
        assert_eq!(
            lines,
            [
                "[error] boom 3",
                "[exception] TypeError: x",
                "[confirm dismissed] Sure?"
            ]
        );
        let reply: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(reply["method"], "Page.handleJavaScriptDialog");
        assert_eq!(reply["params"]["accept"], false);
    }

    #[test]
    fn network_events_build_one_entry_per_request() {
        let pending = std::sync::Mutex::new(Pending::new());
        let (logs, requests) = (Logs::default(), Requests::default());
        let (out, _rx) = mpsc::unbounded_channel();
        let d = |t: String| dispatch(&t, &pending, &logs, &requests, &out);
        d(ev(
            "Network.requestWillBeSent",
            &json!({"requestId": "1", "type": "Document", "request": {"method": "GET", "url": "http://127.0.0.1:9/"}}),
        ));
        d(ev(
            "Network.responseReceived",
            &json!({"requestId": "1", "response": {"status": 200, "mimeType": "text/html"}}),
        ));
        d(ev(
            "Network.loadingFinished",
            &json!({"requestId": "1", "encodedDataLength": 512.0}),
        ));
        d(ev(
            "Network.requestWillBeSent",
            &json!({"requestId": "2", "type": "Fetch", "request": {"method": "POST", "url": "https://evil.example/x"}}),
        ));
        d(ev(
            "Network.loadingFailed",
            &json!({"requestId": "2", "errorText": "net::ERR_PROXY_CONNECTION_FAILED"}),
        ));
        // Unknown ids and events without a session are ignored.
        d(ev(
            "Network.responseReceived",
            &json!({"requestId": "9", "response": {"status": 500}}),
        ));
        dispatch(
            r#"{"method":"Network.requestWillBeSent","params":{"requestId":"3"}}"#,
            &pending,
            &logs,
            &requests,
            &out,
        );
        let all = lock(&requests);
        let q: Vec<&Request> = all["S"].iter().collect();
        assert_eq!(q.len(), 2);
        assert_eq!(
            (q[0].status, q[0].bytes, q[0].mime.as_str()),
            (Some(200), Some(512), "text/html")
        );
        assert_eq!(
            (q[1].method.as_str(), q[1].kind.as_str()),
            ("POST", "Fetch")
        );
        assert_eq!(
            q[1].failed.as_deref(),
            Some("net::ERR_PROXY_CONNECTION_FAILED")
        );
    }

    #[test]
    fn net_rule_allows_loopback_and_listed_hosts_only() {
        let r = NetRule::Only(vec!["example.com".into(), "*.npmjs.org".into()]);
        for ok in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "app.localhost",
            "example.com",
            "registry.npmjs.org",
            "npmjs.org",
        ] {
            assert!(r.allows(ok), "{ok}");
        }
        for bad in [
            "evil.com",
            "example.com.evil.com",
            "notnpmjs.org",
            "8.8.8.8",
        ] {
            assert!(!r.allows(bad), "{bad}");
        }
        assert!(NetRule::Open.allows("evil.com"));
    }

    #[test]
    fn restricted_launch_routes_everything_but_allowed_hosts_to_the_closed_proxy() {
        let p = Path::new("/tmp/p");
        let open = launch_args(p, Some("--headless=new"), None, &NetRule::Open);
        assert!(!open.iter().any(|a| a.starts_with("--proxy")));
        assert!(open.contains(&"--headless=new".to_owned()));
        assert_eq!(open.last().map(String::as_str), Some("about:blank"));
        let only = launch_args(
            p,
            None,
            Some(4321),
            &NetRule::Only(vec!["example.com".into()]),
        );
        assert!(only.contains(&"--proxy-server=http://127.0.0.1:4321".to_owned()));
        let bypass = only
            .iter()
            .find(|a| a.starts_with("--proxy-bypass-list="))
            .unwrap();
        assert!(
            bypass.contains("localhost") && bypass.ends_with(";example.com"),
            "{bypass}"
        );
        assert!(only.iter().any(|a| a.contains("disable_non_proxied_udp")));
        assert!(!only.iter().any(|a| a.starts_with("--headless")));
    }
}
