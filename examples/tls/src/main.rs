//! HTTPS with rustls, HTTP/2 negotiated via ALPN.
//!
//! Run: `cargo run -p example-tls`, then
//!
//! ```sh
//! curl -k https://127.0.0.1:3443/            # HTTP/1.1
//! curl -k --http2 https://127.0.0.1:3443/    # h2 (ALPN-negotiated)
//! ```
//!
//! The certificates here are the repo's *test* CA fixtures (embedded at
//! compile time so the example runs from anywhere). For production, load
//! your own PEM files at runtime instead.

use std::sync::Arc;

const CERT_PEM: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/../../tests/certs/server-cert.pem"
));
const KEY_PEM: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/../../tests/certs/server-key.pem"
));

#[tokio::main]
async fn main() -> desirable::Result<()> {
  let config = desirable::tls::server_config_from_pem(CERT_PEM, KEY_PEM)?;

  let mut app = desirable::Router::new();
  app.get("/", |_| async {
    "Hello over TLS (h1 or h2 — ALPN decides)"
  });

  desirable::Server::try_bind("127.0.0.1:3443")?
    .tls_config(Arc::new(config))
    .run(app)
    .await
}
