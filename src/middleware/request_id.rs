use crate::{Middleware, Next, Request, Result};
use rand::RngCore as _;

/// Header name used by the [`RequestId`] middleware.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

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
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
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

    let mut response = next.run(req).await;

    if let Ok(res) = &mut response {
      if let Ok(value) = hyper::header::HeaderValue::from_str(&id) {
        res.inner.headers_mut().insert(
          hyper::header::HeaderName::from_static(REQUEST_ID_HEADER),
          value,
        );
      }
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
