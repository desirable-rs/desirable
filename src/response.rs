use crate::{HyperResponse, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::StatusCode;
use hyper::header;

/// The HTTP response type for the desirable framework.
///
/// Provides constructors for common response types and ergonomic builders.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Response, Result};
///
/// async fn handler() -> Result {
///   Response::json(&User { name: "Alice" })
/// }
/// ```
pub struct Response {
  /// The underlying hyper response
  pub inner: HyperResponse,
}

impl Response {
  /// Creates a new Response with the given hyper response.
  fn new(response: HyperResponse) -> Self {
    Self { inner: response }
  }

  /// Returns the response status code.
  pub fn status(&self) -> StatusCode {
    self.inner.status()
  }

  /// Creates a text response with the given body.
  ///
  /// # Type Parameters
  ///
  /// * `T` - Any type that can be converted to `Bytes`
  ///
  /// # Arguments
  ///
  /// * `body` - The response body
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an error
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// Response::body("Hello, World!")
  /// ```
  pub fn body<T>(body: T) -> Result<Self>
  where
    Bytes: From<T>,
  {
    let response = hyper::http::Response::builder()
      .body(Full::new(Bytes::from(body)))?
      .into();
    Ok(response)
  }

  /// Creates a text response with a specific status code.
  ///
  /// # Arguments
  ///
  /// * `status` - The HTTP status code (e.g., 200, 404, 500)
  /// * `val` - The response body as a string
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an error
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// Response::with_status(404, "Not Found".to_string())
  /// ```
  pub fn with_status(status: u16, val: String) -> Result<Self> {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(hyper::StatusCode::from_u16(status)?)
      .body(Full::new(Bytes::from(val)))?
      .into();
    Ok(response)
  }

  /// Creates a JSON response with the given serializable payload.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::Serialize`
  ///
  /// # Arguments
  ///
  /// * `payload` - The data to serialize as JSON
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or a serialization error
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Serialize)]
  /// struct User {
  ///   id: i32,
  ///   name: String,
  /// }
  ///
  /// Response::json(&User { id: 1, name: "Alice" })
  /// ```
  pub fn json<T>(payload: T) -> Result<Self>
  where
    T: serde::Serialize + Sized + Send + Sync + 'static,
  {
    let data = serde_json::to_string(&payload)?;
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::APPLICATION_JSON.to_string())
      .body(Full::new(Bytes::from(data)))?
      .into();
    Ok(response)
  }

  /// Creates a redirect response to the given URL.
  ///
  /// # Arguments
  ///
  /// * `status` - The redirect status code (301, 302, 303, 307, 308)
  /// * `url` - The URL to redirect to
  ///
  /// # Returns
  ///
  /// A `Result` containing the redirect response or an error
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// Response::redirect(302, "/new-location")
  /// ```
  pub fn redirect(status: u16, url: &str) -> Result<Self> {
    let response = hyper::http::Response::builder()
      .status(hyper::StatusCode::from_u16(status)?)
      .header(header::LOCATION, url)
      .body(Full::new(Bytes::default()))?
      .into();
    Ok(response)
  }
}

impl From<HyperResponse> for Response {
  fn from(response: HyperResponse) -> Self {
    Response::new(response)
  }
}

impl From<()> for Response {
  fn from(_: ()) -> Self {
    hyper::http::Response::builder()
      .header(
        hyper::header::CONTENT_TYPE,
        mime::TEXT_PLAIN_UTF_8.to_string(),
      )
      .body(Full::new(Bytes::default()))
      .unwrap()
      .into()
  }
}

impl From<String> for Response {
  fn from(val: String) -> Self {
    hyper::http::Response::builder()
      .header(
        hyper::header::CONTENT_TYPE,
        mime::TEXT_PLAIN_UTF_8.to_string(),
      )
      .body(Full::new(Bytes::from(val)))
      .unwrap()
      .into()
  }
}

impl From<&'static str> for Response {
  fn from(val: &'static str) -> Self {
    hyper::http::Response::builder()
      .header(
        hyper::header::CONTENT_TYPE,
        mime::TEXT_PLAIN_UTF_8.to_string(),
      )
      .body(Full::new(Bytes::from(val)))
      .unwrap()
      .into()
  }
}

impl From<anyhow::Error> for Response {
  fn from(e: anyhow::Error) -> Self {
    Response::with_status(500, e.to_string()).unwrap()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_response_status() {
    let response = Response::with_status(200, "OK".to_string()).unwrap();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_with_status() {
    let response = Response::with_status(404, "Not Found".to_string()).unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
  }

  #[test]
  fn test_response_body() {
    let response = Response::body("Hello").unwrap();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_json() {
    #[derive(serde::Serialize)]
    struct TestData {
      name: String,
      value: i32,
    }
    let data = TestData {
      name: "test".to_string(),
      value: 42,
    };
    let response = Response::json(data).unwrap();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_redirect() {
    let response = Response::redirect(302, "/redirected").unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
  }

  #[test]
  fn test_response_from_string() {
    let response: Response = "hello".into();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_from_static_str() {
    let response: Response = "static".into();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_from_unit() {
    let response: Response = ().into();
    assert_eq!(response.status(), StatusCode::OK);
  }
}
