//! Local WebSocket server the Kor browser extension connects to (directly, or through the native
//! messaging host `kor chrome-native-host`, which relays the same frames). ADR 0058.
//!
//! Protocol (JSON text frames):
//! - extension → `{"type":"pair","key":"…"}` (a key from an earlier pairing) or
//!   `{"type":"pair","code":"ABCD-EFGH"}` (one-time code from `kor chrome pair`) first. The server
//!   answers `{"type":"paired","key"?:…,"policy":{allow,deny}}` or `{"type":"unpaired","error"}` and
//!   closes. Nothing else is read before pairing.
//! - server → `{"id":N,"cmd":…}`; extension → `{"id":N,"ok":true,"result":…}` or
//!   `{"id":N,"ok":false,"error":"…"}` (browser tool commands, see `browser/extension.rs`).
//! - extension → `{"type":"client","rid":…,"op":…}`: side-panel requests (chat, approve, stop,
//!   sessions, docs, forms, page→code…) answered by the front end that registered
//!   [`set_client_handler`] (the desktop) as `{"type":"clientReply","rid":…,"result":…}`.
//! - extension → `{"type":"panel","rid":…}`: hosting-panel requests (§29.2, `kor-web`), answered as
//!   `{"type":"panelReply","rid":…,"result":…}`.
//! - server → `{"type":"event","event":…}`: every agent event of the front end ([`broadcast`]), so
//!   the side panel shows what the CLI and desktop show.
//!
//! Only 127.0.0.1, only `chrome-extension://` / `moz-extension://` origins (no web page can open
//! it), constant-time key checks. Several browsers may be paired at once; tool commands go to the
//! one that connected last.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use kor_secrets::SecretStore;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::lock;

/// Fixed in the extension (`frontend/src/extension/transport.ts`).
pub const PORT: u16 = 49201;
/// Screenshots and GIF frames are the largest messages (a few MB as data URLs).
const MAX_MESSAGE: usize = 32 << 20;
const PAIR_TIMEOUT: Duration = Duration::from_secs(10);

type Reply = Result<Value, String>;

/// Side-panel requests about the page the user is on (hosting panels, §29.2), answered by a crate
/// that cannot be a dependency here (`kor-web`): `{"type":"panel","rid":…}` → `{"type":"panelReply","rid":…,"result":…}`.
pub type PanelHandler = fn(Value) -> futures::future::BoxFuture<'static, Value>;
static PANEL: std::sync::OnceLock<PanelHandler> = std::sync::OnceLock::new();

/// Register the side-panel handler once (desktop/CLI start-up).
pub fn set_panel_handler(h: PanelHandler) {
    let _ = PANEL.set(h);
}

/// Side-panel client requests (`{"type":"client","op":…}`), answered by the front end that owns
/// the sessions (the desktop app).
pub type ClientHandler = fn(Value) -> futures::future::BoxFuture<'static, Value>;
static CLIENT: std::sync::OnceLock<ClientHandler> = std::sync::OnceLock::new();

/// Register the side-panel client handler once.
pub fn set_client_handler(h: ClientHandler) {
    let _ = CLIENT.set(h);
}

/// Where pairing state lives.
#[derive(Clone)]
pub struct Auth {
    /// Directory of the one-time code file (`<data dir>/chrome`).
    pub dir: PathBuf,
    pub store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth").field("dir", &self.dir).finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct State {
    /// Paired connections: number → outgoing queue. The highest number is the newest.
    conns: HashMap<u64, mpsc::UnboundedSender<Message>>,
    /// Waiting tool commands: id → (connection, answer).
    pending: HashMap<u64, (u64, oneshot::Sender<Reply>)>,
}

#[derive(Debug)]
pub struct Bridge {
    port: u16,
    auth: Auth,
    next: AtomicU64,
    state: Mutex<State>,
}

/// Compare without an early exit, so timing does not reveal how much of a secret matched.
pub(super) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Handshake check: only extension pages (or the native host relaying for one) may connect,
/// never a web page on some site.
#[allow(clippy::result_large_err)] // the error type is fixed by tungstenite's `Callback`
fn only_extensions(req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
    let origin = req
        .headers()
        .get("origin")
        .and_then(|o| o.to_str().ok())
        .unwrap_or("");
    if origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://") {
        Ok(resp)
    } else {
        let mut e = ErrorResponse::new(None);
        *e.status_mut() = StatusCode::FORBIDDEN;
        Err(e)
    }
}

fn text(v: &Value) -> Message {
    Message::Text(v.to_string().into())
}

impl Bridge {
    /// Bind `127.0.0.1:port` (0 = any free port) and serve in the background.
    pub async fn bind(port: u16, auth: Auth) -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let bridge = Arc::new(Self {
            port,
            auth,
            next: AtomicU64::new(1),
            state: Mutex::new(State::default()),
        });
        let b = bridge.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(b.clone().serve(stream));
            }
        });
        Ok(bridge)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    pub fn connected(&self) -> bool {
        !lock(&self.state).conns.is_empty()
    }

    /// Number of paired browsers connected now.
    pub fn connections(&self) -> usize {
        lock(&self.state).conns.len()
    }

    /// Send `v` to every connected extension (agent events for the side panel).
    pub fn broadcast(&self, v: &Value) {
        let msg = text(v);
        for tx in lock(&self.state).conns.values() {
            let _ = tx.send(msg.clone());
        }
    }

    /// First frame: a known key, or a one-time code traded for a new key.
    fn pair(&self, first: &Value) -> Result<Option<String>, String> {
        if first["type"] != "pair" {
            return Err("not paired".into());
        }
        if let Some(key) = first["key"].as_str()
            && super::pairing::known_key(self.auth.store.as_ref(), key)
        {
            return Ok(None);
        }
        if let Some(code) = first["code"].as_str() {
            return super::pairing::redeem(&self.auth.dir, code, self.auth.store.as_ref()).map(Some);
        }
        Err("this browser is not paired with Kor; run `kor chrome pair` and enter the code".into())
    }

    async fn serve(self: Arc<Self>, stream: TcpStream) {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let Ok(ws) =
            tokio_tungstenite::accept_hdr_async_with_config(stream, only_extensions, Some(config))
                .await
        else {
            return;
        };
        let (mut sink, mut source) = ws.split();

        let first = match tokio::time::timeout(PAIR_TIMEOUT, source.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(t.as_str()).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let issued = match self.pair(&first) {
            Ok(issued) => issued,
            Err(error) => {
                let _ = sink.send(text(&json!({"type": "unpaired", "error": error}))).await;
                let _ = sink.close().await;
                return;
            }
        };
        let policy = super::policy::current();
        let mut hello = json!({"type": "paired", "policy": policy, "port": self.port});
        if let Some(key) = issued {
            hello["key"] = json!(key);
        }
        if sink.send(text(&hello)).await.is_err() {
            return;
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.state).conns.insert(id, tx.clone());
        let writer = tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        while let Some(Ok(msg)) = source.next().await {
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else {
                continue;
            };
            self.dispatch(v, &tx);
        }
        {
            let mut st = lock(&self.state);
            st.conns.remove(&id);
            // Waiters of this connection see a closed channel at once.
            st.pending.retain(|_, (c, _)| *c != id);
        }
        writer.abort();
    }

    fn dispatch(&self, v: Value, tx: &mpsc::UnboundedSender<Message>) {
        if let Some(n) = v["id"].as_u64() {
            let reply = if v["ok"] == true {
                Ok(v["result"].clone())
            } else {
                Err(v["error"].as_str().unwrap_or("extension error").to_owned())
            };
            if let Some((_, waiter)) = lock(&self.state).pending.remove(&n) {
                let _ = waiter.send(reply);
            }
            return;
        }
        let kind = v["type"].as_str().unwrap_or("");
        let (handler, reply_type) = match kind {
            "panel" => (PANEL.get().copied(), "panelReply"),
            "client" => (CLIENT.get().copied(), "clientReply"),
            _ => return, // `ping` keeps the service worker alive; nothing to answer
        };
        let rid = v["rid"].clone();
        let tx = tx.clone();
        if kind == "client" && v["op"] == "policy" {
            let _ = tx.send(text(&json!({"type": reply_type, "rid": rid, "result": super::policy::current()})));
            return;
        }
        let Some(handler) = handler else {
            let error = "Kor masaüstü uygulaması çalışmıyor; yan panel sohbeti için masaüstünü açın.";
            let _ = tx.send(text(&json!({"type": reply_type, "rid": rid, "result": {"error": error}})));
            return;
        };
        let fut = handler(v);
        tokio::spawn(async move {
            let result = fut.await;
            let _ = tx.send(text(&json!({"type": reply_type, "rid": rid, "result": result})));
        });
    }

    /// Send a command to the most recently connected extension and wait for its answer.
    pub async fn request(&self, mut cmd: Value, timeout: Duration) -> Reply {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        cmd["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        {
            let mut st = lock(&self.state);
            let Some((&conn, out)) = st.conns.iter().max_by_key(|(c, _)| **c) else {
                return Err(not_paired());
            };
            if out.send(text(&cmd)).is_err() {
                return Err(not_paired());
            }
            st.pending.insert(id, (conn, tx));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err("the Kor browser extension disconnected".into()),
            Err(_) => {
                lock(&self.state).pending.remove(&id);
                Err(format!(
                    "the Kor browser extension did not answer within {} s",
                    timeout.as_secs()
                ))
            }
        }
    }
}

pub(super) fn not_paired() -> String {
    "the Kor browser extension is not connected: install it (`kor chrome install`), pair it (`kor chrome pair`) and keep the browser open".into()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use kor_secrets::MemoryStore;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    pub(crate) type Ws =
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

    pub(crate) async fn client(port: u16) -> Ws {
        let mut req = format!("ws://127.0.0.1:{port}")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("origin", "chrome-extension://abcdef".parse().unwrap());
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }

    pub(crate) async fn recv(ws: &mut Ws) -> Value {
        let Some(Ok(Message::Text(t))) = ws.next().await else { panic!("no message") };
        serde_json::from_str(t.as_str()).unwrap()
    }

    pub(crate) async fn send(ws: &mut Ws, v: Value) {
        ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }

    /// A bridge on a free port and an extension paired with a fresh code.
    pub(crate) async fn paired() -> (Arc<Bridge>, Ws, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth { dir: dir.path().to_path_buf(), store: Arc::new(MemoryStore::default()) };
        let b = Bridge::bind(0, auth).await.unwrap();
        let (code, _) = super::super::pairing::new_code(dir.path()).unwrap();
        let mut ws = client(b.port()).await;
        send(&mut ws, json!({"type": "pair", "code": code})).await;
        let hello = recv(&mut ws).await;
        assert_eq!(hello["type"], "paired");
        assert_eq!(hello["key"].as_str().map(str::len), Some(64));
        (b, ws, dir)
    }

    #[test]
    fn constant_time_compare() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
    }

    #[tokio::test]
    async fn keys_from_a_code_reconnect_and_unpaired_connections_are_closed() {
        let (b, mut ws, _dir) = paired().await;
        assert!(b.connected());
        let hello_key = {
            // A second connection with the issued key works; with a wrong key it is closed.
            let key = super::super::pairing::redeem(&b.auth.dir, "nope", b.auth.store.as_ref());
            assert!(key.is_err());
            let (code, _) = super::super::pairing::new_code(&b.auth.dir).unwrap();
            let mut w2 = client(b.port()).await;
            send(&mut w2, json!({"type": "pair", "code": code})).await;
            recv(&mut w2).await["key"].as_str().unwrap().to_owned()
        };
        let mut w3 = client(b.port()).await;
        send(&mut w3, json!({"type": "pair", "key": hello_key})).await;
        assert_eq!(recv(&mut w3).await["type"], "paired");
        let mut bad = client(b.port()).await;
        send(&mut bad, json!({"type": "pair", "key": "f".repeat(64)})).await;
        assert_eq!(recv(&mut bad).await["type"], "unpaired");
        assert!(!matches!(bad.next().await, Some(Ok(Message::Text(_)))));
        // Unpaired traffic is never dispatched.
        let mut silent = client(b.port()).await;
        send(&mut silent, json!({"type": "client", "rid": 1, "op": "policy"})).await;
        assert_eq!(recv(&mut silent).await["type"], "unpaired");
        // Broadcast reaches paired connections.
        b.broadcast(&json!({"type": "event", "event": {"type": "done"}}));
        assert_eq!(recv(&mut ws).await["event"]["type"], "done");
    }

    #[tokio::test]
    async fn panel_and_client_requests_reach_their_handlers() {
        fn echo(v: Value) -> futures::future::BoxFuture<'static, Value> {
            Box::pin(async move { json!({"op": v["op"].clone()}) })
        }
        set_panel_handler(echo);
        set_client_handler(echo);
        let (_b, mut ws, _dir) = paired().await;
        send(&mut ws, json!({"type": "panel", "rid": 7, "op": "detect"})).await;
        assert_eq!(recv(&mut ws).await, json!({"type": "panelReply", "rid": 7, "result": {"op": "detect"}}));
        send(&mut ws, json!({"type": "client", "rid": 8, "op": "sessions"})).await;
        assert_eq!(recv(&mut ws).await, json!({"type": "clientReply", "rid": 8, "result": {"op": "sessions"}}));
        send(&mut ws, json!({"type": "client", "rid": 9, "op": "policy"})).await;
        assert_eq!(recv(&mut ws).await["type"], "clientReply");
    }

    #[tokio::test]
    async fn web_pages_cannot_connect() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth { dir: dir.path().to_path_buf(), store: Arc::new(MemoryStore::default()) };
        let b = Bridge::bind(0, auth).await.unwrap();
        let mut req = format!("ws://127.0.0.1:{}", b.port())
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        assert!(tokio_tungstenite::connect_async(req).await.is_err());
        assert!(
            b.request(json!({"cmd": "snapshot"}), Duration::from_secs(1))
                .await
                .is_err()
        );
    }
}
