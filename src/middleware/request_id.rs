use crate::{Middleware, Next, Request, Result};
use rand::Rng as _;
use std::sync::OnceLock;

/// Header name used by the [`RequestId`] middleware.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// The header name, parsed once instead of per request.
fn request_header_name() -> &'static hyper::header::HeaderName {
  static NAME: OnceLock<hyper::header::HeaderName> = OnceLock::new();
  NAME.get_or_init(|| hyper::header::HeaderName::from_static(REQUEST_ID_HEADER))
}

/// The request ID shared via `request.extensions()`.
///
/// Downstream handlers and middleware can read it with:
///
/// ```rust,ignore
/// let id = req.extensions().get::<RequestIdValue>().unwrap().0.clone();
/// ```
#[derive(Debug, Clone)]
pub struct RequestIdValue(pub String);

/// Middleware that attaches a request ID to every request.
///
/// The ID is taken from the incoming `X-Request-Id` header when present,
/// otherwise a random 32-character hex ID is generated. The value is:
///
/// 1. Stored in the request extensions as [`RequestIdValue`]
/// 2. Set as the `X-Request-Id` header on the response
///
/// # Example
///
/// ```rust,ignore
/// use desirable::RequestId;
///
/// let mut router = Router::new();
/// router.with(RequestId);
/// ```
pub struct RequestId;

impl RequestId {
  /// Generates a random 32-character lowercase hex ID.
  fn generate() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(32);
    for byte in bytes {
      out.push(HEX[(byte >> 4) as usize] as char);
      out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
  }
}

#[async_trait::async_trait]
impl Middleware for RequestId {
  async fn handle(&self, mut req: Request, next: Next<'_>) -> Result {
    let id = req
      .header(REQUEST_ID_HEADER)
      .and_then(|v| v.to_str().ok())
      .map(str::to_string)
      .unwrap_or_else(Self::generate);

    req.extensions_mut().insert(RequestIdValue(id.clone()));

    // When the request runs inside the framework's `request` tracing span,
    // record the final id so every log line inside handlers carries it.
    // (Recording an undeclared field on another span is a silent no-op.)
    tracing::Span::current().record("http.request_id", tracing::field::display(&id));

    let mut response = next.run(req).await;

    if let (Ok(res), Ok(value)) = (&mut response, hyper::header::HeaderValue::from_str(&id)) {
      res.inner.headers_mut().insert(request_header_name(), value);
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_generate_is_hex_and_unique() {
    let a = RequestId::generate();
    let b = RequestId::generate();
    assert_eq!(a.len(), 32);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
  }

  #[test]
  fn test_request_id_middleware_can_be_added() {
    use crate::Router;
    let mut router = Router::new();
    router.get("/", |_| async { "ok" });
    router.with(RequestId);
    assert_eq!(router.middlewares.len(), 1);
  }
}
