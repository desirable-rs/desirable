use crate::{Middleware, Next, Request, Response, Result};
use hyper::header;

/// CORS (Cross-Origin Resource Sharing) middleware.
///
/// Adds appropriate `Access-Control-*` headers to responses and handles
/// OPTIONS preflight requests automatically.
///
/// Uses a builder pattern for configuration.
///
/// # Default Configuration
///
/// - `allow_origin`: `"*"`
/// - `allow_methods`: `"GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD"`
/// - `allow_headers`: `"Content-Type, Authorization"`
/// - `allow_credentials`: `false`
/// - `max_age`: `86400` (24 hours)
///
/// # Example
///
/// ```rust,ignore
/// use desirable::Cors;
///
/// let mut router = Router::new();
/// router.with(Cors::new());
///
/// // Or with custom configuration:
/// router.with(
///     Cors::new()
///         .allow_origin("https://example.com")
///         .allow_credentials(true)
///         .max_age(3600)
/// );
/// ```
#[derive(Clone, Debug)]
pub struct Cors {
  allow_origin: String,
  allow_methods: String,
  allow_headers: String,
  allow_credentials: bool,
  max_age: u32,
  expose_headers: Option<String>,
}

impl Cors {
  /// Creates a new CORS middleware with permissive defaults.
  #[must_use]
  pub fn new() -> Self {
    Self {
      allow_origin: "*".to_string(),
      allow_methods: "GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD".to_string(),
      allow_headers: "Content-Type, Authorization".to_string(),
      allow_credentials: false,
      max_age: 86400,
      expose_headers: None,
    }
  }

  /// Sets the `Access-Control-Allow-Origin` header value.
  #[must_use]
  pub fn allow_origin(mut self, origin: &str) -> Self {
    self.allow_origin = origin.to_string();
    self
  }

  /// Sets the `Access-Control-Allow-Methods` header value.
  #[must_use]
  pub fn allow_methods(mut self, methods: &str) -> Self {
    self.allow_methods = methods.to_string();
    self
  }

  /// Sets the `Access-Control-Allow-Headers` header value.
  #[must_use]
  pub fn allow_headers(mut self, headers: &str) -> Self {
    self.allow_headers = headers.to_string();
    self
  }

  /// Sets whether `Access-Control-Allow-Credentials` is `true`.
  #[must_use]
  pub fn allow_credentials(mut self, yes: bool) -> Self {
    self.allow_credentials = yes;
    self
  }

  /// Sets the `Access-Control-Max-Age` header value in seconds.
  #[must_use]
  pub fn max_age(mut self, seconds: u32) -> Self {
    self.max_age = seconds;
    self
  }

  /// Sets the `Access-Control-Expose-Headers` header value.
  #[must_use]
  pub fn expose_headers(mut self, headers: &str) -> Self {
    self.expose_headers = Some(headers.to_string());
    self
  }

  /// Applies CORS headers to a response's header map.
  fn apply_headers(&self, headers: &mut hyper::HeaderMap) {
    headers.insert(
      header::ACCESS_CONTROL_ALLOW_ORIGIN,
      self.allow_origin.parse().unwrap(),
    );
    headers.insert(
      header::ACCESS_CONTROL_ALLOW_METHODS,
      self.allow_methods.parse().unwrap(),
    );
    headers.insert(
      header::ACCESS_CONTROL_ALLOW_HEADERS,
      self.allow_headers.parse().unwrap(),
    );
    headers.insert(
      header::ACCESS_CONTROL_MAX_AGE,
      self.max_age.to_string().parse().unwrap(),
    );
    if self.allow_credentials {
      headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        "true".parse().unwrap(),
      );
    }
    if let Some(ref expose) = self.expose_headers {
      headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        expose.parse().unwrap(),
      );
    }
  }
}

impl Default for Cors {
  fn default() -> Self {
    Self::new()
  }
}

#[async_trait::async_trait]
impl Middleware for Cors {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let is_preflight = req.method() == hyper::Method::OPTIONS;

    let mut response = if is_preflight {
      // Preflight: return 204 No Content, don't call next
      Response::with_status(204, String::new())
    } else {
      // Normal request: run the handler
      next.run(req).await
    };

    if let Ok(ref mut res) = response {
      self.apply_headers(res.inner.headers_mut());
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_cors_defaults() {
    let cors = Cors::new();
    assert_eq!(cors.allow_origin, "*");
    assert_eq!(
      cors.allow_methods,
      "GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD"
    );
    assert!(!cors.allow_credentials);
    assert_eq!(cors.max_age, 86400);
  }

  #[test]
  fn test_cors_builder() {
    let cors = Cors::new()
      .allow_origin("https://example.com")
      .allow_methods("GET, POST")
      .allow_headers("X-Custom")
      .allow_credentials(true)
      .max_age(3600)
      .expose_headers("X-Response-Id");

    assert_eq!(cors.allow_origin, "https://example.com");
    assert_eq!(cors.allow_methods, "GET, POST");
    assert_eq!(cors.allow_headers, "X-Custom");
    assert!(cors.allow_credentials);
    assert_eq!(cors.max_age, 3600);
    assert_eq!(cors.expose_headers, Some("X-Response-Id".to_string()));
  }

  #[test]
  fn test_cors_apply_headers() {
    let cors = Cors::new();
    let mut headers = hyper::HeaderMap::new();
    cors.apply_headers(&mut headers);
    assert_eq!(
      headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
      "*"
    );
    assert!(headers.contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
    assert!(headers.contains_key(header::ACCESS_CONTROL_MAX_AGE));
    assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
  }

  #[test]
  fn test_cors_apply_headers_with_credentials() {
    let cors = Cors::new().allow_credentials(true);
    let mut headers = hyper::HeaderMap::new();
    cors.apply_headers(&mut headers);
    assert_eq!(
      headers
        .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
        .unwrap(),
      "true"
    );
  }
}
