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

mod config;
mod data;
mod error;
mod manager;

pub use config::SessionConfig;
pub use data::SessionData;
pub use error::SessionError;
pub use manager::SessionManager;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use rand::RngCore as _;
use serde::Serialize;
use std::collections::HashMap;

use crate::Result;

const SESSION_ID_LENGTH: usize = 32;

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
  /// Whether the session was destroyed (cleared and scheduled for deletion)
  destroyed: bool,
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
      destroyed: false,
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

  /// Destroys the session: clears all data and schedules a deletion cookie
  /// via the [`SessionLayer`](crate::SessionLayer) middleware.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// req.session().lock().unwrap().destroy(); // logout
  /// ```
  pub fn destroy(&mut self) {
    self.inner.data.clear();
    self.destroyed = true;
    self.modified = true;
  }

  /// Returns `true` after [`Session::destroy`] was called.
  pub fn is_destroyed(&self) -> bool {
    self.destroyed
  }

  /// Retrieves a value from the session and deserializes it.
  ///
  /// Looks up the specified key and deserializes the stored JSON value
  /// back into the requested type.
  ///
  /// # Type Parameters
  ///
  /// * `T` - The type to deserialize the value as. Must implement
  ///   `serde::Deserialize`.
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
  ///   `serde::Deserialize`.
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
    rand::rng().fill_bytes(&mut bytes);
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

  #[test]
  fn test_session_expiry_enforced_server_side() {
    let key = b"this-is-a-32-byte-secret-key-!!!!";
    // max_age_secs(-1): every session is already expired.
    let expired_manager = SessionManager::new(SessionConfig::new(key).max_age_secs(-1));
    let mut session = expired_manager.create_session();
    session.insert("user_id", 1).unwrap();
    let cookie = expired_manager.write_session(&session);

    let err = expired_manager.read_session(&cookie).unwrap_err();
    assert!(matches!(
      err,
      crate::error::Error::Session(crate::session::SessionError::Expired)
    ));

    // A non-expired session still verifies.
    let live_manager = SessionManager::new(SessionConfig::new(key).max_age_secs(3600));
    let mut live = live_manager.create_session();
    live.insert("user_id", 1).unwrap();
    let cookie = live_manager.write_session(&live);
    let loaded = live_manager.read_session(&cookie).unwrap().unwrap();
    let user_id: Option<i32> = loaded.get("user_id").unwrap();
    assert_eq!(user_id, Some(1));
  }

  #[test]
  fn test_regenerate_id_changes_identity() {
    let manager = SessionManager::with_random_key();
    let mut session = manager.create_session();
    let old = session.id().to_string();
    session.regenerate_id();
    assert_ne!(session.id(), old);
  }

  #[test]
  fn test_cookie_value_with_base64_padding_survives() {
    // Regression test: cookie values are base64 and usually end in '='
    // padding. The old `split('=').nth(1)` lookup truncated at the first
    // '=' inside the value, breaking signature verification.
    let key = b"this-is-a-32-byte-secret-key-!!!!";
    let manager = SessionManager::new(SessionConfig::new(key));

    // Find a session whose serialized cookie ends with '=' padding.
    let mut cookie_value = String::new();
    let mut session = manager.create_session();
    for i in 0..64 {
      session.insert("i", i).unwrap();
      cookie_value = manager.write_session(&session);
      if cookie_value.ends_with('=') {
        break;
      }
      session = manager.create_session();
    }
    assert!(
      cookie_value.ends_with('='),
      "expected a padded base64 value in 64 tries"
    );

    let mut headers = hyper::HeaderMap::new();
    headers.insert(
      hyper::header::COOKIE,
      format!("desirable_session={}", cookie_value)
        .parse()
        .unwrap(),
    );

    // The full value (padding included) must come back and must verify.
    let got = manager.get_cookie_value(&headers).unwrap();
    assert_eq!(got, cookie_value);
    assert!(manager.read_session(&got).unwrap().is_some());
  }
}
