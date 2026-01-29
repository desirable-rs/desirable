//! Cookie-based session management for the desirable web framework.
//!
//! This module provides a secure, cookie-based session management system for
//! storing user state across HTTP requests. Sessions are signed using HMAC-SHA256
//! to prevent tampering and serialized using Base64URL encoding for safe cookie
//! transmission.
//!
//! # Example
//!
//! ```rust,ignore
//! use desirable::{SessionManager, SessionConfig};
//!
//! // Create a session manager with a signing key (must be at least 32 bytes)
//! let key = b"your-32-byte-secret-key-here!!!!";
//! let config = SessionConfig::new(key);
//! let manager = SessionManager::new(config);
//!
//! // Create a new session
//! let mut session = manager.create_session();
//! session.insert("user_id", 42).unwrap();
//! session.insert("username", "alice").unwrap();
//!
//! // Generate a cookie header for the session
//! let cookie_value = manager.write_session(&session);
//!
//! // Later, read the session from a cookie value
//! if let Some(loaded_session) = manager.read_session(&cookie_value).unwrap() {
//!     let user_id: Option<i32> = loaded_session.get("user_id").unwrap();
//!     println!("User ID: {:?}", user_id);
//! }
//!
//! // To destroy a session, use the deletion cookie
//! let deletion_cookie = manager.make_deletion_cookie();
//! ```
//!
//! # Security
//!
//! - All session data is signed with HMAC-SHA256 to prevent tampering
//!
//! - The signing key must be at least 32 bytes long
//!
//! - By default, cookies are configured with:
//!   - HttpOnly: Prevents JavaScript access
//!   - SameSite=Lax: CSRF protection
//!   - Secure: HTTPS-only transmission
//!
//! # Session Lifecycle
//!
//! 1. **Creation**: Call [`SessionManager::create_session()`] to create a new session
//!
//! 2. **Storage**: Use [`Session::insert()`] to store data in the session
//!
//! 3. **Transmission**: Generate a cookie header with [`SessionManager::make_cookie_header()`]
//!
//! 4. **Retrieving**: Read the cookie value and load the session with [`SessionManager::read_session()`]
//!
//! 5. **Destruction**: Use [`SessionManager::make_deletion_cookie()`] to invalidate a session

use crate::Result;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use hyper::http;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;

type HmacSha256 = Hmac<Sha256>;
const DEFAULT_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;
const SESSION_ID_LENGTH: usize = 32;
const DEFAULT_COOKIE_NAME: &str = "desirable_session";

/// Errors that can occur during session operations.
///
/// This enum represents all possible errors that may arise when creating,
/// reading, or managing sessions. Each variant includes a user-friendly
/// error message suitable for debugging and logging.
///
/// # Example
///
/// ```rust
/// use desirable::SessionError;
///
/// fn handle_session_error(error: SessionError) {
///     match error {
///         SessionError::InvalidCookie => {
///             println!("The session cookie was malformed or tampered with");
///         }
///         SessionError::SignatureMismatch => {
///             println!("Session cookie signature verification failed");
///         }
///         SessionError::Expired => {
///             println!("The session has expired");
///         }
///         SessionError::NotFound => {
///             println!("Session not found");
///         }
///         SessionError::KeyNotFound(key) => {
///             println!("Key '{}' not found in session", key);
///         }
///         SessionError::Serialization(e) => {
///             println!("JSON serialization error: {}", e);
///         }
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
  /// The session cookie is malformed, empty, or has an invalid format.
  ///
  /// This error occurs when:
  /// - The cookie value is empty
  /// - The cookie cannot be decoded from Base64URL
  /// - The cookie format doesn't match `data|signature`
  #[error("invalid session cookie")]
  InvalidCookie,

  /// The session cookie signature does not match the expected signature.
  ///
  /// This indicates potential tampering with the cookie. The cookie data
  /// may have been modified after it was signed, or the signing key
  /// may have changed.
  #[error("session signature mismatch")]
  SignatureMismatch,

  /// The session has expired and is no longer valid.
  ///
  /// Sessions automatically expire after their configured maximum age.
  /// Consider creating a new session for the user.
  #[error("session expired")]
  Expired,

  /// The requested session was not found.
  ///
  /// This may occur when:
  /// - No session cookie is present in the request
  /// - The session ID doesn't exist in the session store
  #[error("session not found")]
  NotFound,

  /// The specified key was not found in the session data.
  ///
  /// # Arguments
  ///
  /// * `key` - The key that was not found
  #[error("key not found in session: {0}")]
  KeyNotFound(String),

  /// An error occurred during session data serialization or deserialization.
  ///
  /// This typically indicates corrupted session data or a mismatch between
  /// the serialization format and the expected structure.
  ///
  /// # Arguments
  ///
  /// * `error` - The underlying serde_json error
  #[error("session serialization error: {0}")]
  Serialization(#[from] serde_json::Error),
}

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
  ///            that is deleted when the browser closes.
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
    rand::thread_rng().fill_bytes(&mut key);
    Self::new(&key)
  }
}

/// The internal data structure for a session.
///
/// `SessionData` contains the raw session information that gets serialized
/// to JSON and signed for storage in a cookie. It includes:
/// - A unique session ID
/// - Creation and access timestamps
/// - Key-value data stored in the session
///
/// This struct is serialized to JSON and signed with HMAC-SHA256 before
/// being stored in a cookie.
///
/// # Note
///
/// This is the internal representation. Use [`Session`] for runtime session
/// operations which provides additional tracking of modifications.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionData {
  /// A unique identifier for the session.
  ///
  /// Generated using cryptographically random bytes and Base64URL encoded.
  /// Used to identify the session in logs and for session management.
  pub id: String,

  /// The timestamp when the session was created.
  ///
  /// Set once during session creation and never modified.
  pub created: DateTime<Utc>,

  /// The timestamp of the last session access.
  ///
  /// Updated whenever the session is accessed or modified.
  /// Used for session expiration and activity tracking.
  pub accessed: DateTime<Utc>,

  /// The session data stored as key-value pairs.
  ///
  /// Values are serialized to JSON strings for storage.
  /// Supports any type that implements [`serde::Serialize`] and [`serde::Deserialize`].
  #[serde(flatten)]
  pub data: HashMap<String, String>,
}

impl SessionData {
  pub fn new() -> Self {
    let now = Utc::now();
    let mut bytes = [0u8; SESSION_ID_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    Self {
      id,
      created: now,
      accessed: now,
      data: HashMap::new(),
    }
  }
}

impl Default for SessionData {
  fn default() -> Self {
    Self::new()
  }
}

/// A runtime session for storing and retrieving user data.
///
/// `Session` provides a high-level API for managing session data. It wraps
/// `SessionData` and tracks modifications to optimize cookie generation.
///
/// # Session Lifecycle
///
/// 1. Create or load a session
/// 2. Read/write session data using [`Session::get`] and [`Session::insert`]
/// 3. Generate a cookie header with [`SessionManager::make_cookie_header`]
/// 4. On subsequent requests, load the session with [`SessionManager::read_session`]
///
/// # Example
///
/// ```rust
/// use desirable::{SessionManager, SessionConfig, Session, Result};
///
/// fn handle_user_login(manager: &SessionManager, user_id: i32, username: String) -> Result<Session> {
///     let mut session = manager.create_session();
///     session.insert("user_id", user_id)?;
///     session.insert("username", username)?;
///     session.insert("role", "user")?;
///     Ok(session)
/// }
///
/// fn check_auth(session: &Session) -> Result<Option<i32>> {
///     session.get("user_id")
/// }
/// ```
///
/// # Modification Tracking
///
/// The `Session` tracks whether it has been modified. This is useful for
/// conditional cookie updates - only send a Set-Cookie header if the session
/// was modified.
///
/// ```rust
/// use desirable::{SessionManager, SessionConfig};
///
/// let key = b"your-32-byte-secret-key-here!!!!";
/// let manager = SessionManager::new(SessionConfig::new(key));
///
/// let mut session = manager.create_session();
/// assert!(!session.is_modified()); // New sessions start unmodified
///
/// session.insert("key", "value").unwrap();
/// assert!(session.is_modified()); // Now modified
/// ```
#[derive(Clone, Debug)]
pub struct Session {
  /// The underlying session data
  inner: SessionData,
  /// Whether the session has been modified
  modified: bool,
}

impl Session {
  /// Creates a new `Session` wrapping the provided `SessionData`.
  ///
  /// The session starts in an unmodified state.
  ///
  /// # Arguments
  ///
  /// * `data` - The `SessionData` to wrap
  ///
  /// # Returns
  ///
  /// A new `Session` instance
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// use desirable::{Session, SessionData};
  ///
  /// let data = SessionData::new();
  /// let session = Session::new(data);
  /// ```
  pub fn new(data: SessionData) -> Self {
    Self {
      inner: data,
      modified: false,
    }
  }

  /// Returns the unique identifier of this session.
  ///
  /// The session ID is generated during creation and remains constant
  /// for the lifetime of the session.
  ///
  /// # Returns
  ///
  /// A string slice containing the session ID
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let session = manager.create_session();
  /// println!("Session ID: {}", session.id());
  /// ```
  pub fn id(&self) -> &str {
    &self.inner.id
  }

  /// Returns the timestamp when this session was created.
  ///
  /// # Returns
  ///
  /// A `DateTime<Utc>` representing the creation time
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  /// use chrono::Duration;
  ///
  /// let manager = SessionManager::with_random_key();
  /// let session = manager.create_session();
  /// let now = chrono::Utc::now();
  /// let age = now.signed_duration_since(session.created());
  /// println!("Session created {} seconds ago", age.num_seconds());
  /// ```
  pub fn created(&self) -> DateTime<Utc> {
    self.inner.created
  }

  /// Returns the timestamp of the last session access.
  ///
  /// This timestamp is updated when [`Session::touch`] is called,
  /// typically during session read or write operations.
  ///
  /// # Returns
  ///
  /// A `DateTime<Utc>` representing the last access time
  ///
  /// # See Also
  ///
  /// - [`Session::touch`] - Update the access timestamp
  pub fn accessed(&self) -> DateTime<Utc> {
    self.inner.accessed
  }

  /// Returns the number of key-value pairs stored in the session.
  ///
  /// # Returns
  ///
  /// The number of entries in the session data
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// assert_eq!(session.len(), 0);
  ///
  /// session.insert("a", 1).unwrap();
  /// session.insert("b", 2).unwrap();
  /// assert_eq!(session.len(), 2);
  /// ```
  pub fn len(&self) -> usize {
    self.inner.data.len()
  }

  /// Returns `true` if the session contains no key-value pairs.
  ///
  /// # Returns
  ///
  /// Whether the session is empty
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let session = manager.create_session();
  /// assert!(session.is_empty());
  /// ```
  pub fn is_empty(&self) -> bool {
    self.inner.data.is_empty()
  }

  /// Returns a reference to the underlying session data.
  ///
  /// This provides direct access to the `HashMap` storing the session values.
  ///
  /// # Returns
  ///
  /// A reference to the session data map
  ///
  /// # Note
  ///
  /// Modifying this map directly will not mark the session as modified.
  /// Use [`Session::insert`] or [`Session::data_mut`] instead.
  pub fn data(&self) -> &HashMap<String, String> {
    &self.inner.data
  }

  /// Returns `true` if the session contains the specified key.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to check for
  ///
  /// # Returns
  ///
  /// Whether the key exists in the session
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  ///
  /// assert!(!session.contains_key("user_id"));
  /// session.insert("user_id", 42).unwrap();
  /// assert!(session.contains_key("user_id"));
  /// ```
  pub fn contains_key(&self, key: &str) -> bool {
    self.inner.data.contains_key(key)
  }

  /// Returns `true` if the session has been modified since creation or last save.
  ///
  /// This is useful for optimizing cookie updates - only send a Set-Cookie
  /// header if the session was actually modified.
  ///
  /// # Returns
  ///
  /// Whether the session has been modified
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  ///
  /// if session.is_modified() {
  ///     // Update cookie
  /// }
  /// ```
  pub fn is_modified(&self) -> bool {
    self.modified
  }

  /// Retrieves a value from the session and deserializes it.
  ///
  /// Looks up the specified key and deserializes the stored JSON value
  /// back into the requested type.
  ///
  /// # Type Parameters
  ///
  /// * `T` - The type to deserialize the value as. Must implement
  ///         `serde::Deserialize`.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to look up
  ///
  /// # Returns
  ///
  /// `Ok(Some(value))` if the key exists, `Ok(None)` if not found,
  /// or an error if deserialization fails.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// session.insert("user_id", 42).unwrap();
  /// session.insert("name", "Alice").unwrap();
  /// session.insert("preferences", vec!["dark_mode", "notifications"]).unwrap();
  ///
  /// let user_id: Option<i32> = session.get("user_id").unwrap();
  /// assert_eq!(user_id, Some(42));
  ///
  /// let name: Option<String> = session.get("name").unwrap();
  /// assert_eq!(name, Some("Alice".to_string()));
  ///
  /// let missing: Option<i32> = session.get("nonexistent").unwrap();
  /// assert_eq!(missing, None);
  /// ```
  pub fn get<T>(&self, key: &str) -> Result<Option<T>>
  where
    T: for<'de> serde::de::Deserialize<'de>,
  {
    if let Some(value) = self.inner.data.get(key) {
      Ok(Some(serde_json::from_str(value)?))
    } else {
      Ok(None)
    }
  }

  /// Retrieves a string value from the session.
  ///
  /// This is a convenience method for retrieving values that are already strings,
  /// avoiding the overhead of JSON deserialization.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to look up
  ///
  /// # Returns
  ///
  /// `Some(&str)` if the key exists, `None` if not found
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// session.insert("count", 42).unwrap();
  ///
  /// let count = session.get_str("count");
  /// assert_eq!(count, Some("42"));
  /// ```
  pub fn get_str(&self, key: &str) -> Option<&str> {
    self.inner.data.get(key).map(|s| s.as_str())
  }

  /// Inserts a value into the session.
  ///
  /// Serializes the value to JSON and stores it under the given key.
  /// Marks the session as modified.
  ///
  /// # Type Parameters
  ///
  /// * `T` - The type to serialize. Must implement `serde::Serialize`.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to store the value under
  /// * `value` - The value to serialize and store
  ///
  /// # Returns
  ///
  /// `Ok(())` on success, or an error if serialization fails
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  ///
  /// session.insert("user_id", 42).unwrap();
  /// session.insert("username", "alice").unwrap();
  /// session.insert("logged_in", true).unwrap();
  /// ```
  pub fn insert<T>(&mut self, key: &str, value: T) -> Result<()>
  where
    T: Serialize,
  {
    let json = serde_json::to_string(&value)?;
    self.inner.data.insert(key.to_string(), json);
    self.modified = true;
    Ok(())
  }

  /// Removes and returns a value from the session.
  ///
  /// Looks up and removes the specified key, deserializing the stored value.
  /// Marks the session as modified.
  ///
  /// # Type Parameters
  ///
  /// * `T` - The type to deserialize the value as. Must implement
  ///         `serde::Deserialize`.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to remove
  ///
  /// # Returns
  ///
  /// `Ok(Some(value))` if the key existed, `Ok(None)` if not found,
  /// or an error if deserialization fails.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// session.insert("temp", "value").unwrap();
  ///
  /// let removed: Option<String> = session.remove("temp").unwrap();
  /// assert_eq!(removed, Some("value".to_string()));
  /// assert!(session.is_empty());
  /// ```
  pub fn remove<T>(&mut self, key: &str) -> Result<Option<T>>
  where
    T: for<'de> serde::de::Deserialize<'de>,
  {
    if let Some(value) = self.inner.data.remove(key) {
      self.modified = true;
      Ok(Some(serde_json::from_str(&value)?))
    } else {
      Ok(None)
    }
  }

  /// Removes and returns a string value from the session.
  ///
  /// This is a convenience method for removing string values.
  /// Marks the session as modified.
  ///
  /// # Arguments
  ///
  /// * `key` - The key to remove
  ///
  /// # Returns
  ///
  /// `Some(String)` if the key existed, `None` if not found
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// session.insert("count", 42).unwrap();
  ///
  /// let removed = session.remove_str("count");
  /// assert_eq!(removed, Some("42".to_string()));
  /// ```
  pub fn remove_str(&mut self, key: &str) -> Option<String> {
    let removed = self.inner.data.remove(key);
    if removed.is_some() {
      self.modified = true;
    }
    removed
  }

  /// Clears all data from the session.
  ///
  /// Removes all key-value pairs from the session data.
  /// Marks the session as modified.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// session.insert("a", 1).unwrap();
  /// session.insert("b", 2).unwrap();
  /// assert_eq!(session.len(), 2);
  ///
  /// session.clear();
  /// assert!(session.is_empty());
  /// assert!(session.is_modified());
  /// ```
  pub fn clear(&mut self) {
    if !self.inner.data.is_empty() {
      self.inner.data.clear();
      self.modified = true;
    }
  }

  /// Regenerates the session ID.
  ///
  /// Generates a new cryptographically random session ID and replaces
  /// the current one. This is useful for session fixation prevention
  /// after authentication.
  ///
  /// Marks the session as modified.
  ///
  /// # Security Note
  ///
  /// Call this method after successful login to prevent session fixation attacks.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// let old_id = session.id().to_string();
  ///
  /// session.regenerate_id();
  /// assert_ne!(session.id(), old_id);
  /// ```
  pub fn regenerate_id(&mut self) {
    let mut bytes = [0u8; SESSION_ID_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);
    self.inner.id = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    self.modified = true;
  }

  /// Updates the session's access timestamp.
  ///
  /// Sets the `accessed` field to the current time.
  /// This should be called when the session is accessed to track activity.
  ///
  /// # Note
  ///
  /// This method does NOT mark the session as modified, as accessing
  /// session data should not necessarily trigger a cookie update.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  /// use chrono::Duration;
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  /// let original_accessed = session.accessed();
  ///
  /// // Simulate some time passing
  /// session.touch();
  ///
  /// assert!(session.accessed() > original_accessed);
  /// ```
  pub fn touch(&mut self) {
    self.inner.accessed = Utc::now();
  }

  /// Consumes the session and returns the underlying `SessionData`.
  ///
  /// This is useful when you need to access the raw session data
  /// for serialization or custom handling.
  ///
  /// # Returns
  ///
  /// The `SessionData` wrapped by this session
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let session = manager.create_session();
  /// let data = session.into_data();
  ///
  /// println!("Session ID: {}", data.id);
  /// println!("Created: {:?}", data.created);
  /// ```
  pub fn into_data(self) -> SessionData {
    self.inner
  }

  /// Returns a mutable reference to the underlying data map.
  ///
  /// This allows direct manipulation of the session data HashMap.
  /// Marks the session as modified.
  ///
  /// # Returns
  ///
  /// A mutable reference to the session data HashMap
  ///
  /// # Warning
  ///
  /// Use this method with caution. Direct modification of the HashMap
  /// bypasses serialization checks. Prefer [`Session::insert`] when possible.
  ///
  /// # Example
  ///
  /// ```rust
  /// use desirable::{SessionManager, SessionConfig};
  ///
  /// let manager = SessionManager::with_random_key();
  /// let mut session = manager.create_session();
  ///
  /// session.data_mut().insert("custom_key".to_string(), "custom_value".to_string());
  /// assert_eq!(session.get_str("custom_key"), Some("custom_value"));
  /// ```
  pub fn data_mut(&mut self) -> &mut HashMap<String, String> {
    self.modified = true;
    &mut self.inner.data
  }
}

impl Default for Session {
  fn default() -> Self {
    Self::new(SessionData::new())
  }
}

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
    headers
      .get(http::header::COOKIE)
      .and_then(|v| v.to_str().ok())
      .and_then(|cookie_str| {
        cookie_str
          .split(';')
          .map(|s| s.trim())
          .find(|s| s.starts_with(&format!("{}=", self.config.cookie_name)))
          .and_then(|s| s.split('=').nth(1).map(|s| s.to_string()))
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_session_new() {
    let session = Session::new(SessionData::new());
    assert!(!session.id().is_empty());
    assert!(session.is_empty());
    assert!(!session.is_modified());
  }

  #[test]
  fn test_session_insert_get() {
    let mut session = Session::new(SessionData::new());
    session.insert("user_id", 42).unwrap();
    session.insert("name", "Alice").unwrap();
    assert_eq!(session.len(), 2);
    assert!(session.is_modified());

    let user_id: Option<i32> = session.get("user_id").unwrap();
    assert_eq!(user_id, Some(42));

    let name: Option<String> = session.get("name").unwrap();
    assert_eq!(name, Some("Alice".to_string()));
  }

  #[test]
  fn test_session_remove() {
    let mut session = Session::new(SessionData::new());
    session.insert("key", "value").unwrap();
    let removed: Option<String> = session.remove("key").unwrap();
    assert_eq!(removed, Some("value".to_string()));
    assert!(session.is_empty());
  }

  #[test]
  fn test_session_manager_roundtrip() {
    let key = b"this-is-a-32-byte-secret-key-!!!!";
    let manager = SessionManager::new(SessionConfig::new(key));

    let mut session = manager.create_session();
    session.insert("user_id", 123).unwrap();
    session.insert("name", "Bob").unwrap();

    let cookie_value = manager.write_session(&session);
    assert!(!cookie_value.is_empty());

    let loaded = manager.read_session(&cookie_value).unwrap().unwrap();
    assert_eq!(loaded.id(), session.id());
    let user_id: Option<i32> = loaded.get("user_id").unwrap();
    assert_eq!(user_id, Some(123));
  }

  #[test]
  fn test_session_cookie_header() {
    let key = b"this-is-a-32-byte-secret-key-!!!!";
    let config = SessionConfig::new(key).secure(false).http_only(true);
    let manager = SessionManager::new(config);

    let session = manager.create_session();
    let header = manager.make_cookie_header(&session);
    assert!(!header.to_str().unwrap().is_empty());
    assert!(header.to_str().unwrap().contains("HttpOnly"));
  }
}
