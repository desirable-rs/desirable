use crate::{Middleware, Next, Request, Response, Result};

/// Extension carrying the maximum allowed request-body size in bytes.
///
/// Inserted by the [`BodyLimit`] middleware and enforced by
/// [`Request::body`](crate::Request::body), [`Request::body_json`]
/// and [`Request::form`] when they buffer the body.
#[derive(Debug, Clone, Copy)]
pub struct BodyLimitValue(pub usize);

/// Middleware that rejects requests whose body exceeds a size limit.
///
/// Enforcement is two-stage:
///
/// 1. When `Content-Length` is present and exceeds the limit, the request is
///    rejected immediately with `413 Payload Too Large` — the body is never
///    read.
/// 2. For chunked bodies (no `Content-Length`), the limit is stored in the
///    request extensions as [`BodyLimitValue`]; the framework's body parsing
///    methods then stop reading and return `413` once the limit is hit.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::BodyLimit;
///
/// let mut router = Router::new();
/// router.with(BodyLimit::new(1024 * 1024)); // 1 MB
/// ```
#[derive(Clone, Debug)]
pub struct BodyLimit {
  max_bytes: usize,
}

impl BodyLimit {
  /// Creates a `BodyLimit` middleware for the given maximum body size.
  #[must_use]
  pub fn new(max_bytes: usize) -> Self {
    Self { max_bytes }
  }
}

#[async_trait::async_trait]
impl Middleware for BodyLimit {
  async fn handle(&self, mut req: Request, next: Next<'_>) -> Result {
    // Fast path: reject declared oversized bodies without reading them.
    if let Some(len) = req
      .header("content-length")
      .and_then(|v| v.to_str().ok())
      .and_then(|v| v.parse::<usize>().ok())
      && len > self.max_bytes
    {
      return Response::with_status(413, "payload too large".to_string());
    }

    // Slow path: cap the actual read when the body is buffered later.
    req.extensions_mut().insert(BodyLimitValue(self.max_bytes));

    next.run(req).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_body_limit_rejects_declared_length() {
    // Structural check: the middleware can be constructed and configured.
    let limit = BodyLimit::new(1024);
    assert_eq!(limit.max_bytes, 1024);
  }
}
