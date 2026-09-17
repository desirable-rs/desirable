use crate::Response;
use std::sync::Arc;
use std::sync::OnceLock;
use thiserror::Error;

type ErrorRenderer = Arc<dyn Fn(Error) -> Response + Send + Sync>;

static ERROR_RENDERER: OnceLock<ErrorRenderer> = OnceLock::new();

/// Sets a process-wide custom renderer for [`Error`] responses.
///
/// Call this once at startup (e.g. from `main`) to render every handler
/// error through your own function — typically to emit a uniform JSON error
/// envelope. Setting it twice keeps the first renderer.
///
/// Without a custom renderer, client errors (`4xx`) keep their message and
/// server errors (`5xx`) return a generic `"internal server error"` body.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{set_error_handler, Response};
///
/// set_error_handler(|err| {
///   Response::builder()
///     .status(err.status())
///     .json(serde_json::json!({
///       "error": err.to_string(),
///       "status": err.status().as_u16(),
///     }))
/// });
/// ```
pub fn set_error_handler(renderer: impl Fn(Error) -> Response + Send + Sync + 'static) {
  let _ = ERROR_RENDERER.set(Arc::new(renderer));
}

/// Renders an error to a response via the custom renderer when installed,
/// otherwise through the default 4xx/5xx policy.
pub(crate) fn render_error(err: Error) -> Response {
  match ERROR_RENDERER.get() {
    Some(renderer) => renderer(err),
    None => default_render_error(err),
  }
}

fn default_render_error(err: Error) -> Response {
  use hyper::StatusCode;

  let status = err.status();
  if err.is_server_error() {
    // Never leak internal details to clients; log the real error instead.
    tracing::error!(error = %err, "handler failed");
    Response::with_status_code(
      StatusCode::INTERNAL_SERVER_ERROR,
      "internal server error".to_string(),
    )
  } else {
    Response::with_status_code(status, err.to_string())
  }
}

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
  /// Request body exceeded the configured size limit
  #[error("request body too large")]
  BodyTooLarge,
}

impl Error {
  /// Returns the HTTP status code that best represents this error.
  ///
  /// Client errors (bad parameters, malformed payloads, oversized bodies)
  /// map to `4xx`; everything else maps to `500 Internal Server Error`.
  pub fn status(&self) -> hyper::StatusCode {
    use hyper::StatusCode;
    match self {
      Error::MissingParam { .. }
      | Error::InvalidParam { .. }
      | Error::Urlencoded(_)
      | Error::Json(_) => StatusCode::BAD_REQUEST,
      Error::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
      _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
  }

  /// Returns `true` for server-side errors whose details should not be
  /// leaked to clients.
  pub fn is_server_error(&self) -> bool {
    self.status() == hyper::StatusCode::INTERNAL_SERVER_ERROR
  }
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
    render_error(err)
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

  #[test]
  fn test_error_status_codes() {
    use hyper::StatusCode;

    // Client errors map to 4xx.
    assert_eq!(missing_param("id").status(), StatusCode::BAD_REQUEST);
    assert_eq!(Error::BodyTooLarge.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let parse_err: std::num::ParseIntError = "x".parse::<u32>().unwrap_err();
    assert_eq!(
      invalid_param("age", "u32", parse_err).status(),
      StatusCode::BAD_REQUEST
    );

    // Everything else stays a server error.
    assert_eq!(
      error_msg("boom").status(),
      StatusCode::INTERNAL_SERVER_ERROR
    );
  }

  #[test]
  fn test_server_errors_do_not_leak_details() {
    let response: Response = error_msg("secret-db-password").into();
    let body = response
      .inner
      .body()
      .clone()
      .into_inner()
      .expect("full body has data");
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("secret-db-password"));
    assert_eq!(text, "internal server error");
  }

  #[test]
  fn test_client_errors_keep_message() {
    let response: Response = missing_param("user_id").into();
    let body = response
      .inner
      .body()
      .clone()
      .into_inner()
      .expect("full body has data");
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("user_id"), "got: {}", text);
  }
}
