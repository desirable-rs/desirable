use crate::AnyResult;
use crate::HyperRequest;
use crate::Result;
use crate::error::{invalid_param, missing_param};
use bytes::Buf;
use http_body_util::BodyExt;
use hyper::http::Extensions;
use route_recognizer::Params;
use std::net::SocketAddr;
use std::sync::Arc;

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
  pub async fn body<T>(&mut self) -> AnyResult<T>
  where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
  {
    let inner = self.inner();
    let body = inner.collect().await?.aggregate();
    let payload: T = serde_json::from_reader(body.reader())?;
    Ok(payload)
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
  pub fn query<T>(&self) -> AnyResult<Option<T>>
  where
    T: serde::de::DeserializeOwned,
  {
    if let Some(query) = self.uri().query() {
      let result = serde_urlencoded::from_str::<T>(query)?;
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
  pub fn query_or_default<T>(&self) -> AnyResult<T>
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
  pub async fn body_json<T>(&mut self) -> AnyResult<T>
  where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
  {
    self.body::<T>().await
  }
}

impl From<HyperRequest> for Request {
  fn from(request: HyperRequest) -> Self {
    Request::new(request, None)
  }
}
