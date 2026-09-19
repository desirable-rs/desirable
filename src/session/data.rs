//! The serializable payload stored inside a session cookie.

use base64::Engine as _;
use chrono::{DateTime, Utc};
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(super) const DEFAULT_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;
pub(super) const SESSION_ID_LENGTH: usize = 32;
pub(super) const DEFAULT_COOKIE_NAME: &str = "desirable_session";

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
    rand::rng().fill_bytes(&mut bytes);
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
