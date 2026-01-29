use crate::{Error, Response, Result};
use bytes::{Bytes, BytesMut};
use http_body_util::Full;
use hyper::header;
use std::borrow::Cow;

/// Trait for types that can be converted into an HTTP response.
///
/// This is the core trait for handler return types in the framework.
/// Many standard types implement this trait automatically, allowing
/// handlers to return strings, JSON data, responses, errors, and more.
///
/// # Implementors
///
/// The following types implement `IntoResponse`:
/// - [`Response`] - Pass-through
/// - `()` - Empty response with status 200
/// - `&'static str` - Text response
/// - `String` - Text response
/// - [`Error`] - Error response with status 500
/// - `Bytes` / `BytesMut` - Raw bytes with octet-stream content type
/// - `(StatusCode, &str)` / `(StatusCode, String)` - Status with body
/// - `(u16, &str)` / `(u16, String)` - Status code with body
/// - `Result<T, E>` where T, E: IntoResponse - Flattens the result
///
/// # Example
///
/// ```rust,ignore
/// use desirable::IntoResponse;
///
/// async fn handler() -> impl IntoResponse {
///   "Hello, World!"
/// }
///
/// async fn json_handler() -> impl IntoResponse {
///   serde_json::json!({ "message": "Hello" })
/// }
///
/// async fn error_handler() -> impl IntoResponse {
///   desirable::Error::Message {
///     msg: "Something went wrong".to_string()
///   }
/// }
/// ```
pub trait IntoResponse {
  /// Converts this type into a response.
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an error
  fn into_response(self) -> Result;
}

impl IntoResponse for Full<Bytes> {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder().body(self)?.into();
    Ok(response)
  }
}

impl IntoResponse for &'static str {
  fn into_response(self) -> Result {
    Cow::Borrowed(self).into_response()
  }
}

impl IntoResponse for String {
  fn into_response(self) -> Result {
    Cow::<'static, str>::Owned(self).into_response()
  }
}

impl IntoResponse for Cow<'static, str> {
  fn into_response(self) -> Result {
    let mut res = Full::from(self).into_response()?;
    res.inner.headers_mut().insert(
      header::CONTENT_TYPE,
      header::HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
    );
    Ok(res)
  }
}

impl<T, E> IntoResponse for std::result::Result<T, E>
where
  T: IntoResponse,
  E: IntoResponse,
{
  fn into_response(self) -> Result {
    match self {
      Ok(response) => response.into_response(),
      Err(err) => err.into_response(),
    }
  }
}

impl IntoResponse for Error {
  fn into_response(self) -> Result {
    let val = self.to_string();
    Response::with_status(500, val)
  }
}

impl IntoResponse for () {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .body(Full::new(Bytes::default()))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for Response {
  fn into_response(self) -> Result {
    Ok(self)
  }
}

impl IntoResponse for (hyper::StatusCode, String) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(self.0)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (hyper::StatusCode, &'static str) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(self.0)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (u16, String) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(hyper::StatusCode::from_u16(self.0)?)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (u16, &'static str) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(hyper::StatusCode::from_u16(self.0)?)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for Bytes {
  fn into_response(self) -> Result {
    let mut res = Full::from(self).into_response()?;
    res.inner.headers_mut().insert(
      header::CONTENT_TYPE,
      header::HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
    );
    Ok(res)
  }
}

impl IntoResponse for BytesMut {
  fn into_response(self) -> Result {
    self.freeze().into_response()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_into_response_static_str() {
    let response: Result = "hello".into_response();
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), hyper::StatusCode::OK);
  }

  #[test]
  fn test_into_response_string() {
    let response: Result = String::from("hello").into_response();
    assert!(response.is_ok());
  }

  #[test]
  fn test_into_response_unit() {
    let response: Result = ().into_response();
    assert!(response.is_ok());
  }

  #[test]
  fn test_into_response_status_code_and_str() {
    let response: Result = (hyper::StatusCode::NOT_FOUND, "Not Found").into_response();
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), hyper::StatusCode::NOT_FOUND);
  }

  #[test]
  fn test_into_response_u16_and_str() {
    let response: Result = (404, "Not Found").into_response();
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), hyper::StatusCode::NOT_FOUND);
  }

  #[test]
  fn test_into_response_result_ok() {
    let response: Result = Ok::<&str, &str>("ok").into_response();
    assert!(response.is_ok());
  }

  #[test]
  fn test_into_response_result_err() {
    let response: Result = Err::<(), &str>("error").into_response();
    assert!(response.is_ok());
  }

  #[test]
  fn test_into_response_bytes() {
    let bytes: Bytes = Bytes::from(vec![1, 2, 3]);
    let response: Result = bytes.into_response();
    assert!(response.is_ok());
  }
}
