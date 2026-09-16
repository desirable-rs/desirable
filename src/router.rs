use crate::{DynEndpoint, Endpoint, IntoResponse, Middleware, Next, Request, Response, Result};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

/// The HTTP request router.
///
/// Routes incoming requests to handlers based on HTTP method and path.
/// Supports path parameters, middleware, and nested routers.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Router, Result};
///
/// let mut router = Router::new();
/// router.get("/", || async { "Home" });
/// router.get("/users/:id", |req| async {
///   let id: i32 = req.param("id").unwrap();
///   format!("User {}", id)
/// });
/// router.with(Logger); // Add middleware
/// ```
pub struct Router {
  /// Optional path prefix for all routes in this router
  pub prefix: Option<String>,
  /// Middleware stack applied to all routes
  pub middlewares: Vec<Arc<dyn Middleware>>,
  /// Route mappings keyed by HTTP method
  pub routes: HashMap<hyper::Method, route_recognizer::Router<Box<DynEndpoint>>>,
  /// Handler for unmatched routes
  pub not_found_handler: Box<DynEndpoint>,
  /// Handler for paths that exist under other methods
  pub method_not_allowed_handler: Box<DynEndpoint>,
}

/// Extension carrying the HTTP methods that can serve the matched path.
///
/// Inserted by [`Router::dispatch`] before the method-not-allowed handler runs.
#[derive(Debug, Clone)]
pub struct AllowedMethods(pub Vec<hyper::Method>);

async fn default_handler(_req: Request) -> impl IntoResponse {
  (hyper::StatusCode::NOT_FOUND, "not found")
}

async fn default_method_not_allowed_handler(req: Request) -> Response {
  let allow = req
    .extensions()
    .get::<AllowedMethods>()
    .map(|a| a.0.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", "))
    .unwrap_or_default();
  let mut response = Response::builder()
    .status(hyper::StatusCode::METHOD_NOT_ALLOWED)
    .text("method not allowed")
    .expect("static 405 response cannot fail to build");
  if !allow.is_empty() {
    response.set_header(
      hyper::header::ALLOW,
      hyper::header::HeaderValue::from_str(&allow).expect("method names are valid header values"),
    );
  }
  response
}

impl Default for Router {
  fn default() -> Self {
    Self::new()
  }
}

impl Router {
  /// Creates a new empty router.
  ///
  /// # Returns
  ///
  /// A new router with no routes or middleware
  pub fn new() -> Self {
    Router {
      prefix: None,
      middlewares: Vec::new(),
      routes: HashMap::new(),
      not_found_handler: Box::new(default_handler),
      method_not_allowed_handler: Box::new(default_method_not_allowed_handler),
    }
  }

  /// Adds a route for the specified HTTP method.
  ///
  /// # Arguments
  ///
  /// * `method` - The HTTP method (GET, POST, etc.)
  /// * `route` - The route path, optionally with parameters (e.g., `/users/:id`)
  /// * `dest` - The endpoint handler
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// router.at(hyper::Method::GET, "/test", |_| async { "test" });
  /// ```
  pub fn at(&mut self, method: hyper::Method, route: &str, dest: impl Endpoint) {
    let path = match &self.prefix {
      Some(prefix) => format!("{}{}", prefix, route),
      None => route.to_string(),
    };
    self
      .routes
      .entry(method)
      .or_default()
      .add(&path, Box::new(dest));
  }

  /// Adds a GET route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// router.get("/", || async { "Home" });
  /// router.get("/users/:id", |req| async {
  ///   format!("User {}", req.param::<i32>("id").unwrap())
  /// });
  /// ```
  pub fn get(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::GET, route, dest);
  }

  /// Adds a POST route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn post(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::POST, route, dest);
  }

  /// Adds a DELETE route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn delete(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::DELETE, route, dest);
  }

  /// Adds a PATCH route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn patch(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::PATCH, route, dest);
  }

  /// Adds a PUT route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn put(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::PUT, route, dest);
  }

  /// Adds an OPTIONS route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn options(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::OPTIONS, route, dest);
  }

  /// Adds a HEAD route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn head(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::HEAD, route, dest);
  }

  /// Adds a TRACE route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn trace(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::TRACE, route, dest);
  }

  /// Adds a CONNECT route.
  ///
  /// # Arguments
  ///
  /// * `route` - The route path
  /// * `dest` - The endpoint handler
  pub fn connect(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::CONNECT, route, dest);
  }

  /// Adds middleware to the router.
  ///
  /// Middleware is executed before the route handler for all routes in this router.
  ///
  /// # Arguments
  ///
  /// * `middleware` - The middleware to add
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// struct Logger;
  ///
  /// #[async_trait::async_trait]
  /// impl Middleware for Logger {
  ///   async fn handle(&self, req: Request, next: Next<'_>) -> Result {
  ///     println!("Request: {}", req.path());
  ///     next.run(req).await
  ///   }
  /// }
  ///
  /// router.with(Logger);
  /// ```
  pub fn with(&mut self, middleware: impl Middleware) {
    self.middlewares.push(Arc::new(middleware));
  }

  /// Merges another router's routes into this router.
  ///
  /// The routes from the target router are added to this router's routes.
  /// Middleware is not merged.
  ///
  /// # Arguments
  ///
  /// * `target` - The router whose routes to merge
  pub fn merge(&mut self, target: Router) {
    self.routes.extend(target.routes);
  }

  /// Dispatches a request to the appropriate handler.
  ///
  /// This method is called internally by the server to route requests.
  ///
  /// # Arguments
  ///
  /// * `req` - The incoming request
  /// * `remote_addr` - The client's socket address
  ///
  /// # Returns
  ///
  /// The response from the matched handler or an error
  pub async fn dispatch(&self, mut req: Request, remote_addr: Arc<SocketAddr>) -> Result {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let mut params = route_recognizer::Params::new();

    let matched = self
      .routes
      .get(&method)
      .and_then(|route| route.recognize(&path).ok());

    let endpoint: &DynEndpoint = if let Some(m) = matched {
      m.params().clone_into(&mut params);
      &***m.handler()
    } else {
      // No route for this method. If the path exists under other methods,
      // respond 405 Method Not Allowed; otherwise fall back to 404.
      let methods = self.matching_methods(&path);
      if methods.is_empty() {
        &*self.not_found_handler
      } else {
        req.extensions_mut().insert(AllowedMethods(methods));
        &*self.method_not_allowed_handler
      }
    };

    req.params = params;
    req.remote_addr = Some(remote_addr);
    let next = Next {
      endpoint,
      middlewares: &self.middlewares,
    };
    next.run(req).await
  }

  /// Returns the HTTP methods whose route tables contain a match for `path`,
  /// sorted for a stable `Allow` header.
  fn matching_methods(&self, path: &str) -> Vec<hyper::Method> {
    let mut methods: Vec<hyper::Method> = self
      .routes
      .iter()
      .filter(|(_, route)| route.recognize(path).is_ok())
      .map(|(method, _)| method.clone())
      .collect();
    methods.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    methods
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // Note: Full Request creation requires Incoming body type which is not publicly constructible.
  // These tests verify the API structure without creating full requests.

  #[test]
  fn test_router_new() {
    let router = Router::new();
    assert!(router.prefix.is_none());
    assert!(router.middlewares.is_empty());
    assert!(router.routes.is_empty());
  }

  #[test]
  fn test_router_get() {
    let mut router = Router::new();
    router.get("/", |_| async { "home" });
    assert!(router.routes.contains_key(&hyper::Method::GET));
  }

  #[test]
  fn test_router_post() {
    let mut router = Router::new();
    router.post("/api/data", |_| async { "created" });
    assert!(router.routes.contains_key(&hyper::Method::POST));
  }

  #[test]
  fn test_router_all_methods() {
    let mut router = Router::new();
    router.get("/g", |_| async { "get" });
    router.post("/p", |_| async { "post" });
    router.delete("/d", |_| async { "delete" });
    router.patch("/pa", |_| async { "patch" });
    router.put("/pu", |_| async { "put" });
    router.options("/o", |_| async { "options" });
    router.head("/h", |_| async { "head" });
    router.trace("/t", |_| async { "trace" });
    router.connect("/c", |_| async { "connect" });

    assert_eq!(router.routes.len(), 9);
  }

  #[test]
  fn test_router_with_middleware() {
    use crate::{Middleware, Next, Request, Result};

    struct TestMiddleware;

    #[async_trait::async_trait]
    impl Middleware for TestMiddleware {
      async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        next.run(req).await
      }
    }

    let mut router = Router::new();
    router.get("/", |_| async { "test" });
    router.with(TestMiddleware);

    assert_eq!(router.middlewares.len(), 1);
  }

  #[test]
  fn test_router_merge() {
    let mut router1 = Router::new();
    router1.get("/a", |_| async { "a" });

    let mut router2 = Router::new();
    router2.get("/b", |_| async { "b" });

    router1.merge(router2);

    // Both routes should be accessible
    assert!(router1.routes.contains_key(&hyper::Method::GET));
  }
}
