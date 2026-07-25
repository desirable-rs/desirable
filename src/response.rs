use crate::{HyperResponse, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::StatusCode;
use hyper::header;

/// Cached content-type header value for JSON responses.
static CONTENT_TYPE_JSON: header::HeaderValue =
  header::HeaderValue::from_static("application/json");

/// Cached content-type header value for plain text responses.
static CONTENT_TYPE_TEXT: header::HeaderValue =
  header::HeaderValue::from_static("text/plain; charset=utf-8");

/// Cached content-type header value for octet-stream responses.
/// Currently unused but kept for potential future byte-based responses.
#[allow(dead_code)]
static CONTENT_TYPE_OCTET: header::HeaderValue =
  header::HeaderValue::from_static("application/octet-stream");

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
      .header(header::CONTENT_TYPE, CONTENT_TYPE_TEXT.clone())
      .status(StatusCode::from_u16(status)?)
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
  /// A JSON response with `Content-Type: application/json`
  ///
  /// # Panics
  ///
  /// Panics if serialization fails. This should never happen for well-formed types.
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
  /// // Now supports references without 'static bound:
  /// let user = User { id: 1, name: "Alice".into() };
  /// Response::json(&user)
  /// ```
  pub fn json<T: serde::Serialize>(payload: T) -> Self {
    let data = serde_json::to_vec(&payload).expect("JSON serialization failed");
    hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, CONTENT_TYPE_JSON.clone())
      .body(Full::new(Bytes::from(data)))
      .unwrap()
      .into()
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

  /// Sets a header on this response (mutable reference).
  ///
  /// # Arguments
  ///
  /// * `key` - The header name
  /// * `value` - The header value
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// response.set_header(header::CONTENT_TYPE, "application/json".parse().unwrap());
  /// ```
  pub fn set_header(&mut self, key: hyper::header::HeaderName, value: hyper::header::HeaderValue) {
    self.inner.headers_mut().insert(key, value);
  }

  /// Builder-style header setter. Consumes self and returns Self for chaining.
  ///
  /// # Arguments
  ///
  /// * `key` - The header name
  /// * `value` - The header value
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let response = Response::body("hello")?
  ///     .with_header(header::CONTENT_TYPE, "text/plain".parse().unwrap());
  /// ```
  #[must_use]
  pub fn with_header(
    mut self,
    key: hyper::header::HeaderName,
    value: hyper::header::HeaderValue,
  ) -> Self {
    self.set_header(key, value);
    self
  }

  /// Returns a [`ResponseBuilder`] for constructing a response with the
  /// builder pattern.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// Response::builder()
  ///     .status(StatusCode::CREATED)
  ///     .header("X-Custom".parse().unwrap(), "value".parse().unwrap())
  ///     .json(&my_data)
  /// ```
  #[must_use]
  pub fn builder() -> ResponseBuilder {
    ResponseBuilder::new()
  }
}

/// Builder for constructing HTTP responses.
///
/// Provides chainable methods for setting status, headers, and body.
/// Created via [`Response::builder()`].
pub struct ResponseBuilder {
  inner: hyper::http::response::Builder,
}

impl ResponseBuilder {
  fn new() -> Self {
    Self {
      inner: hyper::http::response::Builder::new(),
    }
  }

  /// Sets the HTTP status code.
  #[must_use]
  pub fn status(mut self, status: hyper::StatusCode) -> Self {
    self.inner = self.inner.status(status);
    self
  }

  /// Sets the HTTP status code from a `u16`.
  ///
  /// # Panics
  ///
  /// Panics if the status code is invalid (not in the range 100-999).
  #[must_use]
  pub fn status_u16(self, status: u16) -> Self {
    self.status(hyper::StatusCode::from_u16(status).expect("invalid HTTP status code"))
  }

  /// Sets a header on the response.
  #[must_use]
  pub fn header(
    mut self,
    key: hyper::header::HeaderName,
    value: hyper::header::HeaderValue,
  ) -> Self {
    self.inner = self.inner.header(key, value);
    self
  }

  /// Sets the response body as plain text with `Content-Type: text/plain; charset=utf-8`.
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an HTTP builder error.
  pub fn text<T>(self, body: T) -> Result<Response>
  where
    Bytes: From<T>,
  {
    let response = self
      .inner
      .header(header::CONTENT_TYPE, CONTENT_TYPE_TEXT.clone())
      .body(Full::new(Bytes::from(body)))?
      .into();
    Ok(response)
  }

  /// Sets the response body as JSON with `Content-Type: application/json`.
  ///
  /// # Panics
  ///
  /// Panics if serialization fails.
  pub fn json<T: serde::Serialize>(self, payload: T) -> Response {
    let data = serde_json::to_vec(&payload).expect("JSON serialization failed");
    self
      .inner
      .header(header::CONTENT_TYPE, CONTENT_TYPE_JSON.clone())
      .body(Full::new(Bytes::from(data)))
      .unwrap()
      .into()
  }

  /// Sets the response body without modifying headers.
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an HTTP builder error.
  pub fn body<T>(self, body: T) -> Result<Response>
  where
    Bytes: From<T>,
  {
    let response = self.inner.body(Full::new(Bytes::from(body)))?.into();
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
      .header(header::CONTENT_TYPE, &CONTENT_TYPE_TEXT)
      .body(Full::new(Bytes::default()))
      .unwrap()
      .into()
  }
}

impl From<String> for Response {
  fn from(val: String) -> Self {
    hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, &CONTENT_TYPE_TEXT)
      .body(Full::new(Bytes::from(val)))
      .unwrap()
      .into()
  }
}

impl From<&'static str> for Response {
  fn from(val: &'static str) -> Self {
    hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, &CONTENT_TYPE_TEXT)
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
    let response = Response::json(data);
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[test]
  fn test_response_json_ref() {
    #[derive(serde::Serialize)]
    struct TestData {
      name: String,
    }
    let data = TestData {
      name: "ref".to_string(),
    };
    // Can now pass references without 'static bound
    let response = Response::json(&data);
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

  #[test]
  fn test_response_set_header() {
    let mut response = Response::body("test").unwrap();
    response.set_header(
      hyper::header::HeaderName::from_static("x-custom"),
      hyper::header::HeaderValue::from_static("value"),
    );
    assert_eq!(response.inner.headers().get("x-custom").unwrap(), "value");
  }

  #[test]
  fn test_response_with_header() {
    let response = Response::body("test").unwrap().with_header(
      hyper::header::HeaderName::from_static("x-custom"),
      hyper::header::HeaderValue::from_static("chained"),
    );
    assert_eq!(response.inner.headers().get("x-custom").unwrap(), "chained");
  }

  #[test]
  fn test_response_builder_json() {
    #[derive(serde::Serialize)]
    struct Msg {
      msg: String,
    }
    let response = Response::builder()
      .status(hyper::StatusCode::CREATED)
      .json(Msg {
        msg: "created".into(),
      });
    assert_eq!(response.status(), hyper::StatusCode::CREATED);
  }

  #[test]
  fn test_response_builder_text() {
    let response = Response::builder()
      .status(hyper::StatusCode::NOT_FOUND)
      .text("not found")
      .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::NOT_FOUND);
  }

  #[test]
  fn test_response_builder_body() {
    let response = Response::builder()
      .status_u16(200)
      .body("raw body")
      .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::OK);
  }

  #[test]
  fn test_response_builder_header() {
    let response = Response::builder()
      .header(
        hyper::header::HeaderName::from_static("x-custom"),
        hyper::header::HeaderValue::from_static("builder-value"),
      )
      .text("ok")
      .unwrap();
    assert_eq!(
      response.inner.headers().get("x-custom").unwrap(),
      "builder-value"
    );
  }
}
