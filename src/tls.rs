//! TLS support (feature `tls`), backed by rustls.
//!
//! [`server_config_from_pem`] builds a [`ServerConfig`] from PEM bytes with
//! ALPN pre-configured to negotiate HTTP/2 and HTTP/1.1. Enable TLS on a
//! server with [`Server::tls_config`](crate::Server::tls_config).
//!
//! # Example
//!
//! ```rust,ignore
//! use std::sync::Arc;
//!
//! let config = desirable::tls::server_config_from_pem(&cert_pem, &key_pem)?;
//! desirable::Server::try_bind("0.0.0.0:443")?
//!     .tls_config(Arc::new(config))
//!     .run(router)
//!     .await?;
//! ```

use std::io::Cursor;
use std::sync::Arc;

use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub use tokio_rustls::TlsAcceptor;
pub use tokio_rustls::rustls::ServerConfig;

/// Builds a [`ServerConfig`] from PEM-encoded certificate(s) and private
/// key, with ALPN pre-configured to negotiate HTTP/2 or HTTP/1.1.
///
/// No client certificate is required. Uses the ring crypto provider.
pub fn server_config_from_pem(certs_pem: &[u8], key_pem: &[u8]) -> std::io::Result<ServerConfig> {
  let mut certs_reader = Cursor::new(certs_pem);
  let certs: Vec<CertificateDer<'static>> =
    rustls_pemfile::certs(&mut certs_reader).collect::<std::result::Result<Vec<_>, _>>()?;

  let mut key_reader = Cursor::new(key_pem);
  let key: PrivateKeyDer<'static> =
    rustls_pemfile::private_key(&mut key_reader)?.ok_or_else(|| {
      std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "no private key found in PEM",
      )
    })?;

  let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
  let mut config = ServerConfig::builder_with_provider(provider)
    .with_safe_default_protocol_versions()
    .map_err(cert_io_error)?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(cert_io_error)?;

  config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
  Ok(config)
}

/// Builds a [`TlsAcceptor`] from a [`ServerConfig`].
pub fn tls_acceptor(config: ServerConfig) -> TlsAcceptor {
  TlsAcceptor::from(Arc::new(config))
}

fn cert_io_error<E: std::fmt::Display>(err: E) -> std::io::Error {
  std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
}
