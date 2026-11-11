//! Encrypted, mutually authenticated sessions between Kor peers (ADR 0057): Noise XX with each
//! machine's long-term X25519 key (`Noise_XX_25519_ChaChaPoly_BLAKE2s`), and for pairing
//! `Noise_XXpsk3_…` keyed by the one-time pairing code. Works over any byte stream: TCP (direct,
//! Tailscale) or `ssh … kor peer serve --stdio`.
//!
//! Wire: a 10-byte prelude (`KOR1`, mode `S`/`P`, 4-byte pairing id, 0) in clear, then Noise messages as `u16` big-endian
//! length + bytes. Application frames are JSON values split into chunks (`1` = more follows,
//! `0` = last), at most [`MAX_FRAME`] bytes per value.

use std::time::Duration;

use serde_json::Value;
use snow::{Builder, HandshakeState, TransportState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use super::Identity;

pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
pub const PAIR_PATTERN: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";
/// Largest JSON value one side accepts (a project snapshot fits).
pub const MAX_FRAME: usize = 96 * 1024 * 1024;
const MAX_MSG: usize = 65_535;
/// Plaintext per Noise message: the 16-byte tag and the 1-byte chunk flag leave this much.
const CHUNK: usize = 65_000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const MAGIC: &[u8; 4] = b"KOR1";

/// What the connection is for, sent in clear before the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A session between paired peers.
    Session,
    /// Pairing with a one-time code; `id` picks the pending code on the listener.
    Pair { id: [u8; 4] },
}

/// An established session: JSON values in and out, and who is on the other end.
#[derive(Debug)]
pub struct Link {
    /// The other side's long-term public key (authenticated by the handshake).
    pub remote: [u8; 32],
    /// The other side's handshake payload (name, addresses, protocol).
    pub hello: Value,
    pub tx: mpsc::UnboundedSender<Value>,
    pub rx: mpsc::UnboundedReceiver<Value>,
}

impl Link {
    /// # Errors
    /// When the connection is gone.
    pub fn send(&self, v: Value) -> Result<(), String> {
        self.tx
            .send(v)
            .map_err(|_| "peer connection closed".to_owned())
    }

    /// Next value; `None` once the connection closed.
    pub async fn recv(&mut self) -> Option<Value> {
        self.rx.recv().await
    }

    /// Next value within `t`.
    ///
    /// # Errors
    /// On timeout or a closed connection.
    pub async fn recv_within(&mut self, t: Duration) -> Result<Value, String> {
        tokio::time::timeout(t, self.rx.recv())
            .await
            .map_err(|_| "peer did not answer in time".to_owned())?
            .ok_or_else(|| "peer closed the connection".to_owned())
    }
}

fn params(pattern: &str) -> Result<snow::params::NoiseParams, String> {
    pattern.parse().map_err(|e| format!("noise params: {e:?}"))
}

/// A new long-term key pair.
///
/// # Errors
/// When the system random source fails.
pub fn generate() -> Result<([u8; 32], [u8; 32]), String> {
    let kp = Builder::new(params(PATTERN)?)
        .generate_keypair()
        .map_err(|e| e.to_string())?;
    let private: [u8; 32] = kp.private.as_slice().try_into().map_err(|_| "bad key")?;
    let public: [u8; 32] = kp.public.as_slice().try_into().map_err(|_| "bad key")?;
    Ok((private, public))
}

async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, b: &[u8]) -> Result<(), String> {
    let len = u16::try_from(b.len()).map_err(|_| "noise message too long")?;
    w.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| e.to_string())?;
    w.write_all(b).await.map_err(|e| e.to_string())?;
    w.flush().await.map_err(|e| e.to_string())
}

async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, String> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let mut b = vec![0u8; usize::from(u16::from_be_bytes(len))];
    r.read_exact(&mut b).await.map_err(|e| e.to_string())?;
    Ok(b)
}

fn builder<'a>(
    pattern: &str,
    id: &'a Identity,
    psk: Option<&'a [u8; 32]>,
) -> Result<Builder<'a>, String> {
    let mut b = Builder::new(params(pattern)?).local_private_key(id.private_bytes());
    if let Some(k) = psk {
        b = b.psk(3, k);
    }
    Ok(b)
}

fn remote_key(hs: &HandshakeState) -> Result<[u8; 32], String> {
    hs.get_remote_static()
        .and_then(|k| <[u8; 32]>::try_from(k).ok())
        .ok_or_else(|| "peer sent no key".to_owned())
}

/// Dial side: prelude, then `-> e`, `<- e ee s es (hello)`, `-> s se (hello)`.
///
/// # Errors
/// On a failed handshake (wrong pairing code, wrong key) or I/O errors.
pub async fn initiate<S>(
    mut io: S,
    id: &Identity,
    mode: Mode,
    psk: Option<&[u8; 32]>,
    hello: &Value,
) -> Result<Link, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let pattern = if psk.is_some() { PAIR_PATTERN } else { PATTERN };
    let mut hs = builder(pattern, id, psk)?
        .build_initiator()
        .map_err(|e| e.to_string())?;
    let run = async {
        let mut prelude = [0u8; 10];
        prelude[..4].copy_from_slice(MAGIC);
        match mode {
            Mode::Session => prelude[4] = b'S',
            Mode::Pair { id } => {
                prelude[4] = b'P';
                prelude[5..9].copy_from_slice(&id);
            }
        }
        io.write_all(&prelude).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; MAX_MSG];
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        write_msg(&mut io, &buf[..n]).await?;
        let m = read_msg(&mut io).await?;
        let mut payload = vec![0u8; MAX_MSG];
        let n = hs
            .read_message(&m, &mut payload)
            .map_err(|_| "handshake failed (wrong pairing code or key)".to_owned())?;
        let their: Value = serde_json::from_slice(&payload[..n]).unwrap_or(Value::Null);
        let mine = serde_json::to_vec(hello).map_err(|e| e.to_string())?;
        let n = hs
            .write_message(&mine, &mut buf)
            .map_err(|e| e.to_string())?;
        write_msg(&mut io, &buf[..n]).await?;
        Ok::<_, String>(their)
    };
    let their = tokio::time::timeout(HANDSHAKE_TIMEOUT, run)
        .await
        .map_err(|_| "peer handshake timed out".to_owned())??;
    let remote = remote_key(&hs)?;
    let ts = hs.into_transport_mode().map_err(|e| e.to_string())?;
    Ok(start(io, ts, remote, their))
}

/// The clear-text prelude of an incoming connection.
///
/// # Errors
/// When it is not a Kor peer connection.
pub async fn read_prelude<S: AsyncRead + Unpin>(io: &mut S) -> Result<Mode, String> {
    let mut p = [0u8; 10];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, io.read_exact(&mut p))
        .await
        .map_err(|_| "no prelude".to_owned())?
        .map_err(|e| e.to_string())?;
    if &p[..4] != MAGIC {
        return Err("not a Kor peer connection".into());
    }
    match p[4] {
        b'S' => Ok(Mode::Session),
        b'P' => Ok(Mode::Pair {
            id: [p[5], p[6], p[7], p[8]],
        }),
        _ => Err("unknown connection mode".into()),
    }
}

/// Listen side after [`read_prelude`]. `accept` sees the dialer's key and hello once the
/// handshake proved them, and refuses unknown keys (fail closed).
///
/// # Errors
/// On a failed handshake, I/O errors or a refusal from `accept`.
pub async fn respond<S>(
    mut io: S,
    id: &Identity,
    psk: Option<&[u8; 32]>,
    hello: &Value,
    accept: impl FnOnce(&[u8; 32], &Value) -> Result<(), String>,
) -> Result<Link, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let pattern = if psk.is_some() { PAIR_PATTERN } else { PATTERN };
    let mut hs = builder(pattern, id, psk)?
        .build_responder()
        .map_err(|e| e.to_string())?;
    let run = async {
        let mut buf = vec![0u8; MAX_MSG];
        let mut payload = vec![0u8; MAX_MSG];
        let m = read_msg(&mut io).await?;
        hs.read_message(&m, &mut payload)
            .map_err(|_| "handshake failed".to_owned())?;
        let mine = serde_json::to_vec(hello).map_err(|e| e.to_string())?;
        let n = hs
            .write_message(&mine, &mut buf)
            .map_err(|e| e.to_string())?;
        write_msg(&mut io, &buf[..n]).await?;
        let m = read_msg(&mut io).await?;
        let n = hs
            .read_message(&m, &mut payload)
            .map_err(|_| "handshake failed (wrong pairing code or key)".to_owned())?;
        Ok::<_, String>(serde_json::from_slice::<Value>(&payload[..n]).unwrap_or(Value::Null))
    };
    let their = tokio::time::timeout(HANDSHAKE_TIMEOUT, run)
        .await
        .map_err(|_| "peer handshake timed out".to_owned())??;
    let remote = remote_key(&hs)?;
    accept(&remote, &their)?;
    let ts = hs.into_transport_mode().map_err(|e| e.to_string())?;
    Ok(start(io, ts, remote, their))
}

/// Reader task (raw messages) + pump task (decrypt in, encrypt out).
fn start<S>(io: S, mut ts: TransportState, remote: [u8; 32], hello: Value) -> Link
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(io);
    let (raw_tx, mut raw_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, in_rx) = mpsc::unbounded_channel::<Value>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Ok(m) = read_msg(&mut rd).await {
            if raw_tx.send(m).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut acc: Vec<u8> = Vec::new();
        let mut plain = vec![0u8; MAX_MSG];
        let mut cipher = vec![0u8; MAX_MSG];
        loop {
            tokio::select! {
                m = raw_rx.recv() => {
                    let Some(m) = m else { break };
                    let Ok(n) = ts.read_message(&m, &mut plain) else { break };
                    if n == 0 || acc.len() + n > MAX_FRAME {
                        break;
                    }
                    acc.extend_from_slice(&plain[1..n]);
                    if plain[0] == 0 {
                        let frame = std::mem::take(&mut acc);
                        match serde_json::from_slice::<Value>(&frame) {
                            Ok(v) => {
                                if in_tx.send(v).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                v = out_rx.recv() => {
                    let Some(v) = v else { break };
                    let Ok(bytes) = serde_json::to_vec(&v) else { continue };
                    if bytes.len() > MAX_FRAME {
                        continue;
                    }
                    let mut chunks = bytes.chunks(CHUNK).peekable();
                    let mut failed = bytes.is_empty();
                    while let Some(c) = chunks.next() {
                        let mut p = Vec::with_capacity(c.len() + 1);
                        p.push(u8::from(chunks.peek().is_some()));
                        p.extend_from_slice(c);
                        let Ok(n) = ts.write_message(&p, &mut cipher) else { failed = true; break };
                        if write_msg(&mut wr, &cipher[..n]).await.is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if failed {
                        break;
                    }
                }
            }
        }
        let _ = wr.shutdown().await;
    });
    Link {
        remote,
        hello,
        tx: out_tx,
        rx: in_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    #[allow(clippy::many_single_char_names)]
    async fn session_and_pairing_handshakes_authenticate_both_sides() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let (x, y) = tokio::io::duplex(1 << 20);
        let b_pub = b.public;
        let a_pub = a.public;
        let server = tokio::spawn(async move {
            let mut y = y;
            let mode = read_prelude(&mut y).await.unwrap();
            assert_eq!(mode, Mode::Session);
            respond(y, &b, None, &json!({"name": "b"}), |k, h| {
                assert_eq!(*k, a_pub);
                assert_eq!(h["name"], "a");
                Ok(())
            })
            .await
            .unwrap()
        });
        let mut la = initiate(x, &a, Mode::Session, None, &json!({"name": "a"}))
            .await
            .unwrap();
        let mut lb = server.await.unwrap();
        assert_eq!(la.remote, b_pub);
        assert_eq!(la.hello["name"], "b");
        // A frame bigger than one Noise message is chunked and reassembled.
        let big = "x".repeat(200_000);
        la.send(json!({"type": "big", "data": big})).unwrap();
        let got = lb.recv().await.unwrap();
        assert_eq!(got["data"].as_str().unwrap().len(), 200_000);
        lb.send(json!({"ok": true})).unwrap();
        assert_eq!(la.recv().await.unwrap()["ok"], true);

        // Pairing: the same code on both sides works, a wrong one fails.
        let c = Identity::generate().unwrap();
        let d = Identity::generate().unwrap();
        for (left, right, ok) in [([7u8; 32], [7u8; 32], true), ([7u8; 32], [8u8; 32], false)] {
            let (x, y) = tokio::io::duplex(1 << 16);
            let d2 = d.clone();
            let srv = tokio::spawn(async move {
                let mut y = y;
                let _ = read_prelude(&mut y).await;
                respond(y, &d2, Some(&right), &json!({}), |_, _| Ok(())).await
            });
            let r = initiate(x, &c, Mode::Pair { id: *b"abcd" }, Some(&left), &json!({})).await;
            let s = srv.await.unwrap();
            assert_eq!(r.is_ok() && s.is_ok(), ok, "{:?} {:?}", r.err(), s.err());
        }
    }

    #[tokio::test]
    async fn unknown_keys_are_refused() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let (x, y) = tokio::io::duplex(1 << 16);
        let srv = tokio::spawn(async move {
            let mut y = y;
            let _ = read_prelude(&mut y).await;
            respond(y, &b, None, &json!({}), |_, _| Err("unknown peer".into())).await
        });
        let _ = initiate(x, &a, Mode::Session, None, &json!({})).await;
        assert_eq!(srv.await.unwrap().unwrap_err(), "unknown peer");
    }
}
