//! `kor chrome-native-host`: the native messaging relay (§18.1, ADR 0058).
//!
//! The browser starts this process when the extension calls `connectNative`, and talks to it over
//! stdin/stdout with 4-byte little-endian length-prefixed JSON. The host forwards every message to
//! the bridge WebSocket of the running Kor (desktop or CLI, `127.0.0.1:49201`) and every bridge
//! message back, so both paths speak one protocol and pairing is checked in one place. The host
//! itself answers only `{"type":"host","op":"status"}` (is Kor running?) and
//! `{"type":"host","op":"launch"}` (start the Kor desktop app: the side panel's one-click start).

use std::io;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Browsers accept at most 1 MB from a host.
const MAX_OUT: usize = 1024 * 1024;
/// And send at most 64 MB; Kor's bridge takes 32 MB.
const MAX_IN: u32 = 32 << 20;

/// 4-byte little-endian length plus the payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(payload);
    out
}

/// The `Origin` the bridge sees: Chrome passes `chrome-extension://<id>/` as the first argument,
/// Firefox the manifest path and the add-on id.
pub fn origin_from_args(args: &[String]) -> String {
    args.iter()
        .find_map(|a| a.strip_prefix("chrome-extension://"))
        .map_or_else(
            || "moz-extension://kor".to_owned(),
            |id| format!("chrome-extension://{}", id.trim_end_matches('/')),
        )
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Option<String>> {
    let mut header = [0u8; 4];
    match r.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(header);
    if len == 0 || len > MAX_IN {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad native message length"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "native message is not UTF-8"))
}

/// A bridge message too large for native messaging becomes an error with the same ids.
fn fit(text: String) -> String {
    if text.len() <= MAX_OUT {
        return text;
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let mut e = json!({"ok": false, "error": "message too large for native messaging; the direct connection carries it"});
    for k in ["type", "id", "rid"] {
        if !v[k].is_null() {
            e[k] = v[k].clone();
        }
    }
    if let Some(t) = e["type"].as_str()
        && t.ends_with("Reply")
    {
        e["result"] = json!({"error": e["error"].clone()});
    }
    e.to_string()
}

fn host(mut v: Value) -> String {
    v["type"] = json!("host");
    v.to_string()
}

type Sink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

async fn connect(port: u16, origin: &str, out: &mpsc::UnboundedSender<String>, closed: &mpsc::UnboundedSender<()>) -> Result<Sink, String> {
    let mut req = format!("ws://127.0.0.1:{port}")
        .into_client_request()
        .map_err(|e| e.to_string())?;
    req.headers_mut().insert(
        "origin",
        origin.parse().map_err(|_| "bad origin".to_owned())?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|_| "Kor is not running".to_owned())?;
    let (sink, mut source) = ws.split();
    let out = out.clone();
    let closed = closed.clone();
    tokio::spawn(async move {
        while let Some(Ok(m)) = source.next().await {
            if let Message::Text(t) = m {
                let _ = out.send(fit(t.as_str().to_owned()));
            }
        }
        let _ = closed.send(());
    });
    Ok(sink)
}

/// Relay until the browser closes stdin. `launch` starts Kor (desktop) for `op: "launch"`.
///
/// # Errors
/// Broken stdin/stdout framing.
pub async fn relay<R, W>(
    mut input: R,
    mut output: W,
    port: u16,
    origin: String,
    launch: fn() -> io::Result<()>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(s) = out_rx.recv().await {
            if output.write_all(&frame(s.as_bytes())).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<io::Result<String>>();
    tokio::spawn(async move {
        loop {
            match read_frame(&mut input).await {
                Ok(Some(s)) => {
                    if in_tx.send(Ok(s)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = in_tx.send(Err(e));
                    break;
                }
            }
        }
    });
    let (closed_tx, mut closed_rx) = mpsc::unbounded_channel::<()>();
    let mut ws: Option<Sink> = None;
    let result = loop {
        tokio::select! {
            msg = in_rx.recv() => {
                let text = match msg {
                    None => break Ok(()),
                    Some(Err(e)) => break Err(e),
                    Some(Ok(t)) => t,
                };
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "host" {
                    let reply = match v["op"].as_str() {
                        Some("launch") => match launch() {
                            Ok(()) => json!({"op": "launch", "launched": true}),
                            Err(e) => json!({"op": "launch", "launched": false, "error": e.to_string()}),
                        },
                        _ => {
                            if ws.is_none() {
                                ws = connect(port, &origin, &out_tx, &closed_tx).await.ok();
                            }
                            json!({"op": "status", "core": ws.is_some(), "version": env!("CARGO_PKG_VERSION")})
                        }
                    };
                    let _ = out_tx.send(host(reply));
                    continue;
                }
                if ws.is_none() {
                    match connect(port, &origin, &out_tx, &closed_tx).await {
                        Ok(s) => ws = Some(s),
                        Err(error) => {
                            let _ = out_tx.send(host(json!({"op": "status", "core": false, "error": error})));
                            continue;
                        }
                    }
                }
                if let Some(sink) = ws.as_mut()
                    && sink.send(Message::Text(text.into())).await.is_err()
                {
                    ws = None;
                }
            }
            Some(()) = closed_rx.recv() => {
                ws = None;
                let _ = out_tx.send(host(json!({"op": "status", "core": false})));
            }
        }
    };
    // The browser closed stdin: nobody reads what is still queued, and the bridge reader holds a
    // sender of its own, so the writer is stopped rather than awaited.
    drop(ws);
    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::chrome::pairing;
    use crate::tools::chrome::server::{Auth, Bridge};
    use std::sync::Arc;
    use std::time::Duration;

    async fn next_frame<R: AsyncRead + Unpin>(r: &mut R) -> Value {
        let s = tokio::time::timeout(Duration::from_secs(5), read_frame(r)).await.unwrap().unwrap().unwrap();
        serde_json::from_str(&s).unwrap()
    }

    fn no_launch() -> io::Result<()> {
        Err(io::ErrorKind::NotFound.into())
    }

    #[test]
    fn origins_and_oversized_messages() {
        assert_eq!(origin_from_args(&["chrome-extension://abc/".into(), "--parent-window=0".into()]), "chrome-extension://abc");
        assert_eq!(origin_from_args(&["/x/dev.kor.chrome_host.json".into(), "kor@kor.dev".into()]), "moz-extension://kor");
        let big = json!({"type": "clientReply", "rid": 4, "result": "x".repeat(MAX_OUT)}).to_string();
        let v: Value = serde_json::from_str(&fit(big)).unwrap();
        assert_eq!((v["rid"].as_u64(), v["result"]["error"].is_string()), (Some(4), true));
    }

    #[tokio::test]
    async fn relays_pairing_and_commands_and_reports_a_missing_core() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth { dir: dir.path().to_path_buf(), store: Arc::new(kor_secrets::MemoryStore::default()) };
        let bridge = Bridge::bind(0, auth).await.unwrap();
        let (code, _) = pairing::new_code(dir.path()).unwrap();
        let (mut browser, host_io) = tokio::io::duplex(1 << 20);
        let (host_in, host_out) = tokio::io::split(host_io);
        let port = bridge.port();
        let task = tokio::spawn(relay(host_in, host_out, port, "chrome-extension://abc".into(), no_launch));
        browser.write_all(&frame(json!({"type": "host", "op": "status"}).to_string().as_bytes())).await.unwrap();
        assert_eq!(next_frame(&mut browser).await["core"], true);
        browser.write_all(&frame(json!({"type": "pair", "code": code}).to_string().as_bytes())).await.unwrap();
        assert_eq!(next_frame(&mut browser).await["type"], "paired");
        // A tool command from Kor reaches the browser through the host and the answer comes back.
        let b2 = bridge.clone();
        let ask = tokio::spawn(async move { b2.request(json!({"cmd": "tabs.list"}), Duration::from_secs(5)).await });
        let cmd = next_frame(&mut browser).await;
        assert_eq!(cmd["cmd"], "tabs.list");
        browser.write_all(&frame(json!({"id": cmd["id"], "ok": true, "result": []}).to_string().as_bytes())).await.unwrap();
        assert_eq!(ask.await.unwrap().unwrap(), json!([]));
        browser.write_all(&frame(json!({"type": "host", "op": "launch"}).to_string().as_bytes())).await.unwrap();
        assert_eq!(next_frame(&mut browser).await["launched"], false);
        drop(browser);
        task.await.unwrap().unwrap();

        // No Kor on the port: the host answers `core: false` instead of hanging.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (mut browser, host_io) = tokio::io::duplex(1 << 16);
        let (host_in, host_out) = tokio::io::split(host_io);
        let task = tokio::spawn(relay(host_in, host_out, free, "chrome-extension://abc".into(), no_launch));
        browser.write_all(&frame(json!({"type": "pair", "key": "k"}).to_string().as_bytes())).await.unwrap();
        let v = next_frame(&mut browser).await;
        assert_eq!((v["type"].as_str(), v["core"].as_bool()), (Some("host"), Some(false)));
        drop(browser);
        task.await.unwrap().unwrap();
    }
}
