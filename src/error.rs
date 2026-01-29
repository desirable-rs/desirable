use crate::Response;
use thiserror::Error;

/// The error type for the desirable framework.
///
/// Represents all possible errors that can occur during request handling.
/// Uses `thiserror` for ergonomic error definitions and conversions.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::Error;
///
/// async fn handler() -> Result<String, Error> {
///   Err(Error::Message {
///     msg: "Something went wrong".to_string(),
///   })
/// }
/// ```
#[derive(Debug, Error)]
pub enum Error {
  /// Hyper I/O error
  #[error("hyper error {0:?}")]
  Hyper(#[from] hyper::Error),
  /// HTTP protocol error
  #[error("http error {0:?}")]
  HTTP(#[from] hyper::http::Error),
  /// JSON serialization/deserialization error
  #[error("json error {0:?}")]
  Json(#[from] serde_json::Error),
  /// I/O error (file operations, etc.)
  #[error("IO error {0:?}")]
  IO(#[from] std::io::Error),
  /// Generic error wrapped in anyhow
  #[error("any error {0:?}")]
  Any(#[from] anyhow::Error),
  /// URL query string parsing error
  #[error("query error {0:?}")]
  Urlencoded(#[from] serde_urlencoded::de::Error),
  /// Socket address parsing error
  #[error("addr parse error {0:?}")]
  AddrParse(#[from] std::net::AddrParseError),
  /// Invalid HTTP status code
  #[error("InvalidStatusCode {0:?}")]
  InvalidStatusCode(#[from] hyper::http::status::InvalidStatusCode),
  /// Missing URL path parameter
  #[error("missing url param {name:?}")]
  MissingParam { name: String },
  /// Invalid path parameter value
  #[error("invalid param {name:?} as {expected:?}, {err:?}")]
  InvalidParam {
    /// The parameter name
    name: String,
    /// The expected type
    expected: &'static str,
    /// The parse error
    err: String,
  },
  /// Custom error message
  #[error("error msg {msg:?}")]
  Message { msg: String },
  /// Session error
  #[error("session error {0:?}")]
  Session(#[from] crate::session::SessionError),
}

/// Creates a `MissingParam` error.
///
/// # Arguments
///
/// * `name` - The name of the missing parameter
///
/// # Returns
///
/// An `Error::MissingParam` variant
///
/// # Example
///
/// ```rust,ignore
/// Err(missing_param("user_id"))
/// ```
pub fn missing_param(name: &str) -> Error {
  Error::MissingParam {
    name: name.to_string(),
  }
}

/// Creates a `Message` error with a custom message.
///
/// # Arguments
///
/// * `msg` - The error message
///
/// # Returns
///
/// An `Error::Message` variant
///
/// # Example
///
/// ```rust,ignore
/// Err(error_msg("Custom error occurred"))
/// ```
pub fn error_msg(msg: &str) -> Error {
  Error::Message {
    msg: msg.to_string(),
  }
}

/// Creates an `InvalidParam` error for parameter parsing failures.
///
/// # Arguments
///
/// * `name` - The parameter name
/// * `expected` - The expected type description
/// * `err` - The parse error
///
/// # Returns
///
/// An `Error::InvalidParam` variant
///
/// # Example
///
/// ```rust,ignore
/// Err(invalid_param("id", "i32", e))
/// ```
pub fn invalid_param(
  name: impl ToString,
  expected: &'static str,
  err: impl std::error::Error,
) -> Error {
  Error::InvalidParam {
    name: name.to_string(),
    expected,
    err: err.to_string(),
  }
}

impl From<Error> for Response {
  fn from(err: Error) -> Self {
    Response::with_status(500, err.to_string()).unwrap()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_missing_param() {
    let err = missing_param("user_id");
    assert!(matches!(err, Error::MissingParam { .. }));
    if let Error::MissingParam { name } = err {
      assert_eq!(name, "user_id");
    }
  }

  #[test]
  fn test_error_msg() {
    let err = error_msg("test message");
    assert!(matches!(err, Error::Message { .. }));
    if let Error::Message { msg } = err {
      assert_eq!(msg, "test message");
    }
  }

  #[test]
  fn test_invalid_param() {
    use std::num::ParseIntError;
    let parse_err: ParseIntError = "invalid digit".parse::<u32>().unwrap_err();
    let err = invalid_param("age", "u32", parse_err);
    assert!(matches!(err, Error::InvalidParam { .. }));
    if let Error::InvalidParam {
      name,
      expected,
      err: err_msg,
    } = err
    {
      assert_eq!(name, "age");
      assert_eq!(expected, "u32");
      assert!(err_msg.contains("invalid"));
    }
  }

  #[test]
  fn test_error_display() {
    let err = missing_param("id");
    let display = format!("{}", err);
    assert!(display.contains("missing url param"));
    assert!(display.contains("id"));
  }
}
