//! SSL certificate expiry of any site (§29.1 inventory, §29.13 report): a TLS handshake on port 443
//! that only reads the server's certificate (nothing is sent after the handshake). Works for every
//! panel the same way. The host must pass Kor's network guard first. Whether browsers trust the
//! certificate is checked separately by a normal HTTPS request (`trusted`).

use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use super::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertInfo {
    pub domain: String,
    /// Unix seconds.
    pub not_after: i64,
    pub issuer: String,
    pub subject: String,
}

impl CertInfo {
    pub fn days_left(&self, now: i64) -> i64 {
        (self.not_after - now).div_euclid(86_400)
    }
}

/// Reads the certificate, never judges it (see the module comment).
#[derive(Debug)]
struct ReadOnly(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for ReadOnly {
    fn verify_server_cert(
        &self,
        _end: &CertificateDer<'_>,
        _inter: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Certificate of `domain:port`.
pub async fn probe(domain: &str, port: u16) -> Result<CertInfo> {
    super::check_domain(domain)?;
    kor_core::net::check_host(domain).map_err(|e| e.message)?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ReadOnly(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(domain.to_owned()).map_err(|e| e.to_string())?;
    let fut = async {
        let tcp = TcpStream::connect((domain, port))
            .await
            .map_err(|e| format!("{domain}:{port}: {e}"))?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| format!("{domain}: TLS: {e}"))?;
        let (_, conn) = tls.get_ref();
        let der = conn
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| format!("{domain}: no certificate"))?;
        parse(domain, der.as_ref())
    };
    tokio::time::timeout(Duration::from_secs(15), fut)
        .await
        .map_err(|_| format!("{domain}: TLS timed out"))?
}

/// (tag, content start, content end) of the TLV at `pos`.
fn tlv(b: &[u8], pos: usize) -> Option<(u8, usize, usize)> {
    let tag = *b.get(pos)?;
    let first = *b.get(pos + 1)?;
    let (len, start) = if first < 0x80 {
        (usize::from(first), pos + 2)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | usize::from(*b.get(pos + 2 + i)?);
        }
        (len, pos + 2 + n)
    };
    let end = start.checked_add(len)?;
    (end <= b.len()).then_some((tag, start, end))
}

/// The children of a constructed TLV.
fn children(b: &[u8], start: usize, end: usize) -> Vec<(u8, usize, usize)> {
    let mut out = Vec::new();
    let mut p = start;
    while p < end {
        let Some(t) = tlv(&b[..end], p) else { break };
        out.push(t);
        p = t.2;
    }
    out
}

fn time(b: &[u8], (tag, s, e): (u8, usize, usize)) -> Option<i64> {
    let txt = std::str::from_utf8(&b[s..e]).ok()?;
    let full = match tag {
        0x17 => {
            let yy: i32 = txt.get(0..2)?.parse().ok()?;
            format!("{}{txt}", if yy >= 50 { "19" } else { "20" })
        }
        0x18 => txt.to_owned(),
        _ => return None,
    };
    let dt =
        chrono::NaiveDateTime::parse_from_str(full.trim_end_matches('Z'), "%Y%m%d%H%M%S").ok()?;
    Some(dt.and_utc().timestamp())
}

/// The first CN or O attribute of a Name, O preferred for issuers.
fn name_attr(b: &[u8], (_, s, e): (u8, usize, usize), oid_last: u8) -> Option<String> {
    for set in children(b, s, e) {
        for seq in children(b, set.1, set.2) {
            let kids = children(b, seq.1, seq.2);
            let (oid, val) = (kids.first()?, kids.get(1)?);
            if b[oid.1..oid.2] == [0x55, 0x04, oid_last] {
                return Some(String::from_utf8_lossy(&b[val.1..val.2]).into_owned());
            }
        }
    }
    None
}

/// Expiry, issuer and subject of a DER X.509 certificate.
pub fn parse(domain: &str, der: &[u8]) -> Result<CertInfo> {
    let bad = || format!("{domain}: unreadable certificate");
    let cert = tlv(der, 0).ok_or_else(bad)?;
    let tbs = *children(der, cert.1, cert.2).first().ok_or_else(bad)?;
    let mut f = children(der, tbs.1, tbs.2);
    if f.first().is_some_and(|t| t.0 == 0xa0) {
        f.remove(0); // explicit version
    }
    // serial, signature algorithm, issuer, validity, subject
    let (issuer, validity, subject) = (
        *f.get(2).ok_or_else(bad)?,
        *f.get(3).ok_or_else(bad)?,
        *f.get(4).ok_or_else(bad)?,
    );
    let v = children(der, validity.1, validity.2);
    let not_after = time(der, *v.get(1).ok_or_else(bad)?).ok_or_else(bad)?;
    Ok(CertInfo {
        domain: domain.to_owned(),
        not_after,
        issuer: name_attr(der, issuer, 0x0a)
            .or_else(|| name_attr(der, issuer, 0x03))
            .unwrap_or_default(),
        subject: name_attr(der, subject, 0x03).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DER TLV builder for a synthetic certificate.
    fn t(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        if body.len() < 0x80 {
            v.push(u8::try_from(body.len()).unwrap());
        } else {
            v.push(0x82);
            v.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        }
        v.extend_from_slice(body);
        v
    }

    fn name(oid_last: u8, value: &str) -> Vec<u8> {
        let attr = [t(0x06, &[0x55, 0x04, oid_last]), t(0x0c, value.as_bytes())].concat();
        t(0x30, &t(0x31, &t(0x30, &attr)))
    }

    #[test]
    fn reads_expiry_issuer_and_subject() {
        let validity = t(
            0x30,
            &[t(0x17, b"250101000000Z"), t(0x18, b"20270315120000Z")].concat(),
        );
        let tbs = t(
            0x30,
            &[
                t(0xa0, &t(0x02, &[2])),
                t(0x02, &[1, 2, 3]),
                t(0x30, &t(0x06, &[0x2a])),
                name(0x0a, "Let's Encrypt"),
                validity,
                name(0x03, "example.com"),
            ]
            .concat(),
        );
        let cert = t(0x30, &[tbs, t(0x30, &[]), t(0x03, &[0])].concat());
        let c = parse("example.com", &cert).unwrap();
        assert_eq!(c.issuer, "Let's Encrypt");
        assert_eq!(c.subject, "example.com");
        assert_eq!(c.not_after, 1_805_112_000);
        assert_eq!(c.days_left(1_805_112_000 - 86_400 * 10), 10);
        assert!(parse("x.com", &[0x30, 0x05, 1]).is_err());
    }
}
