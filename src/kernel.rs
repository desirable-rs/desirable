use crate::IntoResponse;
use crate::{Request, Result};
use std::future::Future;
use std::sync::Arc;

/// Trait for HTTP request handlers.
///
/// An endpoint is the core handler that processes incoming requests and returns responses.
/// Any function or type that implements this trait can be used as a route handler.
///
/// # Implementors
///
/// Functions that return an `async` block implementing `IntoResponse` are automatically
/// endpoints:
///
/// ```rust,ignore
/// async fn hello() -> &'static str {
///   "Hello, World!"
/// }
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Endpoint, Request, Result};
///
/// #[async_trait::async_trait]
/// impl Endpoint for MyHandler {
///   async fn call(&self, req: Request) -> Result {
///     // Handle the request
///     Ok("Hello!".into())
///   }
/// }
/// ```
#[async_trait::async_trait]
pub trait Endpoint: Send + Sync + 'static {
  /// Calls the endpoint with the given request.
  ///
  /// # Arguments
  ///
  /// * `req` - The incoming HTTP request
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an error
  async fn call(&self, req: Request) -> Result;
}

/// Dynamic endpoint type for trait objects.
pub type DynEndpoint = dyn Endpoint;

#[async_trait::async_trait]
impl<F, Fut, Res> Endpoint for F
where
  F: Send + Sync + 'static + Fn(Request) -> Fut,
  Fut: Future<Output = Res> + Send + 'static,
  Res: IntoResponse + 'static,
{
  async fn call(&self, req: Request) -> Result {
    let fut = (self)(req);
    let res = fut.await;
    res.into_response()
  }
}

/// Internal structure for middleware chain execution.
///
/// `Next` holds the remaining middleware stack and final endpoint,
/// allowing each middleware to decide whether and when to call the next handler.
pub struct Next<'a> {
  /// The final endpoint to call if all middleware pass through
  pub endpoint: &'a DynEndpoint,
  /// Remaining middleware to execute
  pub middlewares: &'a [Arc<dyn Middleware>],
}

impl Next<'_> {
  /// Executes the middleware chain, calling each middleware in sequence.
  ///
  /// # Arguments
  ///
  /// * `req` - The incoming request
  ///
  /// # Returns
  ///
  /// The response from the last executed middleware or endpoint
  ///
  /// # Behavior
  ///
  /// - If middleware remain: calls the first middleware with `self` as the next handler
  /// - If no middleware remain: calls the final endpoint directly
  /// - If a middleware returns an error, execution stops and error is returned
  pub async fn run(mut self, req: Request) -> Result {
    if let Some((cur, next)) = self.middlewares.split_first() {
      self.middlewares = next;
      match cur.handle(req, self).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
      }
    } else {
      self.endpoint.call(req).await
    }
  }
}

/// Trait for HTTP middleware.
///
/// Middleware can inspect, modify, or short-circuit incoming requests.
/// They receive the request and a `Next` handler that they can optionally call.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Middleware, Next, Request, Result};
///
/// struct Logger;
///
/// #[async_trait::async_trait]
/// impl Middleware for Logger {
///   async fn handle(&self, req: Request, next: Next<'_>) -> Result {
///     println!("Request: {} {}", req.method(), req.path());
///     next.run(req).await
///   }
/// }
/// ```
#[async_trait::async_trait]
pub trait Middleware: Send + Sync + 'static {
  /// Handles the request by optionally calling the next handler.
  ///
  /// # Arguments
  ///
  /// * `req` - The incoming request
  /// * `next` - The next handler in the chain
  ///
  /// # Returns
  ///
  /// A `Result` containing the response or an error
  async fn handle(&self, req: Request, next: Next<'_>) -> Result;

  /// Returns the name of this middleware for debugging purposes.
  ///
  /// Default implementation returns the type name.
  fn name(&self) -> &str {
    std::any::type_name::<Self>()
  }
}

#[async_trait::async_trait]
impl<F, Fut, Res> Middleware for F
where
  F: Send + Sync + 'static + Fn(Request, Next) -> Fut,
  Fut: Future<Output = Res> + Send + 'static,
  Res: IntoResponse + 'static,
{
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let fut = (self)(req, next);
    let res = fut.await;
    res.into_response()
  }
}
