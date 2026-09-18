use crate::HyperRequest;
use crate::Result;
use crate::error::{invalid_param, missing_param};
use bytes::Buf;
use bytes::Bytes;
use hyper::http::Extensions;
use route_recognizer::Params;
use std::any::Any;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tracing::debug;

/// Extension carrying the resolved client IP (see
/// [`Request::client_ip`](Request::client_ip)).
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

/// The HTTP request type for the desirable framework.
///
/// Contains the underlying hyper request, route parameters, and remote address.
/// Provides ergonomic accessors for common request data.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Request, Result};
///
/// async fn handler(req: Request) -> Result {
///   let method = req.method();
///   let path = req.path();
///   let user_id: i32 = req.param("id")?;
///   Ok(format!("Hello {} at {}", user_id, path).into())
/// }
/// ```
#[derive(Debug)]
pub struct Request {
  /// The underlying hyper request
  pub inner: HyperRequest,
  /// Route parameters extracted from the URL path
  pub params: Params,
  /// The remote socket address of the client
  pub remote_addr: Option<Arc<SocketAddr>>,
}

impl Request {
  /// Creates a new Request with the given hyper request and remote address.
  ///
  /// # Arguments
  ///
  /// * `request` - The underlying hyper request
  /// * `remote_addr` - The client's socket address, if available
  pub fn new(request: HyperRequest, remote_addr: Option<Arc<SocketAddr>>) -> Self {
    Self {
      inner: request,
      params: Params::new(),
      remote_addr,
    }
  }

  /// Creates a new Request from a hyper request without a remote address.
  ///
  /// This is a convenience constructor for cases where the remote address is not needed.
  pub fn mk_request(request: HyperRequest) -> Self {
    Request::new(request, None)
  }

  /// Returns a reference to the HTTP method.
  pub fn method(&self) -> &hyper::Method {
    self.inner.method()
  }

  /// Returns a reference to the URI.
  pub fn uri(&self) -> &hyper::Uri {
    self.inner.uri()
  }

  /// Returns the path component of the URI.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let path = req.path(); // "/users/123"
  /// ```
  pub fn path(&self) -> &str {
    self.inner.uri().path()
  }

  /// Returns a reference to the route parameters.
  pub fn params(&self) -> &Params {
    &self.params
  }

  /// Returns a reference to the request extensions.
  ///
  /// Extensions can be used to share data between middleware and handlers.
  pub fn extensions(&self) -> &Extensions {
    self.inner.extensions()
  }

  /// Returns a mutable reference to the request extensions.
  ///
  /// Extensions can be used to share data between middleware and handlers.
  pub fn extensions_mut(&mut self) -> &mut Extensions {
    self.inner.extensions_mut()
  }

  /// Returns a mutable reference to the underlying hyper request.
  pub fn inner(&mut self) -> &mut HyperRequest {
    &mut self.inner
  }

  /// Deserializes the request body as JSON into type `T`.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::DeserializeOwned`
  ///
  /// # Returns
  ///
  /// The deserialized body, or an error if deserialization fails
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Deserialize)]
  /// struct User {
  ///   name: String,
  ///   email: String,
  /// }
  ///
  /// async fn create_user(mut req: Request) -> Result {
  ///   let user: User = req.body().await?;
  ///   Ok(format!("Created {}", user.name).into())
  /// }
  /// ```
  pub async fn body<T>(&mut self) -> Result<T>
  where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
  {
    let inner = self.inner();
    let bytes = Self::collect_body_limited(inner).await?;
    let payload: T = serde_json::from_reader(bytes.reader()).map_err(crate::error::Error::Json)?;
    Ok(payload)
  }

  /// Collects the request body, honoring a [`BodyLimitValue`] extension when
  /// the [`crate::BodyLimit`] middleware is installed.
  ///
  /// Exceeding the limit yields `Error::BodyTooLarge` (HTTP 413).
  async fn collect_body_limited(
    req: &mut HyperRequest,
  ) -> std::result::Result<Bytes, crate::Error> {
    use http_body_util::BodyExt as _;

    let limit = req
      .extensions()
      .get::<crate::middleware::body_limit::BodyLimitValue>()
      .map(|l| l.0);
    let body = req.body_mut();
    let collected = match limit {
      Some(max) => http_body_util::Limited::new(body, max)
        .collect()
        .await
        .map_err(|err| {
          if err
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
          {
            crate::Error::BodyTooLarge
          } else {
            crate::Error::Any(anyhow::anyhow!("failed to read request body: {err}"))
          }
        })?,
      None => body.collect().await.map_err(crate::Error::Hyper)?,
    };
    Ok(collected.to_bytes())
  }

  /// Parses the query string into type `T`.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::DeserializeOwned`
  ///
  /// # Returns
  ///
  /// `Ok(Some(T))` if a query string exists and parsing succeeds,
  /// `Ok(None)` if no query string exists,
  /// or an error if parsing fails
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Deserialize)]
  /// struct Pagination {
  ///   page: u32,
  ///   limit: u32,
  /// }
  ///
  /// async fn list_items(mut req: Request) -> Result {
  ///   if let Some(pagination) = req.query::<Pagination>()? {
  ///     // Use pagination.page and pagination.limit
  ///   }
  ///   Ok("ok".into())
  /// }
  /// ```
  pub fn query<T>(&self) -> Result<Option<T>>
  where
    T: serde::de::DeserializeOwned,
  {
    if let Some(query) = self.uri().query() {
      let result =
        serde_urlencoded::from_str::<T>(query).map_err(crate::error::Error::Urlencoded)?;
      Ok(Some(result))
    } else {
      Ok(None)
    }
  }

  /// Retrieves a path parameter and parses it into type `T`.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `FromStr`
  ///
  /// # Arguments
  ///
  /// * `param` - The name of the parameter to retrieve
  ///
  /// # Returns
  ///
  /// The parsed parameter value, or an error if the parameter is missing or invalid
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// // Route: /users/:id
  /// async fn get_user(mut req: Request) -> Result {
  ///   let id: i32 = req.param("id")?;
  ///   Ok(format!("User {}", id).into())
  /// }
  /// ```
  pub fn param<T>(&self, param: &str) -> Result<T>
  where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::error::Error,
  {
    match self.params.find(param) {
      Some(param) => param
        .parse()
        .map_err(|e| invalid_param(param, std::any::type_name::<T>(), e)),
      None => Err(missing_param(param)),
    }
  }

  /// Retrieves a path parameter as a `String` without requiring a type annotation.
  ///
  /// This is a convenience method for the common case of extracting string parameters.
  ///
  /// # Arguments
  ///
  /// * `name` - The name of the parameter to retrieve
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// // Route: /files/:filename
  /// async fn get_file(req: Request) -> Result {
  ///   let filename = req.param_str("filename")?;
  ///   // No ::<String> turbofish needed
  ///   Ok(format!("File: {}", filename).into())
  /// }
  /// ```
  pub fn param_str(&self, name: &str) -> Result<String> {
    self.param::<String>(name)
  }

  /// Parses the query string into type `T`, returning `T::default()` when
  /// no query string is present.
  ///
  /// Unlike [`Request::query`], this method never returns `None`.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::DeserializeOwned` and `Default`
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Deserialize, Default)]
  /// struct Pagination {
  ///   page: u32,  // defaults to 0
  ///   limit: u32, // defaults to 0
  /// }
  ///
  /// async fn list_items(req: Request) -> Result {
  ///   let pagination = req.query_or_default::<Pagination>()?;
  ///   // Always has a value — no Option handling needed
  /// }
  /// ```
  pub fn query_or_default<T>(&self) -> Result<T>
  where
    T: serde::de::DeserializeOwned + Default,
  {
    if let Some(query) = self.uri().query() {
      Ok(serde_urlencoded::from_str::<T>(query)?)
    } else {
      Ok(T::default())
    }
  }

  /// Deserializes the request body as JSON into type `T`.
  ///
  /// This is an alias for [`Request::body`] with a more descriptive name.
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::DeserializeOwned`
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Deserialize)]
  /// struct Login {
  ///   username: String,
  ///   password: String,
  /// }
  ///
  /// async fn login(mut req: Request) -> Result {
  ///   let credentials: Login = req.body_json().await?;
  ///   Ok("Logged in".into())
  /// }
  /// ```
  pub async fn body_json<T>(&mut self) -> Result<T>
  where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
  {
    self.body::<T>().await
  }

  /// Deserializes an `application/x-www-form-urlencoded` request body into type `T`.
  ///
  /// Reads the full body and parses it with `serde_urlencoded`. Intended for
  /// HTML form submissions (`<form method="post">` without `enctype`).
  ///
  /// # Type Parameters
  ///
  /// * `T` - A type that implements `serde::DeserializeOwned`
  ///
  /// # Errors
  ///
  /// Returns an error if the body cannot be read or does not deserialize
  /// into `T`. The Content-Type header is not strictly validated; the body
  /// is always parsed as urlencoded.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// #[derive(serde::Deserialize)]
  /// struct Login {
  ///   username: String,
  ///   password: String,
  /// }
  ///
  /// async fn login(mut req: Request) -> Result {
  ///   let form: Login = req.form().await?;
  ///   Ok(format!("Hi {}", form.username).into())
  /// }
  /// ```
  pub async fn form<T>(&mut self) -> Result<T>
  where
    T: serde::de::DeserializeOwned,
  {
    let inner = self.inner();
    let bytes = Self::collect_body_limited(inner).await?;
    let payload = serde_urlencoded::from_bytes(&bytes).map_err(crate::error::Error::Urlencoded)?;
    Ok(payload)
  }

  /// Returns the first value of the given request header.
  ///
  /// # Arguments
  ///
  /// * `name` - The header name (case-insensitive per HTTP semantics)
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let auth = req.header("Authorization");
  /// ```
  pub fn header(&self, name: &str) -> Option<&hyper::header::HeaderValue> {
    self.inner.headers().get(name)
  }

  /// Returns the cookie with the given name from the request's `Cookie` header.
  ///
  /// Names are compared with a zero-allocation scan; only the matching
  /// cookie is parsed, so the cost does not grow with the number of cookies
  /// the client sends.
  ///
  /// # Arguments
  ///
  /// * `name` - The cookie name
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// if let Some(theme) = req.cookie("theme") {
  ///   println!("theme = {}", theme.value());
  /// }
  /// ```
  pub fn cookie(&self, name: &str) -> Option<cookie::Cookie<'static>> {
    let header = self.inner.headers().get(hyper::header::COOKIE)?;
    cookie_from_header(header, name)
  }

  /// Returns the resolved client IP.
  ///
  /// When the server was configured with
  /// [`Server::trusted_proxies`](crate::Server::trusted_proxies) and the
  /// request came through a trusted proxy carrying `X-Forwarded-For`, this
  /// is the first non-trusted address of the chain. Otherwise it is the
  /// immediate peer address.
  pub fn client_ip(&self) -> Option<std::net::IpAddr> {
    if let Some(client) = self.inner.extensions().get::<ClientIp>() {
      return Some(client.0);
    }
    Some(self.remote_addr.as_deref()?.ip())
  }

  /// Returns the shared application state set via
  /// [`Router::with_state`](crate::Router::with_state).
  ///
  /// # Type Parameters
  ///
  /// * `T` - The concrete state type (must match the type passed to
  ///   `with_state`)
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let db = req.state::<DbPool>()?;
  /// db.query(...).await;
  /// ```
  pub fn state<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
    let state = self
      .inner
      .extensions()
      .get::<Arc<dyn Any + Send + Sync>>()?;
    Arc::clone(state).downcast::<T>().ok()
  }

  /// Returns the request's session, loaded by the
  /// [`SessionLayer`](crate::SessionLayer) middleware.
  ///
  /// Dereferences to [`Mutex`](std::sync::Mutex)`<Session>`; modifications
  /// are persisted via `Set-Cookie` automatically when the response returns
  /// — and only when the session was actually modified.
  ///
  /// When [`SessionLayer`](crate::SessionLayer) is not installed, returns a
  /// detached session whose changes are discarded (a debug-level log notes
  /// it).
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// req.session().lock().unwrap().insert("user_id", 42)?;
  /// let uid: Option<i32> = req.session().lock().unwrap().get("user_id")?;
  /// ```
  pub fn session(&self) -> crate::middleware::session::SessionHandle {
    use crate::middleware::session::SessionHandle;
    use crate::session::{Session, SessionData};

    if let Some(handle) = self.inner.extensions().get::<SessionHandle>() {
      handle.clone()
    } else {
      debug!("SessionLayer is not installed; session changes will be discarded");
      SessionHandle::new(Session::new(SessionData::new()))
    }
  }
}

impl From<HyperRequest> for Request {
  fn from(request: HyperRequest) -> Self {
    Request::new(request, None)
  }
}

/// Finds `name` in a raw `Cookie` header value and parses only the matching
/// cookie. Name comparison is a zero-allocation scan, so cost does not grow
/// with the number of cookies the client sends.
fn cookie_from_header(
  header: &hyper::header::HeaderValue,
  name: &str,
) -> Option<cookie::Cookie<'static>> {
  let header = header.to_str().ok()?;
  header
    .split(';')
    .filter(|part| {
      let part = part.trim();
      match part.split_once('=') {
        Some((part_name, _)) => part_name.trim() == name,
        // A bare token (no `=`) is its own name.
        None => part == name,
      }
    })
    .find_map(|part| cookie::Cookie::parse(part.trim()).ok())
    .map(cookie::Cookie::into_owned)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn cookie_header(value: &'static str) -> hyper::header::HeaderValue {
    hyper::header::HeaderValue::from_static(value)
  }

  #[test]
  fn test_cookie_finds_match_among_many() {
    let header = cookie_header("theme=dark; session=abc123; consent=1");
    let found = cookie_from_header(&header, "session").unwrap();
    assert_eq!(found.name(), "session");
    assert_eq!(found.value(), "abc123");
  }

  #[test]
  fn test_cookie_value_may_contain_equals() {
    // Base64 padding in the value must not be truncated at the first `=`.
    let header = cookie_header("session=abc=; theme=dark");
    let found = cookie_from_header(&header, "session").unwrap();
    assert_eq!(found.value(), "abc=");
  }

  #[test]
  fn test_cookie_name_is_not_prefix_matched() {
    let header = cookie_header("sessionx=1; session=2");
    assert_eq!(cookie_from_header(&header, "session").unwrap().value(), "2");
    assert_eq!(
      cookie_from_header(&header, "sessionx").unwrap().value(),
      "1"
    );
  }

  #[test]
  fn test_cookie_missing_returns_none() {
    let header = cookie_header("theme=dark; consent=1");
    assert!(cookie_from_header(&header, "session").is_none());
    assert!(cookie_from_header(&cookie_header(""), "session").is_none());
  }

  #[test]
  fn test_cookie_whitespace_tolerant() {
    // Semicolon-separated parts are trimmed before the name scan. The
    // matched part is handed to the cookie crate unchanged, so its own
    // whitespace handling (spaces around `=` are tolerated) is preserved.
    let header = cookie_header("theme=dark;\t session=spaced ");
    let found = cookie_from_header(&header, "session").unwrap();
    assert_eq!(found.value(), "spaced");
    let spaced = cookie_header("session = spaced");
    assert_eq!(
      cookie_from_header(&spaced, "session").unwrap().value(),
      "spaced"
    );
  }

  #[test]
  fn test_cookie_unparseable_parts_are_skipped() {
    // Parts whose name does not match are never parsed; the scan continues
    // past them (here, a name with a space that some clients send).
    let header = cookie_header("a b=c; session=ok");
    assert_eq!(
      cookie_from_header(&header, "session").unwrap().value(),
      "ok"
    );
  }

  #[test]
  fn test_cookie_duplicate_names_first_wins() {
    let header = cookie_header("session=first; session=second");
    assert_eq!(
      cookie_from_header(&header, "session").unwrap().value(),
      "first"
    );
  }
}
