//! gzip response compression middleware.
//!
//! Enabled by the `compression` cargo feature.
//!
//! [`Compression`] gzips eligible `Body::Full` responses when the client
//! advertises `gzip` support. Streaming bodies pass through untouched —
//! dynamic streams (like server-sent events) should not be compressed, and
//! static files should prefer serving precompressed `.gz`/`.br` siblings
//! (`ServeDir::precompressed`).
//!
//! # Example
//!
//! ```rust,ignore
//! use desirable::Compression;
//!
//! let mut router = Router::new();
//! router.with(Compression::new());
//! ```

use crate::{Middleware, Next, Request, Result};
use bytes::Bytes;

/// Minimum body size worth compressing (bytes).
const MIN_SIZE: usize = 256;

/// Content types that are already compressed or compress poorly.
const SKIP_TYPE_PREFIXES: &[&str] = &[
  "image/",
  "video/",
  "audio/",
  "application/octet-stream",
  "application/zip",
  "application/gzip",
  "application/pdf",
  "application/wasm",
];

/// Case-insensitive ASCII `starts_with` for a trimmed string — no
/// allocation (both checks run per request).
fn starts_with_ignore_case(part: &str, prefix: &str) -> bool {
  let part = part.trim();
  part.len() >= prefix.len()
    && part.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// gzip response-compression middleware (feature `compression`).
///
/// When the client sends `Accept-Encoding: gzip` and the response is an
/// eligible buffered body (see below), it is gzipped in place,
/// `Content-Encoding: gzip` is set and `Content-Length` is recomputed.
///
/// A response is skipped when:
///
/// - the client did not accept gzip,
/// - the status is not 2xx, or a `Content-Encoding` header already exists,
/// - the body is not `Body::Full` (streaming bodies pass through),
/// - the body is smaller than 256 bytes, or
/// - the `Content-Type` is already-compressed media (images, video, …).
#[derive(Clone, Debug, Default)]
pub struct Compression;

impl Compression {
  /// Creates the middleware with gzip at the default level.
  #[must_use]
  pub fn new() -> Self {
    Self
  }

  /// Returns true when the request's `Accept-Encoding` mentions gzip.
  fn accepts_gzip(req: &Request) -> bool {
    req
      .header("accept-encoding")
      .and_then(|v| v.to_str().ok())
      // Simplified negotiation: presence of the token, ignoring q-values.
      .is_some_and(|v| {
        v.split(',')
          .any(|part| starts_with_ignore_case(part, "gzip"))
      })
  }

  /// Returns true when the response's content type is worth compressing.
  fn compressible(headers: &hyper::HeaderMap) -> bool {
    let Some(ct) = headers
      .get(hyper::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
    else {
      return false;
    };
    !SKIP_TYPE_PREFIXES
      .iter()
      .any(|prefix| starts_with_ignore_case(ct, prefix))
  }
}

#[async_trait::async_trait]
impl Middleware for Compression {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let accepts = Self::accepts_gzip(&req);
    let mut response = next.run(req).await;

    if !accepts {
      return response;
    }
    let Ok(res) = &mut response else {
      return response;
    };
    let status_ok = res.status().is_success();
    let already_encoded = res
      .inner
      .headers()
      .contains_key(hyper::header::CONTENT_ENCODING);
    let compressible = Self::compressible(res.inner.headers());

    if !status_ok || already_encoded || !compressible {
      return response;
    }
    if !matches!(res.inner.body(), crate::Body::Full(_)) {
      // Streaming bodies pass through; static files should use
      // ServeDir::precompressed siblings instead.
      return response;
    }

    let original_len = match res.inner.body() {
      crate::Body::Full(data) => data.len(),
      crate::Body::Streaming(_) => unreachable!("checked above"),
    };
    if original_len < MIN_SIZE {
      return response;
    }

    let data = match std::mem::take(res.inner.body_mut()) {
      crate::Body::Full(data) => data,
      crate::Body::Streaming(_) => unreachable!("checked above"),
    };

    let mut encoder =
      async_compression::tokio::write::GzipEncoder::new(Vec::with_capacity(original_len / 2));
    use tokio::io::AsyncWriteExt as _;
    if encoder.write_all(&data).await.is_err() || encoder.shutdown().await.is_err() {
      // Compression failed: serve the original body unchanged.
      *res.inner.body_mut() = crate::Body::Full(data);
      return response;
    }
    let compressed = encoder.into_inner();

    res
      .inner
      .headers_mut()
      .insert(hyper::header::CONTENT_ENCODING, "gzip".parse().unwrap());
    res.inner.headers_mut().insert(
      hyper::header::CONTENT_LENGTH,
      compressed.len().to_string().parse().unwrap(),
    );
    *res.inner.body_mut() = crate::Body::Full(Bytes::from(compressed));

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_compression_middleware_constructible() {
    let _ = Compression::new();
  }

  #[test]
  fn test_starts_with_ignore_case() {
    assert!(starts_with_ignore_case("gzip", "gzip"));
    assert!(starts_with_ignore_case("GZIP", "gzip"));
    assert!(starts_with_ignore_case("gZip;q=0.5", "gzip"));
    assert!(starts_with_ignore_case("  Image/png", "image/"));
    assert!(!starts_with_ignore_case("gzi", "gzip"));
    assert!(!starts_with_ignore_case("text/html", "image/"));
    // Prefix match by design: "gzipx" matches the "gzip" token, matching
    // the previous lowercase-then-starts_with behavior.
    assert!(starts_with_ignore_case("gzipx", "gzip"));
  }
}
