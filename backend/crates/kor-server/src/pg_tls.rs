//! Postgres over TLS through the workspace's rustls (no OpenSSL, no extra TLS crate). The server's
//! certificate is checked against the OS trust store, or against `KOR_SERVER_DB_CA` (a PEM file)
//! for a private CA. `sslmode=disable` is accepted only for a database on this machine.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect};

/// Refuse a cleartext connection to another machine.
///
/// # Errors
/// An unparsable URL, or `sslmode=disable` with a non-local host.
pub fn check_url(url: &str) -> Result<(), String> {
    let cfg: tokio_postgres::Config = url.parse().map_err(|_| "invalid postgres URL".to_owned())?;
    let local = cfg.get_hosts().iter().all(|h| match h {
        Host::Tcp(n) => n == "localhost" || n == "::1" || n.starts_with("127."),
        #[cfg(unix)]
        Host::Unix(_) => true,
    });
    if cfg.get_ssl_mode() == SslMode::Disable && !local {
        return Err("sslmode=disable is only allowed for a database on this machine".into());
    }
    Ok(())
}

fn client_config() -> Result<rustls::ClientConfig, String> {
    let Some(file) = std::env::var_os("KOR_SERVER_DB_CA") else {
        use rustls_platform_verifier::ConfigVerifierExt as _;
        return rustls::ClientConfig::with_platform_verifier().map_err(|e| format!("TLS: {e}"));
    };
    let pem = std::fs::read(&file).map_err(|e| format!("KOR_SERVER_DB_CA: {e}"))?;
    let mut roots = rustls::RootCertStore::empty();
    for c in crate::pki::certs_from_pem(&pem)? {
        roots.add(c).map_err(|e| e.to_string())?;
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth())
}

/// Connect and drive the connection in the background.
///
/// # Errors
/// The database cannot be reached or refused the login.
pub async fn connect(url: &str) -> Result<tokio_postgres::Client, String> {
    let tls = Rustls(Arc::new(client_config()?));
    let (client, conn) = tokio_postgres::connect(url, tls)
        .await
        .map_err(|e| format!("postgres: {e}"))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(client)
}

#[derive(Clone)]
struct Rustls(Arc<rustls::ClientConfig>);

impl<S> MakeTlsConnect<S> for Rustls
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = Stream<S>;
    type TlsConnect = Connector;
    type Error = rustls::pki_types::InvalidDnsNameError;

    fn make_tls_connect(&mut self, domain: &str) -> Result<Connector, Self::Error> {
        Ok(Connector {
            cfg: self.0.clone(),
            name: ServerName::try_from(domain.to_owned())?,
        })
    }
}

struct Connector {
    cfg: Arc<rustls::ClientConfig>,
    name: ServerName<'static>,
}

impl<S> TlsConnect<S> for Connector
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = Stream<S>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = std::io::Result<Stream<S>>> + Send>>;

    fn connect(self, io: S) -> Self::Future {
        Box::pin(async move {
            let s = tokio_rustls::TlsConnector::from(self.cfg)
                .connect(self.name, io)
                .await?;
            Ok(Stream(s))
        })
    }
}

struct Stream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> tokio_postgres::tls::TlsStream for Stream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Stream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Stream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleartext_only_to_this_machine() {
        assert!(super::check_url("postgres://kor@localhost/kor?sslmode=disable").is_ok());
        assert!(super::check_url("postgres://kor@db.internal/kor?sslmode=require").is_ok());
        assert!(super::check_url("postgres://kor@db.internal/kor?sslmode=disable").is_err());
    }
}
