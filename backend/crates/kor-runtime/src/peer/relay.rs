//! The team-server relay route (§31.1 "yoksa takım sunucusu röle olur"): when no direct,
//! Tailscale or SSH route answers, the Noise session to a peer travels through the team server
//! (`[team] server`), which only forwards ciphertext. The serving machine parks a WebSocket on
//! `relay/listen/peer:<its key>`; the dialling one opens `relay/dial/peer:<that key>`. The same
//! listener also bridges the browser extension of a remote machine to this machine's local Kor
//! bridge (`ext:<name>`, §18.4). TLS trusts the team CA and presents the member certificate
//! from the keychain (`kor server join`); the member token goes in the `Authorization` header.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt as _, StreamExt as _};
use kor_agent::team_client::Material;
use kor_secrets::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::rustls;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use super::Node;

const CONNECT: Duration = Duration::from_secs(8);
/// Local Kor bridge the extension talks to (`frontend/src/extension/transport.ts`).
pub const LOCAL_BRIDGE: &str = "ws://127.0.0.1:49201";

/// The relay name of a peer key.
pub fn peer_name(key_hex: &str) -> String {
    format!("peer:{key_hex}")
}

fn tls(m: &Material) -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = || -> Result<rustls::RootCertStore, String> {
        use rustls::pki_types::pem::PemObject as _;
        let mut r = rustls::RootCertStore::empty();
        if let Some(ca) = &m.ca {
            for c in rustls::pki_types::CertificateDer::pem_slice_iter(ca.as_bytes()) {
                r.add(c.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(r)
    };
    let base = if m.ca.is_some() {
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots()?)
    } else {
        // A server behind a public certificate: the OS trust store.
        use rustls_platform_verifier::BuilderVerifierExt as _;
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_platform_verifier()
            .map_err(|e| e.to_string())?
    };
    let cfg = match (&m.cert, &m.key) {
        (Some(cert), Some(key)) => {
            use rustls::pki_types::pem::PemObject as _;
            let chain = rustls::pki_types::CertificateDer::pem_slice_iter(cert.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let key =
                rustls::pki_types::PrivateKeyDer::from_pem_slice(key.expose_secret().as_bytes())
                    .map_err(|e| e.to_string())?;
            base.with_client_auth_cert(chain, key)
                .map_err(|e| e.to_string())?
        }
        _ => base.with_no_client_auth(),
    };
    Ok(Arc::new(cfg))
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

/// Open `relay/<mode>/<name>` on `server` (`https://host[:port]`).
///
/// # Errors
/// Refused by privacy/offline mode, not reachable, or refused by the server.
pub async fn open(server: &str, m: &Material, mode: &str, name: &str) -> Result<Ws, String> {
    let u = reqwest::Url::parse(server).map_err(|_| "bad team server address")?;
    if u.scheme() != "https" {
        return Err("the team server must be https://".into());
    }
    let host = u.host_str().ok_or("bad team server address")?.to_owned();
    let port = u.port_or_known_default().unwrap_or(443);
    let tcp = super::client::tcp(&format!("{host}:{port}")).await?;
    let sni = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| e.to_string())?;
    let stream = tokio::time::timeout(
        CONNECT,
        tokio_rustls::TlsConnector::from(tls(m)?).connect(sni, tcp),
    )
    .await
    .map_err(|_| "TLS timed out".to_owned())?
    .map_err(|e| format!("TLS: {e}"))?;
    let url = format!("wss://{host}:{port}/api/v1/relay/{mode}/{name}");
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    if let Some(t) = &m.token {
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", t.expose_secret())
                .parse()
                .map_err(|_| "bad token")?,
        );
    }
    let (ws, _) = tokio::time::timeout(CONNECT, tokio_tungstenite::client_async(req, stream))
        .await
        .map_err(|_| "relay timed out".to_owned())?
        .map_err(|e| format!("relay: {e}"))?;
    Ok(ws)
}

/// Binary WebSocket messages as one byte stream (what the Noise layer reads and writes).
fn bytes_over<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> tokio::io::DuplexStream
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (ours, mut theirs) = tokio::io::duplex(256 * 1024);
    tokio::spawn(async move {
        let (mut tx, mut rx) = ws.split();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            tokio::select! {
                m = rx.next() => match m {
                    Some(Ok(Message::Binary(b))) => {
                        if theirs.write_all(&b).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
                n = theirs.read(&mut buf) => match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() { break; }
                    }
                },
            }
        }
        let _ = tx.close().await;
    });
    ours
}

/// A byte stream to the peer with `key_hex` through the team server.
///
/// # Errors
/// As [`open`].
pub async fn dial(
    server: &str,
    m: &Material,
    key_hex: &str,
) -> Result<tokio::io::DuplexStream, String> {
    let ws = open(server, m, "dial", &peer_name(key_hex)).await?;
    Ok(bytes_over(ws))
}

/// Wait on the relay for the next caller: `Ok(stream)` once somebody dialled.
async fn next_caller(server: &str, m: &Material, name: &str) -> Result<Ws, String> {
    let mut ws = open(server, m, "listen", name).await?;
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) if t.contains("\"connected\"") => return Ok(ws),
            Some(Ok(Message::Close(_)) | Err(_)) | None => return Err("relay closed".into()),
            Some(Ok(_)) => {}
        }
    }
}

/// Answer paired peers through the relay until the process ends (reconnects with backoff).
pub async fn serve(node: Arc<Node>, server: String, m: Arc<Material>) {
    let name = peer_name(&super::hex(&node.identity.public));
    let mut wait = Duration::from_secs(1);
    loop {
        if let Ok(ws) = next_caller(&server, &m, &name).await {
            wait = Duration::from_secs(1);
            let n = node.clone();
            tokio::spawn(async move {
                let _ = super::server::serve_conn(&n, bytes_over(ws)).await;
            });
        } else {
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
        }
    }
}

/// Bridge remote extensions (`ext:<name>`) to this machine's local Kor bridge: every message is
/// forwarded unchanged both ways, so pairing with the bridge still happens end to end.
pub async fn serve_extension(server: String, m: Arc<Material>, name: String, local: String) {
    let mut wait = Duration::from_secs(1);
    loop {
        let Ok(remote) = next_caller(&server, &m, &format!("ext:{name}")).await else {
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
            continue;
        };
        wait = Duration::from_secs(1);
        let local = local.clone();
        tokio::spawn(async move {
            let Ok((near, _)) = tokio_tungstenite::connect_async(local.as_str()).await else {
                return;
            };
            let (mut a_tx, mut a_rx) = remote.split();
            let (mut b_tx, mut b_rx) = near.split();
            let data = |m: &Message| matches!(m, Message::Text(_) | Message::Binary(_));
            let ab = async {
                while let Some(Ok(m)) = a_rx.next().await {
                    if matches!(m, Message::Close(_)) || (data(&m) && b_tx.send(m).await.is_err()) {
                        break;
                    }
                }
            };
            let ba = async {
                while let Some(Ok(m)) = b_rx.next().await {
                    if matches!(m, Message::Close(_)) || (data(&m) && a_tx.send(m).await.is_err()) {
                        break;
                    }
                }
            };
            tokio::select! {
                () = ab => {}
                () = ba => {}
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binary_messages_become_a_byte_stream() {
        let (a, b) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            let mut ws = tokio_tungstenite::accept_async(b).await.unwrap();
            ws.send(Message::Text("ignored".into())).await.unwrap();
            ws.send(Message::Binary(b"hello ".to_vec().into()))
                .await
                .unwrap();
            ws.send(Message::Binary(b"peer".to_vec().into()))
                .await
                .unwrap();
            let back = ws.next().await.unwrap().unwrap();
            assert_eq!(back.into_data().as_ref(), b"pong");
        });
        let (ws, _) = tokio_tungstenite::client_async("ws://relay.invalid/x", a)
            .await
            .unwrap();
        let mut s = bytes_over(ws);
        let mut got = [0u8; 10];
        s.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello peer");
        s.write_all(b"pong").await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn http_servers_are_refused() {
        let m = Material::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let e = rt
            .block_on(open("http://kor.example.com", &m, "dial", "peer:00"))
            .unwrap_err();
        assert!(e.contains("https"), "{e}");
    }
}
