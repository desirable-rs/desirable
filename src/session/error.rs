//! Error type for session operations.

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
