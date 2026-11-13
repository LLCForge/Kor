//! The team's own certificate authority (mTLS between members and the server, §11.1).
//!
//! `kor server init` creates a CA (`ca.pem`, `ca.key`) and a server certificate for the domain
//! (plus `localhost`). `kor server add-member` issues a client certificate whose subject is the
//! member's e-mail; the server maps the certificate's SHA-256 fingerprint to the member, so a
//! removed member's certificate stops working at once even though the TLS layer still accepts its
//! signature. Keys are P-256 ECDSA. Private keys are written owner-only.

use std::path::Path;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest as _, Sha256};

const DAY: time::Duration = time::Duration::days(1);

/// A certificate and its private key, both PEM.
#[derive(Clone)]
pub struct Issued {
    pub cert_pem: String,
    pub key_pem: String,
    /// SHA-256 of the certificate's DER, hex.
    pub fingerprint: String,
    pub expires_ms: i64,
}

impl std::fmt::Debug for Issued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Issued")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// SHA-256 of a certificate's DER, hex.
pub fn fingerprint(der: &[u8]) -> String {
    crate::hex(&Sha256::digest(der))
}

fn err(e: impl std::fmt::Display) -> String {
    format!("certificate: {e}")
}

/// The CA's parameters; rebuilt identically whenever a certificate is signed (rcgen needs the
/// issuer's name and key usages, not its certificate).
fn ca_params(team: &str) -> Result<CertificateParams, String> {
    let mut p = CertificateParams::new(Vec::new()).map_err(err)?;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.distinguished_name
        .push(DnType::CommonName, format!("Kor team CA ({team})"));
    p.distinguished_name.push(DnType::OrganizationName, team);
    p.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    Ok(p)
}

fn window(days: i64) -> (time::OffsetDateTime, time::OffsetDateTime) {
    let now = time::OffsetDateTime::now_utc();
    (now - DAY, now + time::Duration::days(days))
}

fn ms(t: time::OffsetDateTime) -> i64 {
    i64::try_from(t.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX)
}

/// A new CA, valid ten years.
///
/// # Errors
/// Key generation or signing failed.
pub fn create_ca(team: &str) -> Result<Issued, String> {
    let mut p = ca_params(team)?;
    let (from, to) = window(3650);
    p.not_before = from;
    p.not_after = to;
    let key = KeyPair::generate().map_err(err)?;
    let cert = p.self_signed(&key).map_err(err)?;
    Ok(Issued {
        fingerprint: fingerprint(cert.der()),
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
        expires_ms: ms(to),
    })
}

fn issue(
    team: &str,
    ca_key_pem: &str,
    sans: Vec<String>,
    cn: &str,
    usage: ExtendedKeyUsagePurpose,
    days: i64,
) -> Result<Issued, String> {
    let ca_key = KeyPair::from_pem(ca_key_pem).map_err(err)?;
    let issuer = Issuer::new(ca_params(team)?, ca_key);
    let mut p = CertificateParams::new(sans).map_err(err)?;
    p.distinguished_name.push(DnType::CommonName, cn);
    p.distinguished_name.push(DnType::OrganizationName, team);
    p.use_authority_key_identifier_extension = true;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![usage];
    let (from, to) = window(days);
    p.not_before = from;
    p.not_after = to;
    let key = KeyPair::generate().map_err(err)?;
    let cert = p.signed_by(&key, &issuer).map_err(err)?;
    Ok(Issued {
        fingerprint: fingerprint(cert.der()),
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
        expires_ms: ms(to),
    })
}

/// The server's certificate for `domain`, `localhost` and `127.0.0.1`, valid one year.
///
/// # Errors
/// A bad domain, or signing failed.
pub fn issue_server(team: &str, ca_key_pem: &str, domain: &str) -> Result<Issued, String> {
    let mut sans = vec![domain.to_owned()];
    for s in ["localhost", "127.0.0.1"] {
        if s != domain {
            sans.push(s.to_owned());
        }
    }
    issue(
        team,
        ca_key_pem,
        sans,
        domain,
        ExtendedKeyUsagePurpose::ServerAuth,
        365,
    )
}

/// A member's client certificate (subject = e-mail), valid `days`.
///
/// # Errors
/// Signing failed.
pub fn issue_client(
    team: &str,
    ca_key_pem: &str,
    email: &str,
    days: i64,
) -> Result<Issued, String> {
    issue(
        team,
        ca_key_pem,
        Vec::new(),
        email,
        ExtendedKeyUsagePurpose::ClientAuth,
        days,
    )
}

/// Every certificate in a PEM text.
///
/// # Errors
/// No certificate, or a damaged one.
pub fn certs_from_pem(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let v: Vec<_> = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .map_err(err)?;
    if v.is_empty() {
        return Err("no certificate in the PEM".into());
    }
    Ok(v)
}

/// The private key in a PEM text.
///
/// # Errors
/// No key, or a damaged one.
pub fn key_from_pem(pem: &[u8]) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_slice(pem).map_err(err)
}

/// Write a file only its owner can read (Unix mode 0600; on Windows the profile's ACL).
///
/// # Errors
/// An I/O error.
pub fn write_private(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    let mut f = kor_platform::owner_only(&mut o)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_certificate_chains_to_the_ca() {
        let ca = create_ca("acme").unwrap();
        let member = issue_client("acme", &ca.key_pem, "ali@example.com", 30).unwrap();
        let server = issue_server("acme", &ca.key_pem, "kor.example.com").unwrap();
        assert_ne!(member.fingerprint, server.fingerprint);
        assert!(member.expires_ms > crate::now_ms());

        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(certs_from_pem(ca.cert_pem.as_bytes()).unwrap().remove(0))
            .unwrap();
        let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let v = rustls::server::WebPkiClientVerifier::builder_with_provider(
            std::sync::Arc::new(roots),
            provider,
        )
        .build()
        .unwrap();
        let leaf = certs_from_pem(member.cert_pem.as_bytes())
            .unwrap()
            .remove(0);
        v.verify_client_cert(&leaf, &[], rustls::pki_types::UnixTime::now())
            .unwrap();
        // A certificate from another team's CA does not chain.
        let other = create_ca("other").unwrap();
        let stranger = issue_client("other", &other.key_pem, "eve@example.com", 30).unwrap();
        let leaf = certs_from_pem(stranger.cert_pem.as_bytes())
            .unwrap()
            .remove(0);
        assert!(
            v.verify_client_cert(&leaf, &[], rustls::pki_types::UnixTime::now())
                .is_err()
        );
    }
}
