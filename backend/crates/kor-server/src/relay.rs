//! Peer relay (§31.1 "yoksa takım sunucusu röle olur", §18.4 remote extension connection).
//!
//! A machine that answers (`kor peer serve` with a team server) parks a WebSocket on
//! `/api/v1/relay/listen/<name>`; a member who wants it opens `/api/v1/relay/dial/<name>`. The
//! server hands the dialer to one parked listener, tells the listener `{"type":"connected"}`
//! (so it parks a fresh socket for the next caller), and then forwards messages both ways
//! unchanged. Peer traffic inside is a Noise session between the two machines' keys, so the
//! server sees only ciphertext. Names: `peer:<public key hex>` for peers, `ext:<name>` for the
//! browser extension bridge. The first member to listen on a name owns it; another member's
//! listener is refused.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{SinkExt as _, StreamExt as _};
use hyper_util::rt::TokioIo;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

/// The WebSocket over an upgraded HTTP connection.
pub type Ws = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

const MAX_PARKED: usize = 8;
const MAX_MESSAGE: usize = 1024 * 1024;
const PING: Duration = Duration::from_secs(25);

/// `peer:<hex>` / `ext:<name>`: letters, digits, `:._-`, 1–100.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 100
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

/// WebSocket limits for relayed connections.
pub fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

#[derive(Default)]
struct Inner {
    owners: HashMap<String, String>,
    parked: HashMap<String, Vec<(u64, oneshot::Sender<Ws>)>>,
    next: u64,
}

/// Parked listeners by name.
#[derive(Default, Clone)]
pub struct Hub(Arc<Mutex<Inner>>);

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hub")
    }
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// May `email` listen on `name`? Claims it on first use.
    ///
    /// # Errors
    /// Another member owns the name, or too many sockets are parked.
    pub fn claim(&self, name: &str, email: &str) -> Result<(), String> {
        let mut g = self.lock();
        let owner = g
            .owners
            .entry(name.to_owned())
            .or_insert_with(|| email.to_owned());
        if owner != email {
            return Err("another member listens on this name".into());
        }
        if g.parked.get(name).is_some_and(|v| v.len() >= MAX_PARKED) {
            return Err("too many listeners parked on this name".into());
        }
        Ok(())
    }

    /// Is somebody listening on `name`?
    pub fn listening(&self, name: &str) -> bool {
        self.lock().parked.get(name).is_some_and(|v| !v.is_empty())
    }

    /// Park `ws` until a dialer arrives, then pipe the two. Returns when either side closes.
    pub async fn listen(&self, name: String, mut ws: Ws) {
        let (tx, mut rx) = oneshot::channel();
        let id = {
            let mut g = self.lock();
            g.next += 1;
            let id = g.next;
            g.parked.entry(name.clone()).or_default().push((id, tx));
            id
        };
        let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
        let dialer = loop {
            tokio::select! {
                d = &mut rx => break d.ok(),
                m = ws.next() => match m {
                    // The listener may answer pings; anything else before a dialer is ignored.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break None,
                    Some(Ok(_)) => {}
                },
                _ = ping.tick() => {
                    if ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break None;
                    }
                }
            }
        };
        self.unpark(&name, id);
        let Some(dialer) = dialer else {
            return;
        };
        if ws
            .send(Message::Text(r#"{"type":"connected"}"#.into()))
            .await
            .is_err()
        {
            return;
        }
        pipe(ws, dialer).await;
    }

    fn unpark(&self, name: &str, id: u64) {
        let mut g = self.lock();
        if let Some(v) = g.parked.get_mut(name) {
            v.retain(|(i, _)| *i != id);
            if v.is_empty() {
                g.parked.remove(name);
            }
        }
    }

    /// Hand `ws` to a parked listener on `name`.
    ///
    /// # Errors
    /// Nobody is listening (the socket is closed).
    pub async fn dial(&self, name: &str, mut ws: Ws) -> Result<(), String> {
        loop {
            let next = {
                let mut g = self.lock();
                g.parked.get_mut(name).and_then(Vec::pop)
            };
            let Some((_, tx)) = next else {
                let _ = ws
                    .send(Message::Text(
                        r#"{"type":"error","message":"nobody is listening"}"#.into(),
                    ))
                    .await;
                let _ = ws.close(None).await;
                return Err("nobody is listening".into());
            };
            match tx.send(ws) {
                Ok(()) => return Ok(()),
                // That listener just went away; try the next one.
                Err(back) => ws = back,
            }
        }
    }
}

/// Forward data messages both ways until one side closes.
async fn pipe(a: Ws, b: Ws) {
    let (mut a_tx, mut a_rx) = a.split();
    let (mut b_tx, mut b_rx) = b.split();
    let forward = |m: Message| matches!(m, Message::Text(_) | Message::Binary(_));
    let ab = async {
        while let Some(Ok(m)) = a_rx.next().await {
            if matches!(m, Message::Close(_)) {
                break;
            }
            if forward(m.clone()) && b_tx.send(m).await.is_err() {
                break;
            }
        }
        let _ = b_tx.close().await;
    };
    let ba = async {
        while let Some(Ok(m)) = b_rx.next().await {
            if matches!(m, Message::Close(_)) {
                break;
            }
            if forward(m.clone()) && a_tx.send(m).await.is_err() {
                break;
            }
        }
        let _ = a_tx.close().await;
    };
    tokio::select! {
        () = ab => {}
        () = ba => {}
    }
}

/// Turn an upgraded connection into a server-side WebSocket.
pub async fn accept(up: hyper::upgrade::Upgraded) -> Ws {
    WebSocketStream::from_raw_socket(TokioIo::new(up), Role::Server, Some(ws_config())).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn names() {
        assert!(super::valid_name("peer:ab12"));
        assert!(super::valid_name("ext:laptop"));
        for bad in ["", "a/b", "a b", &"x".repeat(101)] {
            assert!(!super::valid_name(bad), "{bad}");
        }
        let hub = super::Hub::default();
        hub.claim("peer:1", "a@x.io").unwrap();
        assert!(hub.claim("peer:1", "b@x.io").is_err());
        assert!(!hub.listening("peer:1"));
    }
}
