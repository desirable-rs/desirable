//! Session lifecycle management: creation, cookie round-trip, deletion.

use super::config::SessionConfig;
use super::error::SessionError;
use super::{Session, SessionData};
use crate::Result;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use hyper::http;
use sha2::Sha256;
use std::sync::Arc;

type HmacSha256 = Hmac<Sha256>;

/// Manages session creation, reading, and cookie generation.
///
/// `SessionManager` handles the lifecycle of sessions including:
/// - Creating new sessions
/// - Reading sessions from cookie values
/// - Writing sessions to cookie headers
/// - Generating deletion cookies for logout
///
/// # Thread Safety
///
/// `SessionManager` is cheap to clone and share across threads.
/// Internally it uses `Arc<SessionConfig>` for efficient sharing.
///
/// # Example
///
/// ```rust
/// use desirable::{SessionManager, SessionConfig};
///
/// // Create a manager with a specific signing key
/// let key = b"your-32-byte-secret-key-here!!!!";
/// let config = SessionConfig::new(key);
/// let manager = SessionManager::new(config);
///
/// // Create a new session
/// let mut session = manager.create_session();
/// session.insert("user_id", 42).unwrap();
///
/// // Generate a Set-Cookie header
/// let _cookie_header = manager.make_cookie_header(&session);
/// // Use in response: response.headers_mut().insert(SET_COOKIE, cookie_header);
///
/// // Later, read the session from a request cookie
/// let cookie_value = manager.write_session(&session);
/// if let Some(loaded) = manager.read_session(&cookie_value).unwrap() {
///     let user_id: Option<i32> = loaded.get("user_id").unwrap();
///     println!("User ID: {:?}", user_id);
/// }
/// ```
///
/// # Cookie Format
///
/// Sessions are stored in cookies using the following format:
///
/// ```text
/// <base64-encoded-session-data>|<hmac-sha256-signature>
/// ```
///
/// This format provides:
/// 1. Tamper detection via HMAC signature
/// 2. Safe transmission via Base64URL encoding
#[derive(Clone, Debug)]
pub struct SessionManager {
  /// The session configuration
  config: Arc<SessionConfig>,
}

impl SessionManager {
  /// Creates a new `SessionManager` with the given configuration.
  ///
  /// The configuration is stored in an `Arc` for efficient cloning.
  ///
  /// # Arguments
  ///
  /// * `config` - The session configuration to use
  ///
  /// # Returns
  ///
  /// A new `SessionManager` instance
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let config = SessionConfig::new(key);
  /// let manager = SessionManager::new(config);
  /// ```
  pub fn new(config: SessionConfig) -> Self {
    Self {
      config: Arc::new(config),
    }
  }

  /// Creates a new `SessionManager` with a randomly generated signing key.
  ///
  /// This is useful for development or when persistence of the signing key
  /// is not required (e.g., single-instance applications).
  ///
  /// # Warning
  ///
  /// Sessions created with this manager cannot be validated after restart
  /// because the signing key will be different.
  ///
  /// # Returns
  ///
  /// A new `SessionManager` with a random 32-byte signing key
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionManager;
  ///
  /// let manager = SessionManager::with_random_key();
  /// ```
  pub fn with_random_key() -> Self {
    Self::new(SessionConfig::default())
  }

  /// Returns a reference to the session configuration.
  ///
  /// # Returns
  ///
  /// A reference to the `SessionConfig`
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let config = SessionConfig::new(key);
  /// let manager = SessionManager::new(config);
  ///
  /// let cookie_name = &manager.config().cookie_name;
  /// ```
  pub fn config(&self) -> &SessionConfig {
    &self.config
  }

  /// Creates a new, empty session.
  ///
  /// The session will have a unique ID and empty data.
  ///
  /// # Returns
  ///
  /// A new `Session` ready for use
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionManager;
  ///
  /// let manager = SessionManager::with_random_key();
  /// let session = manager.create_session();
  ///
  /// assert!(!session.id().is_empty());
  /// assert!(session.is_empty());
  /// ```
  pub fn create_session(&self) -> Session {
    Session::new(SessionData::new())
  }

  /// Reads and validates a session from a cookie value.
  ///
  /// Decodes the cookie value, verifies the HMAC signature, and deserializes
  /// the session data.
  ///
  /// # Arguments
  ///
  /// * `cookie_value` - The raw cookie value (should be Base64URL encoded)
  ///
  /// # Returns
  ///
  /// - `Ok(Some(Session))` if the cookie is valid
  /// - `Ok(None)` if the cookie is empty
  /// - `Err(SessionError)` if the cookie is invalid or tampered
  ///
  /// # Errors
  ///
  /// - `SessionError::InvalidCookie` if the cookie format is invalid
  /// - `SessionError::SignatureMismatch` if the signature verification fails
  /// - `SessionError::Serialization` if JSON deserialization fails
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let manager = SessionManager::new(SessionConfig::new(key));
  ///
  /// // Create and serialize a session
  /// let mut session = manager.create_session();
  /// session.insert("user_id", 42).unwrap();
  /// let cookie = manager.write_session(&session);
  ///
  /// // Later, read the session back
  /// let loaded = manager.read_session(&cookie).unwrap();
  /// assert!(loaded.is_some());
  /// let user_id: Option<i32> = loaded.unwrap().get("user_id").unwrap();
  /// assert_eq!(user_id, Some(42));
  /// ```
  pub fn read_session(&self, cookie_value: &str) -> Result<Option<Session>> {
    if cookie_value.is_empty() {
      return Ok(None);
    }
    let decoded = base64::engine::general_purpose::URL_SAFE
      .decode(cookie_value)
      .map_err(|_| SessionError::InvalidCookie)?;
    let pos = decoded.iter().position(|&c| c == b'|');
    if let Some(idx) = pos {
      let (data_bytes, signature_bytes) = decoded.split_at(idx);
      if signature_bytes.is_empty() || signature_bytes[0] != b'|' {
        return Err(SessionError::InvalidCookie.into());
      }
      let sig = &signature_bytes[1..];
      let mut mac = HmacSha256::new_from_slice(&self.config.signing_key)
        .map_err(|_| SessionError::InvalidCookie)?;
      mac.update(data_bytes);
      mac
        .verify_slice(sig)
        .map_err(|_| SessionError::SignatureMismatch)?;
      let session_data: SessionData =
        serde_json::from_slice(data_bytes).map_err(|_| SessionError::InvalidCookie)?;
      Ok(Some(Session::new(session_data)))
    } else {
      Err(SessionError::InvalidCookie.into())
    }
  }

  /// Serializes a session to a cookie-safe string.
  ///
  /// The session data is serialized to JSON, signed with HMAC-SHA256,
  /// and Base64URL encoded for safe transmission in cookies.
  ///
  /// # Arguments
  ///
  /// * `session` - The session to serialize
  ///
  /// # Returns
  ///
  /// A Base64URL-encoded string of the format `data|signature`
  ///
  /// # Note
  ///
  /// This method does not set any cookie attributes (path, domain, etc.).
  /// Use [`SessionManager::make_cookie_header`] for a complete cookie header.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let manager = SessionManager::new(SessionConfig::new(key));
  ///
  /// let session = manager.create_session();
  /// let cookie_value = manager.write_session(&session);
  ///
  /// // The value is Base64URL encoded
  /// println!("Cookie value: {}", cookie_value);
  /// ```
  pub fn write_session(&self, session: &Session) -> String {
    let data_bytes = serde_json::to_vec(&session.inner).unwrap_or_default();
    let mut mac = HmacSha256::new_from_slice(&self.config.signing_key).unwrap();
    mac.update(&data_bytes);
    let signature = mac.finalize().into_bytes();
    let mut combined = data_bytes;
    combined.push(b'|');
    combined.extend_from_slice(&signature);
    base64::engine::general_purpose::URL_SAFE.encode(&combined)
  }

  /// Creates a complete Set-Cookie header value for a session.
  ///
  /// This is a convenience method that combines [`SessionManager::write_session`]
  /// with all the configured cookie attributes.
  ///
  /// # Arguments
  ///
  /// * `session` - The session to create a cookie for
  ///
  /// # Returns
  ///
  /// A `HeaderValue` suitable for the `Set-Cookie` header
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let config = SessionConfig::new(key)
  ///     .domain("example.com")
  ///     .path("/api");
  /// let manager = SessionManager::new(config);
  ///
  /// let session = manager.create_session();
  /// let header = manager.make_cookie_header(&session);
  ///
  /// // Use in response:
  /// // response.headers_mut().insert(hyper::header::SET_COOKIE, header);
  /// ```
  pub fn make_cookie_header(&self, session: &Session) -> http::HeaderValue {
    let cookie_value = self.write_session(session);
    let mut builder = cookie::CookieBuilder::new(self.config.cookie_name.clone(), cookie_value)
      .path(self.config.path.clone())
      .http_only(self.config.http_only)
      .same_site(self.config.same_site);
    if let Some(max_age) = self.config.max_age_secs {
      builder = builder.max_age(time::Duration::seconds(max_age));
    }
    if self.config.secure {
      builder = builder.secure(true);
    }
    if let Some(ref domain) = self.config.domain {
      builder = builder.domain(domain.clone());
    }
    builder.build().to_string().parse().unwrap()
  }

  /// Creates a cookie header that will delete the session.
  ///
  /// Sets the cookie's max-age to 0 and value to empty, instructing the
  /// browser to delete the session cookie immediately.
  ///
  /// Use this method when logging out a user to invalidate their session.
  ///
  /// # Returns
  ///
  /// A `HeaderValue` that will delete the session cookie
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let config = SessionConfig::new(key).domain("example.com");
  /// let manager = SessionManager::new(config);
  ///
  /// let deletion_cookie = manager.make_deletion_cookie();
  ///
  /// // Send this to the client to log them out:
  /// // response.headers_mut().insert(hyper::header::SET_COOKIE, deletion_cookie);
  /// ```
  pub fn make_deletion_cookie(&self) -> http::HeaderValue {
    let mut builder = cookie::CookieBuilder::new(self.config.cookie_name.clone(), "")
      .path(self.config.path.clone())
      .http_only(self.config.http_only)
      .same_site(self.config.same_site)
      .max_age(time::Duration::seconds(0));
    if self.config.secure {
      builder = builder.secure(true);
    }
    if let Some(ref domain) = self.config.domain {
      builder = builder.domain(domain.clone());
    }
    builder.build().to_string().parse().unwrap()
  }

  /// Extracts the session cookie value from a request's headers.
  ///
  /// Parses the Cookie header and extracts the value of the configured
  /// session cookie name.
  ///
  /// # Arguments
  ///
  /// * `headers` - The request headers to extract from
  ///
  /// # Returns
  ///
  /// `Some(cookie_value)` if the session cookie is present,
  /// `None` if the cookie is not found
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// use desirable::{SessionManager, SessionConfig};
  /// use hyper::header::HeaderMap;
  ///
  /// let key = b"your-32-byte-secret-key-here!!!!";
  /// let manager = SessionManager::new(SessionConfig::new(key));
  ///
  /// // Simulate incoming request headers
  /// let mut headers = HeaderMap::new();
  /// headers.insert(hyper::header::COOKIE, "desirable_session=abc123".parse().unwrap());
  ///
  /// if let Some(value) = manager.get_cookie_value(&headers) {
  ///     println!("Session cookie: {}", value);
  /// }
  /// ```
  pub fn get_cookie_value(&self, headers: &hyper::header::HeaderMap) -> Option<String> {
    self.get_cookie_value_str(headers).map(str::to_string)
  }

  /// Borrowing variant of [`SessionManager::get_cookie_value`] — no
  /// allocation; the reference is valid as long as `headers` is.
  pub(crate) fn get_cookie_value_str<'a>(
    &self,
    headers: &'a hyper::header::HeaderMap,
  ) -> Option<&'a str> {
    let name = &self.config.cookie_name;
    headers
      .get(http::header::COOKIE)
      .and_then(|v| v.to_str().ok())
      .and_then(|cookie_str| {
        cookie_str
          .split(';')
          .map(str::trim)
          // Compare the `name=` prefix without allocating, then take
          // everything after the FIRST `=` — cookie values may themselves
          // contain `=` (base64 padding), which must not be truncated.
          .find(|s| s.starts_with(name.as_str()) && s.as_bytes().get(name.len()) == Some(&b'='))
          .and_then(|s| s.split_once('=').map(|(_, value)| value))
      })
  }
}
