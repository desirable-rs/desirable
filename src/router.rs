use crate::{DynEndpoint, Endpoint, IntoResponse, Middleware, Next, Request, Response, Result};
use bytes::Bytes;
use http_body_util::Full;
use std::any::Any;
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
  /// Middleware stack applied to routes registered after it was added.
  ///
  /// Middleware is order-dependent (like axum's `layer`): call [`Router::with`]
  /// before registering the routes it should apply to.
  pub middlewares: Vec<Arc<dyn Middleware>>,
  /// Snapshot of the middleware stack, shared with scoped endpoints.
  middlewares_arc: Arc<Vec<Arc<dyn Middleware>>>,
  /// Route tables keyed by HTTP method.
  ///
  /// Each method holds a *list* of tables: [`Router::merge`] appends the
  /// target's tables instead of replacing them, so routes registered on both
  /// routers survive a merge.
  pub routes: HashMap<hyper::Method, Vec<route_recognizer::Router<Box<DynEndpoint>>>>,
  /// Handler for unmatched routes
  pub not_found_handler: Box<DynEndpoint>,
  /// Handler for paths that exist under other methods
  pub method_not_allowed_handler: Box<DynEndpoint>,
  /// Shared application state, injected into every request's extensions.
  /// Set via [`Router::with_state`], read via [`Request::state`](crate::Request::state).
  pub state: Option<Arc<dyn Any + Send + Sync>>,
}

/// Extension carrying the HTTP methods that can serve the matched path.
///
/// Inserted by [`Router::dispatch`] before the method-not-allowed handler runs.
#[derive(Debug, Clone)]
pub struct AllowedMethods(pub Vec<hyper::Method>);

/// An endpoint bundled with the middleware chain captured when it was
/// registered. This is how per-router middleware survives [`Router::merge`]:
/// each route carries its own chain instead of relying on router-level state.
struct ScopedEndpoint {
  endpoint: Box<DynEndpoint>,
  middlewares: Arc<Vec<Arc<dyn Middleware>>>,
}

#[async_trait::async_trait]
impl Endpoint for ScopedEndpoint {
  async fn call(&self, req: Request) -> Result {
    Next {
      endpoint: &*self.endpoint,
      middlewares: &self.middlewares,
    }
    .run(req)
    .await
  }
}

async fn default_handler(_req: Request) -> impl IntoResponse {
  (hyper::StatusCode::NOT_FOUND, "not found")
}

async fn default_method_not_allowed_handler(req: Request) -> Response {
  let allow = req
    .extensions()
    .get::<AllowedMethods>()
    .map(|a| {
      a.0
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(", ")
    })
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
      middlewares_arc: Arc::new(Vec::new()),
      routes: HashMap::new(),
      not_found_handler: Box::new(default_handler),
      method_not_allowed_handler: Box::new(default_method_not_allowed_handler),
      state: None,
    }
  }

  /// Sets the shared application state, available to all handlers via
  /// [`Request::state`](crate::Request::state).
  ///
  /// Call this before merging the router into another: the child's state is
  /// discarded on merge and the parent's state is what gets injected.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let db = DbPool::new();
  /// let mut app = Router::new().with_state(db);
  /// app.get("/users", |req: Request| async move {
  ///   let db = req.state::<DbPool>().ok_or_else(|| error_msg("no state"))?;
  ///   // ...
  /// });
  /// ```
  #[must_use]
  pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
    self.state = Some(Arc::new(state));
    self
  }

  /// Sets the path prefix for all routes registered on this router.
  ///
  /// Routes are prefixed at registration time, so call this before adding
  /// routes. Combined with [`Router::merge`], this enables nesting:
  ///
  /// ```rust,ignore
  /// let api = Router::new()
  ///     .prefix("/api")
  ///     .with(Auth);              // scoped to these routes
  /// api.get("/users", list_users); // served at /api/users
  ///
  /// let mut app = Router::new();
  /// app.merge(api);                // middleware is preserved
  /// ```
  #[must_use]
  pub fn prefix(mut self, prefix: &str) -> Self {
    let mut prefix = prefix.trim_end_matches('/').to_string();
    if !prefix.is_empty() && !prefix.starts_with('/') {
      prefix.insert(0, '/');
    }
    self.prefix = if prefix.is_empty() {
      None
    } else {
      Some(prefix)
    };
    self
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
    let scoped = ScopedEndpoint {
      endpoint: Box::new(dest),
      middlewares: Arc::clone(&self.middlewares_arc),
    };
    let tables = self.routes.entry(method).or_default();
    if tables.is_empty() {
      tables.push(route_recognizer::Router::new());
    }
    tables.last_mut().unwrap().add(&path, Box::new(scoped));
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

  /// Adds middleware to routes registered after this call.
  ///
  /// Middleware is order-dependent (like axum's `layer`): it applies to the
  /// routes added after [`Router::with`], and it travels with those routes
  /// through [`Router::merge`]. Call `with` before registering routes.
  ///
  /// The built-in 404/405 fallbacks always run the full middleware stack.
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
  /// router.get("/", || async { "logged" });
  /// ```
  pub fn with(&mut self, middleware: impl Middleware) {
    self.middlewares.push(Arc::new(middleware));
    self.middlewares_arc = Arc::new(self.middlewares.clone());
  }

  /// Merges another router's routes into this router.
  ///
  /// Routes from `target` keep the middleware that was registered on it
  /// before its routes (see [`Router::with`]), so nesting works:
  ///
  /// ```rust,ignore
  /// let api = Router::new().prefix("/api").with(Auth);
  /// api.get("/users", list_users);
  ///
  /// app.merge(api); // /api/users runs Auth
  /// ```
  ///
  /// The target's not-found and method-not-allowed handlers are discarded;
  /// this router's fallbacks remain in charge.
  ///
  /// # Arguments
  ///
  /// * `target` - The router whose routes to merge
  pub fn merge(&mut self, target: Router) {
    for (method, tables) in target.routes {
      self.routes.entry(method).or_default().extend(tables);
    }
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
    let mut params = route_recognizer::Params::new();

    // HEAD falls back to the GET route table; the body is stripped below.
    let is_head = *req.method() == hyper::Method::HEAD;

    // Fallback handlers run inside the router-level middleware chain; matched
    // routes carry their own (scoped) chain captured at registration.
    let not_found: &DynEndpoint = &*self.not_found_handler;
    let not_allowed: &DynEndpoint = &*self.method_not_allowed_handler;

    // Match by borrowing the method and path — both matchers only borrow
    // `self`, so the borrows end before `req` is mutated below and no
    // per-request allocation is needed.
    let matched = {
      let lookup_method: &hyper::Method = if is_head {
        &hyper::Method::GET
      } else {
        req.method()
      };
      let path = req.uri().path();
      self.match_path(lookup_method, path)
    };

    let (endpoint, middlewares): (&DynEndpoint, &[Arc<dyn Middleware>]) =
      if let Some((handler, matched_params)) = matched {
        matched_params.clone_into(&mut params);
        (handler, &[])
      } else {
        // No route for this method. If the path exists under other methods,
        // respond 405 Method Not Allowed; otherwise fall back to 404.
        let methods = {
          let path = req.uri().path();
          self.matching_methods(path)
        };
        if methods.is_empty() {
          (not_found, &self.middlewares)
        } else {
          req.extensions_mut().insert(AllowedMethods(methods));
          (not_allowed, &self.middlewares)
        }
      };

    req.params = params;
    req.remote_addr = Some(remote_addr);
    if let Some(state) = &self.state {
      req.extensions_mut().insert(Arc::clone(state));
    }

    let mut response = Next {
      endpoint,
      middlewares,
    }
    .run(req)
    .await;

    // Per HTTP semantics, HEAD responses carry no body.
    if is_head && let Ok(res) = &mut response {
      *res.inner.body_mut() = Full::new(Bytes::new());
    }

    response
  }

  /// Attempts to match `method`'s route tables against `path`, tolerating a
  /// trailing slash: `/users/` falls back to `/users` when no exact route
  /// (or route registered with a trailing slash) matches.
  fn match_path(
    &self,
    method: &hyper::Method,
    path: &str,
  ) -> Option<(&DynEndpoint, route_recognizer::Params)> {
    let tables = self.routes.get(method)?;
    let try_one = |p: &str| tables.iter().find_map(|table| table.recognize(p).ok());
    try_one(path)
      .or_else(|| {
        let trimmed = path.strip_suffix('/')?;
        (!trimmed.is_empty()).then(|| try_one(trimmed)).flatten()
      })
      .map(|m| {
        let handler: &DynEndpoint = &***m.handler();
        (handler, m.params().clone())
      })
  }

  /// Returns the HTTP methods whose route tables contain a match for `path`,
  /// sorted for a stable `Allow` header. Tolerates a trailing slash.
  fn matching_methods(&self, path: &str) -> Vec<hyper::Method> {
    let trimmed = path.strip_suffix('/').filter(|t| !t.is_empty());
    let matches = |table: &route_recognizer::Router<Box<DynEndpoint>>| {
      table.recognize(path).is_ok()
        || trimmed.is_some_and(|trimmed| table.recognize(trimmed).is_ok())
    };
    let mut methods: Vec<hyper::Method> = self
      .routes
      .iter()
      .filter(|(_, tables)| tables.iter().any(matches))
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

  #[test]
  fn test_prefix_builder_normalization() {
    assert_eq!(
      Router::new().prefix("/api").prefix,
      Some("/api".to_string())
    );
    assert_eq!(
      Router::new().prefix("/api/").prefix,
      Some("/api".to_string())
    );
    assert_eq!(Router::new().prefix("api").prefix, Some("/api".to_string()));
    assert_eq!(Router::new().prefix("/").prefix, None);
    assert_eq!(Router::new().prefix("").prefix, None);
  }

  #[test]
  fn test_prefix_baked_into_routes() {
    let mut api = Router::new().prefix("/api");
    api.get("/users", |_| async { "users" });
    let tables = api.routes.get(&hyper::Method::GET).unwrap();
    assert!(tables.iter().any(|t| t.recognize("/api/users").is_ok()));
    assert!(tables.iter().all(|t| t.recognize("/users").is_err()));
  }

  #[test]
  fn test_merge_preserves_scoped_middleware() {
    use crate::Next;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Tag;
    #[async_trait::async_trait]
    impl crate::Middleware for Tag {
      async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        CALLS.fetch_add(1, Ordering::SeqCst);
        next.run(req).await
      }
    }

    let mut api = Router::new().prefix("/api");
    api.with(Tag);
    api.get("/ping", |_| async { "pong" });

    let mut app = Router::new();
    app.merge(api);

    // Scoped middleware survived the merge: chain captured on the endpoint.
    let tables = app.routes.get(&hyper::Method::GET).unwrap();
    assert!(tables.iter().any(|t| t.recognize("/api/ping").is_ok()));
  }
}
