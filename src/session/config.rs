//! Configuration builder for session cookies.

use super::data::{DEFAULT_COOKIE_NAME, DEFAULT_MAX_AGE_SECS};
use rand::Rng as _;

/// Configuration for session cookie behavior.
///
/// `SessionConfig` controls all aspects of how session cookies are created,
/// transmitted, and managed. It uses a builder pattern for flexible configuration.
///
/// # Default Configuration
///
/// By default, sessions use:
/// - Cookie name: `"desirable_session"`
/// - Path: `"/"`
/// - Secure: `true` (HTTPS only)
/// - HttpOnly: `true` (no JavaScript access)
/// - SameSite: `Lax`
/// - Max age: 30 days
///
/// # Example
///
/// ```rust
/// use desirable::SessionConfig;
///
/// // Create a config with a custom signing key
/// let key = b"your-32-byte-secret-key-here!!!!";
/// let config = SessionConfig::new(key)
///     .cookie_name("my_app_session")
///     .path("/api")
///     .domain("example.com")
///     .secure(true)
///     .http_only(true)
///     .same_site(cookie::SameSite::Strict)
///     .max_age_secs(86400); // 1 day
/// ```
///
/// # Security Notes
///
/// - The signing key must be at least 32 bytes long
/// - Use a cryptographically random key in production
/// - Keep the signing key secret and consistent across restarts
/// - Consider rotating keys periodically
#[derive(Clone, Debug)]
pub struct SessionConfig {
  /// The name of the session cookie.
  ///
  /// Default: `"desirable_session"`
  pub cookie_name: String,

  /// The URL path scope for the cookie.
  ///
  /// Determines which paths can access the cookie.
  /// Default: `"/"`
  pub path: String,

  /// The domain scope for the cookie.
  ///
  /// If `None`, the cookie is only sent to the exact origin.
  /// Default: `None`
  pub domain: Option<String>,

  /// Whether the cookie requires HTTPS.
  ///
  /// When `true`, the cookie will only be sent over secure connections.
  /// Default: `true`
  pub secure: bool,

  /// Whether the cookie is inaccessible to JavaScript.
  ///
  /// When `true`, the cookie cannot be accessed via `document.cookie`,
  /// providing protection against XSS attacks.
  /// Default: `true`
  pub http_only: bool,

  /// The SameSite attribute for CSRF protection.
  ///
  /// - `Strict`: Cookie is only sent in first-party context
  /// - `Lax`: Cookie is sent with top-level navigations and safe HTTP methods
  /// - `None`: Cookie is sent in all contexts (requires `secure = true`)
  ///
  /// Default: `SameSite::Lax`
  pub same_site: cookie::SameSite,

  /// The maximum age of the session cookie in seconds.
  ///
  /// If `None`, the cookie is a session cookie (deleted when browser closes).
  /// Default: `Some(30 days)` (2592000 seconds)
  pub max_age_secs: Option<i64>,

  /// The secret key used to sign session cookies.
  ///
  /// Must be at least 32 bytes. Used with HMAC-SHA256 to prevent tampering.
  pub signing_key: Vec<u8>,
}

impl SessionConfig {
  /// Creates a new `SessionConfig` with the specified signing key.
  ///
  /// The signing key is used to create HMAC-SHA256 signatures for session cookies,
  /// preventing tampering with session data. It must be at least 32 bytes long.
  ///
  /// # Arguments
  ///
  /// * `signing_key` - A byte slice containing the signing key (must be >= 32 bytes)
  ///
  /// # Panics
  ///
  /// Panics if the signing key is less than 32 bytes.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  ///
  /// // Use a cryptographically random 32-byte key
  /// let key = b"this-is-a-32-byte-secret-key-!!!!";
  /// let config = SessionConfig::new(key);
  /// ```
  pub fn new(signing_key: &[u8]) -> Self {
    assert!(
      signing_key.len() >= 32,
      "signing key must be at least 32 bytes"
    );
    Self {
      cookie_name: DEFAULT_COOKIE_NAME.to_string(),
      path: "/".to_string(),
      domain: None,
      secure: true,
      http_only: true,
      same_site: cookie::SameSite::Lax,
      max_age_secs: Some(DEFAULT_MAX_AGE_SECS),
      signing_key: signing_key.to_vec(),
    }
  }

  /// Sets the name of the session cookie.
  ///
  /// Default: `"desirable_session"`
  ///
  /// # Arguments
  ///
  /// * `name` - The cookie name to use for sessions
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  ///
  /// let config = SessionConfig::new(&[0; 32])
  ///     .cookie_name("my_session_id");
  /// ```
  #[must_use]
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Sets the path scope for the session cookie.
  ///
  /// Determines which URLs the cookie will be sent with.
  /// Default: `"/"`
  ///
  /// # Arguments
  ///
  /// * `path` - The URL path scope for the cookie
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  ///
  /// // Cookie only sent to /api routes
  /// let config = SessionConfig::new(&[0; 32])
  ///     .path("/api");
  /// ```
  #[must_use]
  pub fn path(mut self, path: &str) -> Self {
    self.path = path.to_string();
    self
  }

  /// Sets the domain scope for the session cookie.
  ///
  /// If set, the cookie will be sent to this domain and all subdomains.
  /// Default: `None` (exact origin only)
  ///
  /// # Arguments
  ///
  /// * `domain` - The domain scope for the cookie (e.g., "example.com")
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  ///
  /// // Cookie sent to example.com and sub.example.com
  /// let config = SessionConfig::new(&[0; 32])
  ///     .domain("example.com");
  /// ```
  #[must_use]
  pub fn domain(mut self, domain: &str) -> Self {
    self.domain = Some(domain.to_string());
    self
  }

  /// Sets whether the cookie requires HTTPS.
  ///
  /// When `true`, the browser will only send the cookie over secure (HTTPS) connections.
  /// Default: `true`
  ///
  /// # Arguments
  ///
  /// * `secure` - Whether to require HTTPS for the cookie
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Security Note
  ///
  /// Setting this to `false` in production is not recommended as it allows
  /// session cookies to be sent over unencrypted connections.
  #[must_use]
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }

  /// Sets whether the cookie is inaccessible to JavaScript.
  ///
  /// When `true`, the cookie cannot be accessed via `document.cookie`,
  /// providing protection against XSS attacks that could steal session data.
  /// Default: `true`
  ///
  /// # Arguments
  ///
  /// * `http_only` - Whether to make the cookie HttpOnly
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Security Note
  ///
  /// This should generally remain `true` to prevent XSS attacks from stealing sessions.
  #[must_use]
  pub fn http_only(mut self, http_only: bool) -> Self {
    self.http_only = http_only;
    self
  }

  /// Sets the SameSite attribute for the session cookie.
  ///
  /// This provides CSRF protection by controlling when the cookie is sent with cross-site requests.
  /// Default: `SameSite::Lax`
  ///
  /// # Arguments
  ///
  /// * `same_site` - The SameSite mode to use
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # SameSite Modes
  ///
  /// - `Strict`: Cookie is only sent in a first-party context
  /// - `Lax`: Cookie is sent with top-level navigations and safe HTTP methods (GET for navigation)
  /// - `None`: Cookie is sent in all contexts (requires `secure = true`)
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  /// use cookie::SameSite;
  ///
  /// let config = SessionConfig::new(&[0; 32])
  ///     .same_site(SameSite::Strict);
  /// ```
  #[must_use]
  pub fn same_site(mut self, same_site: cookie::SameSite) -> Self {
    self.same_site = same_site;
    self
  }

  /// Sets the maximum age of the session cookie in seconds.
  ///
  /// This determines how long the session cookie will persist in the browser.
  /// Default: `Some(30 days)` (2592000 seconds)
  ///
  /// # Arguments
  ///
  /// * `secs` - The maximum age in seconds. `None` creates a session cookie
  ///   that is deleted when the browser closes.
  ///
  /// # Returns
  ///
  /// The updated `SessionConfig` for method chaining
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::SessionConfig;
  ///
  /// // 1 hour session
  /// let config = SessionConfig::new(b"your-32-byte-secret-key-here!!!!")
  ///     .max_age_secs(3600);
  ///
  /// // Note: For session cookies (deleted when browser closes),
  /// // set max_age_secs to None in the config before creating the manager
  /// ```
  #[must_use]
  pub fn max_age_secs(mut self, secs: i64) -> Self {
    self.max_age_secs = Some(secs);
    self
  }
}

impl Default for SessionConfig {
  fn default() -> Self {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    Self::new(&key)
  }
}
