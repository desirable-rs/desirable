use crate::{Middleware, Next, Request, Response, Result};
use rand::Rng as _;

/// Default name of the CSRF cookie and (by convention) the header clients
/// echo the token in.
const DEFAULT_COOKIE_NAME: &str = "csrf_token";

/// Cross-Site Request Forgery protection via the double-submit cookie
/// pattern.
///
/// The middleware keeps a random token in a readable cookie (`HttpOnly`
/// stays off — the frontend needs it) and requires the same token on every
/// unsafe request in the `X-CSRF-Token` header. An attacker on another
/// origin can make the browser *send* the cookie, but cannot *read* it to
/// copy into the header, so their forged request is rejected with `403`.
///
/// # Behavior
///
/// - Safe methods (`GET`/`HEAD`/`OPTIONS`) are never blocked; when no token
///   cookie exists yet, one is generated and attached to the response, so
///   the first page load arms the protection.
/// - Unsafe methods (`POST`/`PUT`/`PATCH`/`DELETE`/...) must carry
///   `X-CSRF-Token` matching the cookie (compared in constant time).
/// - No token cookie at all on an unsafe request → `403` (with a fresh
///   token cookie attached for the next attempt).
///
/// # Example
///
/// ```rust,ignore
/// use desirable::Csrf;
///
/// let mut app = Router::new();
/// app.with(Csrf::new()); // or Csrf::new().cookie_name("csrf").secure(true)
/// ```
///
/// Frontend pattern: read the `csrf_token` cookie once and send it back as
/// `X-CSRF-Token` on every mutating request (fetch wrapper / axios
/// interceptor).
#[derive(Clone, Debug)]
pub struct Csrf {
  cookie_name: String,
  header_name: &'static str,
  secure: bool,
}

/// Header clients echo the token in.
pub const CSRF_HEADER: &str = "x-csrf-token";

impl Csrf {
  /// Creates the middleware with the default cookie name (`csrf_token`).
  #[must_use]
  pub fn new() -> Self {
    Self {
      cookie_name: DEFAULT_COOKIE_NAME.to_string(),
      header_name: CSRF_HEADER,
      secure: false,
    }
  }

  /// Renames the token cookie.
  #[must_use]
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Marks the token cookie `Secure` (HTTPS-only). Enable in production.
  #[must_use]
  pub fn secure(mut self, yes: bool) -> Self {
    self.secure = yes;
    self
  }

  /// Generates a fresh 256-bit hex token.
  fn generate_token() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for byte in bytes {
      out.push(HEX[(byte >> 4) as usize] as char);
      out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
  }

  /// Builds the `Set-Cookie` value for `token` (readable, path `/`).
  fn make_cookie(&self, token: &str) -> hyper::header::HeaderValue {
    let mut value = format!("{}={}; Path=/; SameSite=Lax", self.cookie_name, token);
    if self.secure {
      value.push_str("; Secure");
    }
    value
      .parse()
      .expect("csrf cookie value is a valid header value")
  }
}

impl Default for Csrf {
  fn default() -> Self {
    Self::new()
  }
}

/// Constant-time equality for token comparison: both sides are hashed and
/// the fixed-length digests compared, so mismatch position leaks no timing
/// signal about the token.
fn constant_time_eq(a: &str, b: &str) -> bool {
  use sha2::{Digest, Sha256};
  let da = Sha256::digest(a.as_bytes());
  let db = Sha256::digest(b.as_bytes());
  da.as_slice() == db.as_slice()
}

#[async_trait::async_trait]
impl Middleware for Csrf {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let cookie_token = req.cookie(&self.cookie_name).map(|c| c.value().to_string());
    let method = req.method().clone();
    let is_safe = method == hyper::Method::GET
      || method == hyper::Method::HEAD
      || method == hyper::Method::OPTIONS;

    // Unsafe requests must present the cookie's token in the header.
    if !is_safe {
      let provided = req.header(self.header_name).and_then(|v| v.to_str().ok());
      let valid = match (&cookie_token, provided) {
        (Some(cookie_token), Some(provided)) => constant_time_eq(cookie_token, provided),
        _ => false,
      };
      if !valid {
        // Attach a fresh token so a corrected next attempt can succeed.
        let fresh = Self::generate_token();
        let mut blocked = Response::static_text(hyper::StatusCode::FORBIDDEN, "csrf check failed");
        blocked.append_header(hyper::header::SET_COOKIE, self.make_cookie(&fresh));
        return Ok(blocked);
      }
    }

    let mut response = next.run(req).await;

    // Arm the protection: no token cookie yet → set one on the response.
    if let Ok(res) = &mut response
      && cookie_token.is_none()
    {
      let fresh = Self::generate_token();
      res.append_header(hyper::header::SET_COOKIE, self.make_cookie(&fresh));
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_defaults_and_builder() {
    let csrf = Csrf::new();
    assert_eq!(csrf.cookie_name, "csrf_token");
    assert!(!csrf.secure);
    let csrf = csrf.cookie_name("xsrf").secure(true);
    assert_eq!(csrf.cookie_name, "xsrf");
    assert!(csrf.secure);
  }

  #[test]
  fn test_constant_time_eq() {
    assert!(constant_time_eq("abc", "abc"));
    assert!(!constant_time_eq("abc", "abd"));
    assert!(!constant_time_eq("abc", "abcd"));
    assert!(!constant_time_eq("", "a"));
  }

  #[test]
  fn test_token_is_hex_and_unique() {
    let a = Csrf::generate_token();
    let b = Csrf::generate_token();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
  }

  #[test]
  fn test_make_cookie_attributes() {
    let csrf = Csrf::new().secure(true);
    let header = csrf.make_cookie("tok123");
    let value = header.to_str().unwrap();
    assert!(value.starts_with("csrf_token=tok123"));
    assert!(value.contains("SameSite=Lax"));
    assert!(value.contains("Secure"));
    assert!(!value.contains("HttpOnly"), "token must be JS-readable");
  }
}
