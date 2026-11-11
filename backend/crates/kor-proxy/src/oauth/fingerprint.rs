//! Claude Code 2.1.220 OAuth ClientHello.
//!
//! The JA3 string is the one captured from the native Axios control-plane handshake
//! (`771,4865-4866-4867-49195-49199-49196-49200-52393-52392-49161-49171-49162-49172-156-157-47-53,0-23-65281-10-11-35-13-51-45-43,29-23-24,0`).
//! BoringSSL is configured to advertise that cipher list, curve list and extension order, with
//! GREASE and ALPN left off, which is what the OAuth token host expects.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use wreq::tls::{AlpnProtocol, ExtensionType, KeyShare, TlsOptions, TlsVersion};

use super::{Error, err};

#[allow(dead_code)] // kept for reference and tests
pub(super) const OAUTH_JA3: &str = "771,4865-4866-4867-49195-49199-49196-49200-52393-52392-49161-49171-49162-49172-156-157-47-53,0-23-65281-10-11-35-13-51-45-43,29-23-24,0";

const CIPHERS: &str = "TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:\
ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:\
ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:\
ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES256-SHA:ECDHE-RSA-AES256-SHA:\
AES128-GCM-SHA256:AES256-GCM-SHA384:AES128-SHA:AES256-SHA";

const CURVES: &str = "X25519:P-256:P-384";
const SIGALGS: &str = "ecdsa_secp256r1_sha256:rsa_pss_rsae_sha256:rsa_pkcs1_sha256:\
ecdsa_secp384r1_sha384:rsa_pss_rsae_sha384:rsa_pkcs1_sha384:rsa_pss_rsae_sha512:rsa_pkcs1_sha512:\
rsa_pkcs1_sha1";

fn oauth_extensions() -> [ExtensionType; 10] {
    [
        ExtensionType::SERVER_NAME,
        ExtensionType::EXTENDED_MASTER_SECRET,
        ExtensionType::RENEGOTIATE,
        ExtensionType::SUPPORTED_GROUPS,
        ExtensionType::EC_POINT_FORMATS,
        ExtensionType::SESSION_TICKET,
        ExtensionType::SIGNATURE_ALGORITHMS,
        ExtensionType::KEY_SHARE,
        ExtensionType::PSK_KEY_EXCHANGE_MODES,
        ExtensionType::SUPPORTED_VERSIONS,
    ]
}

pub(super) fn oauth_tls() -> TlsOptions {
    TlsOptions::builder()
        .alpn_protocols(std::iter::empty::<AlpnProtocol>())
        .min_tls_version(TlsVersion::TLS_1_2)
        .max_tls_version(TlsVersion::TLS_1_3)
        .grease_enabled(false)
        .permute_extensions(false)
        .enable_ech_grease(false)
        .enable_ocsp_stapling(false)
        .enable_signed_cert_timestamps(false)
        .session_ticket(true)
        .renegotiation(true)
        .psk_dhe_ke(true)
        .pre_shared_key(false)
        .curves_list(CURVES)
        .sigalgs_list(SIGALGS)
        .cipher_list(CIPHERS)
        .preserve_tls13_cipher_list(true)
        .key_shares(vec![KeyShare::X25519])
        .extension_permutation(oauth_extensions().to_vec())
        .build()
}

/// Messages use the same handshake plus HTTP/1.1 ALPN, which is the inference profile's protocol.
pub(super) fn messages_tls() -> TlsOptions {
    TlsOptions::builder()
        .alpn_protocols([AlpnProtocol::HTTP1])
        .min_tls_version(TlsVersion::TLS_1_2)
        .max_tls_version(TlsVersion::TLS_1_3)
        .grease_enabled(false)
        .permute_extensions(false)
        .enable_ech_grease(false)
        .enable_ocsp_stapling(true)
        .enable_signed_cert_timestamps(true)
        .session_ticket(true)
        .renegotiation(true)
        .psk_dhe_ke(true)
        .pre_shared_key(false)
        .curves_list(CURVES)
        .sigalgs_list(SIGALGS)
        .cipher_list(CIPHERS)
        .preserve_tls13_cipher_list(true)
        .key_shares(vec![KeyShare::X25519])
        .extension_permutation(vec![
            ExtensionType::SERVER_NAME,
            ExtensionType::EXTENDED_MASTER_SECRET,
            ExtensionType::RENEGOTIATE,
            ExtensionType::SUPPORTED_GROUPS,
            ExtensionType::EC_POINT_FORMATS,
            ExtensionType::SESSION_TICKET,
            ExtensionType::APPLICATION_LAYER_PROTOCOL_NEGOTIATION,
            ExtensionType::STATUS_REQUEST,
            ExtensionType::SIGNATURE_ALGORITHMS,
            ExtensionType::CERTIFICATE_TIMESTAMP,
            ExtensionType::KEY_SHARE,
            ExtensionType::PSK_KEY_EXCHANGE_MODES,
            ExtensionType::SUPPORTED_VERSIONS,
        ])
        .build()
}

fn shared(messages: bool) -> &'static wreq::Client {
    if messages {
        static CLIENT: OnceLock<wreq::Client> = OnceLock::new();
        CLIENT.get_or_init(|| build(messages_tls(), true).expect("claude messages tls client"))
    } else {
        static CLIENT: OnceLock<wreq::Client> = OnceLock::new();
        CLIENT.get_or_init(|| build(oauth_tls(), false).expect("claude oauth tls client"))
    }
}

pub(super) fn build(tls: TlsOptions, http1: bool) -> Result<wreq::Client, Error> {
    let mut builder = wreq::Client::builder().tls_options(tls).timeout(Duration::from_secs(30));
    if http1 {
        builder = builder.http1_only();
    }
    builder.build().map_err(|e| err(format!("claude tls client: {e}")))
}

pub(super) async fn post_json(url: &str, body: &impl Serialize, headers: &[(&str, &str)]) -> Result<(u16, String), Error> {
    let payload = serde_json::to_string(body).map_err(|e| err(e.to_string()))?;
    let mut request = shared(false).post(url).header("content-type", "application/json").body(payload);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    send(request).await
}

pub(super) async fn get(url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), Error> {
    let mut request = shared(false).get(url);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    send(request).await
}

pub(super) fn messages_client() -> &'static wreq::Client {
    shared(true)
}

async fn send(request: wreq::RequestBuilder) -> Result<(u16, String), Error> {
    let response = request.send().await.map_err(|e| err(format!("claude tls request: {e}")))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| err(format!("claude tls response: {e}")))?;
    Ok((status, body))
}

#[allow(dead_code)] // kept for reference and tests
pub(super) fn ja3(record: &[u8]) -> Option<String> {
    if record.len() < 5 || record[0] != 22 {
        return None;
    }
    let hello = &record[5..];
    if hello.first() != Some(&1) || hello.len() < 38 {
        return None;
    }
    let mut index = 1 + 3 + 2 + 32;
    let session = *hello.get(index)? as usize;
    index += 1 + session;
    let cipher_len = u16::from_be_bytes(hello.get(index..index + 2)?.try_into().ok()?) as usize;
    index += 2;
    let ciphers = hello.get(index..index + cipher_len)?;
    index += cipher_len;
    let comp_len = *hello.get(index)? as usize;
    index += 1 + comp_len;
    let ext_len = u16::from_be_bytes(hello.get(index..index + 2)?.try_into().ok()?) as usize;
    index += 2;
    let mut extensions = hello.get(index..index + ext_len)?;
    let mut ext_ids = Vec::new();
    let mut curves = String::new();
    let mut points = String::new();
    while extensions.len() >= 4 {
        let kind = u16::from_be_bytes([extensions[0], extensions[1]]);
        let len = u16::from_be_bytes([extensions[2], extensions[3]]) as usize;
        if extensions.len() < 4 + len {
            return None;
        }
        let data = &extensions[4..4 + len];
        extensions = &extensions[4 + len..];
        if grease(kind) {
            continue;
        }
        ext_ids.push(kind.to_string());
        if kind == 10 {
            curves = groups(data);
        } else if kind == 11 {
            points = data.iter().skip(1).map(|b| b.to_string()).collect::<Vec<_>>().join("-");
        }
    }
    let cipher_ids = ciphers
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .filter(|id| !grease(*id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");
    let version = u16::from_be_bytes([hello[4], hello[5]]);
    Some(format!("{version},{cipher_ids},{},{curves},{points}", ext_ids.join("-")))
}

#[allow(dead_code)] // kept for reference and tests
fn groups(data: &[u8]) -> String {
    let list = data.get(2..).unwrap_or_default();
    list.chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .filter(|id| !grease(*id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-")
}

#[allow(dead_code)] // kept for reference and tests
fn grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn oauth_client_hello_matches_the_claude_code_capture() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = wreq::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap();
        let client = wreq::Client::builder()
            .tls_options(oauth_tls())
            .timeout(Duration::from_secs(3))
            .proxy(proxy)
            .build()
            .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = client.get("https://platform.claude.com/").send().await;
            let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
        });
        let accepted = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
        let Ok(Ok((mut sock, _))) = accepted else {
            let why = rx.await.unwrap_or_else(|_| Err("client task dropped".into()));
            panic!("proxy accepted no connection: {why:?}");
        };
        let mut prefix = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            sock.read_exact(&mut byte).await.unwrap();
            prefix.push(byte[0]);
            if prefix.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        use tokio::io::AsyncWriteExt;
        sock.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
        let mut header = [0u8; 5];
        sock.read_exact(&mut header).await.unwrap();
        let len = u16::from_be_bytes([header[3], header[4]]) as usize;
        let mut body = vec![0u8; len];
        sock.read_exact(&mut body).await.unwrap();
        let mut record = header.to_vec();
        record.extend(body);
        assert_eq!(ja3(&record).as_deref(), Some(OAUTH_JA3));
    }
}
