//! Pairing (`kor peer pair`, ADR 0057). The listening machine makes a one-time code
//! `KOR1-<id>-<secret>@<host:port>[,<host:port>…]` (4 + 16 characters of Crockford base32, 80
//! bits of secret, valid 10 minutes, one attempt) and shows it as text and a QR code; the other
//! machine joins with it. The Noise `XXpsk3` handshake succeeds only when both hold the same
//! secret, so a man in the middle cannot pair; each side then stores the other's public key.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::noise::{self, Link, Mode};
use super::{Book, Node, PeerEntry, hex};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// How long a code stays valid.
pub const TTL: Duration = Duration::from_secs(10 * 60);

/// A parsed pairing code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub id: [u8; 4],
    pub secret: String,
    pub addrs: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    secret: String,
    expires: u64,
}

fn random_chars(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter()
        .map(|x| ALPHABET[usize::from(x & 31)] as char)
        .collect())
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

impl Code {
    /// The pre-shared key both sides derive from the code.
    pub fn psk(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"kor-peer-pair-v1\0");
        h.update(self.id);
        h.update(self.secret.as_bytes());
        h.finalize().into()
    }

    pub fn render(&self) -> String {
        let s = &self.secret;
        format!(
            "KOR1-{}-{}-{}-{}-{}@{}",
            String::from_utf8_lossy(&self.id),
            &s[0..4],
            &s[4..8],
            &s[8..12],
            &s[12..16],
            self.addrs.join(",")
        )
    }

    /// # Errors
    /// On a malformed code.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || "not a Kor pairing code (KOR1-XXXX-XXXX-XXXX-XXXX-XXXX@host:port)".to_owned();
        let text = text.trim();
        let (left, addrs) = text.split_once('@').ok_or_else(bad)?;
        let left = left.trim();
        let rest = left
            .get(..4)
            .filter(|p| p.eq_ignore_ascii_case("kor1"))
            .and_then(|_| left.get(4..))
            .ok_or_else(bad)?;
        let body = normalize(rest);
        let body = body.as_str();
        if body.len() != 20 || !body.bytes().all(|b| ALPHABET.contains(&b)) {
            return Err(bad());
        }
        let id: [u8; 4] = body.as_bytes()[..4].try_into().map_err(|_| bad())?;
        let addrs: Vec<String> = addrs
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect();
        if addrs.is_empty()
            || addrs
                .iter()
                .any(|a| a.len() > 300 || a.contains(char::is_whitespace))
        {
            return Err(bad());
        }
        Ok(Self {
            id,
            secret: body[4..].to_owned(),
            addrs,
        })
    }
}

fn pending_path(dir: &Path, id: [u8; 4]) -> std::path::PathBuf {
    dir.join("pending")
        .join(format!("{}.json", String::from_utf8_lossy(&id)))
}

/// A new code for this machine, valid for [`TTL`], answered by `kor peer serve` (or the
/// listener `kor peer pair` starts itself).
///
/// # Errors
/// When the random source or the disk fails.
pub fn create(dir: &Path, addrs: Vec<String>) -> Result<Code, String> {
    let code = Code {
        id: random_chars(4)?.as_bytes().try_into().map_err(|_| "rng")?,
        secret: random_chars(16)?,
        addrs,
    };
    let p = pending_path(dir, code.id);
    std::fs::create_dir_all(p.parent().ok_or("bad dir")?).map_err(|e| e.to_string())?;
    let body = serde_json::to_vec(&Pending {
        secret: code.secret.clone(),
        expires: super::now_ms() + u64::try_from(TTL.as_millis()).unwrap_or(u64::MAX),
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(&p, body).map_err(|e| e.to_string())?;
    Ok(code)
}

/// Is code `id` still waiting (`kor peer pair` polls this to know when it was used)?
pub fn is_pending(dir: &Path, id: [u8; 4]) -> bool {
    pending_path(dir, id).is_file()
}

/// The pending code `id`, removed: one attempt per code.
fn take(dir: &Path, id: [u8; 4]) -> Result<Code, String> {
    let p = pending_path(dir, id);
    let b = std::fs::read(&p).map_err(|_| "unknown or used pairing code".to_owned())?;
    let _ = std::fs::remove_file(&p);
    let pend: Pending = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    if super::now_ms() > pend.expires {
        return Err("the pairing code expired".into());
    }
    Ok(Code {
        id,
        secret: pend.secret,
        addrs: Vec::new(),
    })
}

fn entry_from(remote: &[u8; 32], hello: &Value) -> PeerEntry {
    PeerEntry {
        name: hello["name"].as_str().unwrap_or("peer").to_owned(),
        key: hex(remote),
        addrs: hello["addrs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        ssh: None,
        grant: super::Grant::default(),
        paired_at: super::now_ms(),
    }
}

fn store(node: &Node, mut e: PeerEntry, ssh: Option<String>) -> Result<PeerEntry, String> {
    let mut book: Book = node.book()?;
    e.ssh = ssh;
    let name = book.upsert(e);
    book.save(&node.dir)?;
    book.get(&name)
        .cloned()
        .ok_or_else(|| "peer not saved".into())
}

/// Listening side of a pairing connection whose prelude named code `id`.
///
/// # Errors
/// Unknown/expired/used code or a failed handshake (wrong secret).
pub async fn accept<S>(node: &Node, id: [u8; 4], io: S) -> Result<PeerEntry, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let code = take(&node.dir, id)?;
    let link = noise::respond(
        io,
        &node.identity,
        Some(&code.psk()),
        &node.hello()?,
        |_, _| Ok(()),
    )
    .await?;
    finish(node, link, None).await
}

async fn finish(node: &Node, mut link: Link, ssh: Option<String>) -> Result<PeerEntry, String> {
    let entry = entry_from(&link.remote, &link.hello);
    link.send(json!({ "type": "paired", "as": entry.name }))?;
    // SECURITY: the pre-shared secret is mixed in at the handshake's last message, so the joining
    // side only learns of a wrong secret when the other side drops the link. Nothing is stored
    // until the other side has confirmed the pairing too.
    let reply = link.recv_within(Duration::from_secs(10)).await?;
    if reply["type"] != "paired" {
        return Err("the other machine did not confirm the pairing".into());
    }
    let e = store(node, entry, ssh)?;
    super::audit(
        "paired",
        &e.name,
        json!({ "fingerprint": super::fingerprint(&link.remote) }),
    );
    Ok(e)
}

/// Joining side: dial the code's addresses (or `ssh`), pair, store the listener as a peer.
///
/// # Errors
/// When no address answers or the handshake fails.
pub async fn join(node: &Node, code: &Code, ssh: Option<&str>) -> Result<PeerEntry, String> {
    let psk = code.psk();
    let hello = node.hello()?;
    let mut errors = Vec::new();
    if let Some(target) = ssh {
        match super::client::ssh_stream(target) {
            Ok(io) => {
                let link = noise::initiate(
                    io,
                    &node.identity,
                    Mode::Pair { id: code.id },
                    Some(&psk),
                    &hello,
                )
                .await?;
                return finish(node, link, Some(target.to_owned())).await;
            }
            Err(e) => errors.push(e),
        }
    }
    for a in &code.addrs {
        let addr = a.strip_prefix("ts:").unwrap_or(a);
        match super::client::tcp(addr).await {
            Ok(io) => {
                let link = noise::initiate(
                    io,
                    &node.identity,
                    Mode::Pair { id: code.id },
                    Some(&psk),
                    &hello,
                )
                .await?;
                return finish(node, link, None).await;
            }
            Err(e) => errors.push(format!("{addr}: {e}")),
        }
    }
    Err(format!(
        "could not reach the pairing machine ({})",
        errors.join("; ")
    ))
}

/// The code as a terminal QR code (two rows per character).
pub fn qr_text(code: &Code) -> String {
    qrcode::QrCode::new(code.render().as_bytes()).map_or_else(
        |_| String::new(),
        |q| {
            q.render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build()
        },
    )
}

/// The code's QR modules (`true` = dark), row by row, and the side length — the desktop draws it.
pub fn qr_matrix(code: &Code) -> (usize, Vec<bool>) {
    qrcode::QrCode::new(code.render().as_bytes()).map_or((0, Vec::new()), |q| {
        let w = q.width();
        let cells = q
            .to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect();
        (w, cells)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn codes_render_parse_and_tolerate_typing() {
        let d = tempfile::tempdir().unwrap();
        let c = create(
            d.path(),
            vec!["192.168.1.5:47321".into(), "ts:100.64.0.2:47321".into()],
        )
        .unwrap();
        let text = c.render();
        assert!(text.starts_with("KOR1-"));
        let back = Code::parse(&text).unwrap();
        assert_eq!(back, c);
        assert_eq!(Code::parse(&text.to_lowercase()).unwrap().secret, c.secret);
        assert!(Code::parse("KOR1-AAAA@x").is_err());
        assert!(Code::parse("hello").is_err());
        assert!(is_pending(d.path(), c.id));
        let t = take(d.path(), c.id).unwrap();
        assert_eq!(t.psk(), c.psk());
        assert!(take(d.path(), c.id).is_err(), "one attempt per code");
        let (w, m) = qr_matrix(&c);
        assert!(w > 20 && m.len() == w * w);
        assert!(!qr_text(&c).is_empty());
    }
}
