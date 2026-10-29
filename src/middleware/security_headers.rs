use crate::{Middleware, Next, Request, Result};

/// Security-response-headers middleware: one line to set the standard
/// hardening headers on every response.
///
/// All values are parsed into `HeaderValue`s once at construction, so
/// applying them per request is a handful of map inserts with no
/// allocation. Invalid values fail fast at construction.
///
/// Nothing is enabled by default — opt in per header:
///
/// ```rust,ignore
/// use desirable::SecurityHeaders;
///
/// let mut app = Router::new();
/// app.with(
///     SecurityHeaders::new()
///         .nosniff()
///         .frame_options_deny()
///         .referrer_policy("strict-origin-when-cross-origin")
///         .hsts(31_536_000)                                  // HTTPS-only deployments
///         .content_security_policy("default-src 'self'"),
/// );
/// ```
#[derive(Clone, Debug, Default)]
pub struct SecurityHeaders {
  headers: Vec<(hyper::header::HeaderName, hyper::header::HeaderValue)>,
}

fn parse_value(kind: &str, value: &str) -> hyper::header::HeaderValue {
  value
    .parse()
    .unwrap_or_else(|_| panic!("invalid {kind} header value: {value:?}"))
}

impl SecurityHeaders {
  /// Creates an empty set — opt in to each header with the builders.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  fn with(mut self, name: hyper::header::HeaderName, value: hyper::header::HeaderValue) -> Self {
    self.headers.push((name, value));
    self
  }

  /// Sets `X-Content-Type-Options: nosniff` — browsers must not MIME-sniff
  /// responses away from the declared Content-Type.
  #[must_use]
  pub fn nosniff(self) -> Self {
    self.with(
      hyper::header::X_CONTENT_TYPE_OPTIONS,
      hyper::header::HeaderValue::from_static("nosniff"),
    )
  }

  /// Sets `X-Frame-Options: DENY` — the page must not be framed.
  #[must_use]
  pub fn frame_options_deny(self) -> Self {
    self.with(
      hyper::header::HeaderName::from_static("x-frame-options"),
      hyper::header::HeaderValue::from_static("DENY"),
    )
  }

  /// Sets `X-Frame-Options: SAMEORIGIN` — framing only from the same origin.
  #[must_use]
  pub fn frame_options_same_origin(self) -> Self {
    self.with(
      hyper::header::HeaderName::from_static("x-frame-options"),
      hyper::header::HeaderValue::from_static("SAMEORIGIN"),
    )
  }

  /// Sets `Referrer-Policy`.
  #[must_use]
  pub fn referrer_policy(self, value: &str) -> Self {
    self.with(
      hyper::header::REFERRER_POLICY,
      parse_value("Referrer-Policy", value),
    )
  }

  /// Sets `Strict-Transport-Security: max-age=<secs>; includeSubDomains`
  /// for HTTPS-only deployments.
  #[must_use]
  pub fn hsts(self, max_age_secs: u64) -> Self {
    self.with(
      hyper::header::HeaderName::from_static("strict-transport-security"),
      parse_value(
        "Strict-Transport-Security",
        &format!("max-age={max_age_secs}; includeSubDomains"),
      ),
    )
  }

  /// Sets a `Content-Security-Policy`.
  #[must_use]
  pub fn content_security_policy(self, value: &str) -> Self {
    self.with(
      hyper::header::HeaderName::from_static("content-security-policy"),
      parse_value("Content-Security-Policy", value),
    )
  }
}

#[async_trait::async_trait]
impl Middleware for SecurityHeaders {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let mut response = next.run(req).await;
    if let Ok(res) = &mut response {
      let headers = res.inner.headers_mut();
      for (name, value) in &self.headers {
        headers.insert(name.clone(), value.clone());
      }
    }
    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_builder_collects_headers() {
    let sh = SecurityHeaders::new()
      .nosniff()
      .frame_options_deny()
      .referrer_policy("no-referrer")
      .hsts(3600)
      .content_security_policy("default-src 'self'");

    let names: Vec<&str> = sh.headers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
      names,
      vec![
        "x-content-type-options",
        "x-frame-options",
        "referrer-policy",
        "strict-transport-security",
        "content-security-policy"
      ]
    );
    let hsts = sh.headers[3].1.to_str().unwrap();
    assert_eq!(hsts, "max-age=3600; includeSubDomains");
  }

  #[test]
  #[should_panic(expected = "invalid Referrer-Policy header value")]
  fn test_invalid_value_fails_fast() {
    let _ = SecurityHeaders::new().referrer_policy("bad\nvalue");
  }
}
