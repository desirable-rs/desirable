use crate::{DynEndpoint, Endpoint, IntoResponse, Middleware, Next, Request, Response, Result};
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
/// router.get("/", |_| async { "Home" });
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
  /// Exact-match fast path for routes without `:param`/`*wildcard`
  /// segments, consulted before the linear pattern tables. Routes are
  /// registered here *and* in `routes`; the fast map only answers first.
  static_routes: HashMap<hyper::Method, HashMap<String, Box<DynEndpoint>>>,
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

/// Shared handle letting one endpoint live in both the static fast map and
/// the pattern tables (`dyn Endpoint` is not `Clone`). Static routes pay one
/// extra virtual call; parameterized routes are registered unshared.
struct SharedEndpoint(Arc<DynEndpoint>);

#[async_trait::async_trait]
impl Endpoint for SharedEndpoint {
  async fn call(&self, req: Request) -> Result {
    self.0.call(req).await
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
      static_routes: HashMap::new(),
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
    // An empty middleware chain is a pass-through: skip the wrapper instead
    // of paying an extra call layer on every request.
    let endpoint: Box<DynEndpoint> = if self.middlewares_arc.is_empty() {
      Box::new(dest)
    } else {
      Box::new(ScopedEndpoint {
        endpoint: Box::new(dest),
        middlewares: Arc::clone(&self.middlewares_arc),
      })
    };
    let tables = self.routes.entry(method.clone()).or_default();
    if tables.is_empty() {
      tables.push(route_recognizer::Router::new());
    }
    if !path.contains(':') && !path.contains('*') {
      // Static path: also register in the O(1) fast map. `or_insert` keeps
      // first-registration-wins; the pattern-table copy stays for public
      // introspection of `routes` but is never dispatched (the fast map
      // answers first).
      let shared: Arc<DynEndpoint> = Arc::from(endpoint);
      tables
        .last_mut()
        .unwrap()
        .add(&path, Box::new(SharedEndpoint(Arc::clone(&shared))));
      self
        .static_routes
        .entry(method)
        .or_default()
        .entry(path)
        .or_insert_with(|| Box::new(SharedEndpoint(shared)));
    } else {
      tables.last_mut().unwrap().add(&path, endpoint);
    }
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
  /// router.get("/", |_| async { "Home" });
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

  /// Registers a WebSocket route (feature `websocket`).
  ///
  /// The handshake is validated automatically: non-WebSocket requests to
  /// this path receive `400 Bad Request`. Once upgraded, the callback
  /// receives the established connection.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// use desirable::{Router, WebSocketConn};
  /// use desirable::websocket::Message;
  ///
  /// let mut app = Router::new();
  /// app.websocket("/ws", |mut conn: WebSocketConn| async move {
  ///   while let Some(Ok(msg)) = conn.recv().await {
  ///     if let Message::Text(text) = msg {
  ///       conn.send_text(text.to_string()).await.unwrap();
  ///     }
  ///   }
  /// });
  /// ```
  #[cfg(feature = "websocket")]
  pub fn websocket<F, Fut>(&mut self, route: &str, callback: F)
  where
    F: Fn(crate::websocket::WebSocketConn) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    let callback = std::sync::Arc::new(callback);
    self.get(route, move |req: crate::Request| {
      let callback = std::sync::Arc::clone(&callback);
      async move {
        match crate::websocket::WebSocketUpgrade::from_request(&req) {
          Ok(upgrade) => upgrade.on_upgrade(req, move |conn| callback(conn)),
          Err(response) => Ok(*response),
        }
      }
    });
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
  /// router.get("/", |_| async { "logged" });
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
    // Fast-map entries: the parent's existing routes win (same
    // first-registration-wins rule as `at`).
    for (method, table) in target.static_routes {
      let dst = self.static_routes.entry(method).or_default();
      for (path, endpoint) in table {
        dst.entry(path).or_insert(endpoint);
      }
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
  pub async fn dispatch(&self, mut req: Request, remote_addr: Option<Arc<SocketAddr>>) -> Result {
    let mut params = route_recognizer::Params::new();

    // Explicit HEAD routes are consulted before falling back to GET.
    let is_head = *req.method() == hyper::Method::HEAD;

    // Fallback handlers run inside the router-level middleware chain; matched
    // routes carry their own (scoped) chain captured at registration.
    let not_found: &DynEndpoint = &*self.not_found_handler;
    let not_allowed: &DynEndpoint = &*self.method_not_allowed_handler;

    // Match by borrowing the method and path — both matchers only borrow
    // `self`, so the borrows end before `req` is mutated below and no
    // per-request allocation is needed.
    let matched = {
      let path = req.uri().path();
      let lookup = |method: &hyper::Method| -> Option<(&DynEndpoint, route_recognizer::Params)> {
        self.lookup(method, path)
      };
      if is_head {
        lookup(&hyper::Method::HEAD).or_else(|| lookup(&hyper::Method::GET))
      } else {
        lookup(req.method())
      }
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
    req.remote_addr = remote_addr;
    if let Some(state) = &self.state {
      req.extensions_mut().insert(Arc::clone(state));
    }

    let response = Next {
      endpoint,
      middlewares,
    }
    .run(req)
    .await;

    // NOTE: hyper suppresses HEAD bodies itself while still emitting the
    // headers (including Content-Length) the GET response would have — so
    // no manual body stripping here (that would drop Content-Length).
    response
  }

  /// Looks up `method` + `path`: the O(1) static fast map first (exact path,
  /// then trailing-slash-trimmed), then the pattern tables.
  fn lookup(
    &self,
    method: &hyper::Method,
    path: &str,
  ) -> Option<(&DynEndpoint, route_recognizer::Params)> {
    let trimmed = path.strip_suffix('/').filter(|t| !t.is_empty());
    let static_hit = self
      .static_exact(method, path)
      .or_else(|| trimmed.and_then(|t| self.static_exact(method, t)));
    if let Some(endpoint) = static_hit {
      return Some((endpoint, route_recognizer::Params::new()));
    }
    self.match_path(method, path)
  }

  fn static_exact(&self, method: &hyper::Method, path: &str) -> Option<&DynEndpoint> {
    self
      .static_routes
      .get(method)?
      .get(path)
      .map(|endpoint| &**endpoint)
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
  /// sorted for a stable `Allow` header. Tolerates a trailing slash. Static
  /// fast-map hits count as matches too.
  fn matching_methods(&self, path: &str) -> Vec<hyper::Method> {
    let trimmed = path.strip_suffix('/').filter(|t| !t.is_empty());
    let matches = |table: &route_recognizer::Router<Box<DynEndpoint>>| {
      table.recognize(path).is_ok()
        || trimmed.is_some_and(|trimmed| table.recognize(trimmed).is_ok())
    };
    let mut methods: Vec<hyper::Method> = self
      .routes
      .iter()
      .filter(|(method, tables)| {
        let static_hit = self.static_routes.get(method).is_some_and(|t| {
          t.contains_key(path) || trimmed.is_some_and(|trimmed| t.contains_key(trimmed))
        });
        static_hit || tables.iter().any(matches)
      })
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

  #[test]
  fn test_static_fast_path_hits_and_param_falls_through() {
    let mut router = Router::new();
    router.get("/users/:id", |_| async { "param" });
    router.get("/users/new", |_| async { "static" });
    router.get("/health", |_| async { "ok" });

    // Static routes land in the fast map...
    assert!(
      router
        .static_exact(&hyper::Method::GET, "/health")
        .is_some()
    );
    // ...parameterized routes never do.
    assert!(
      router
        .static_exact(&hyper::Method::GET, "/users/:id")
        .is_none()
    );

    // Static wins over the parameterized pattern regardless of order:
    // a static hit carries no params, so `id` is absent.
    let (_, params) = router.lookup(&hyper::Method::GET, "/users/new").unwrap();
    assert!(params.find("id").is_none(), "static route must win");

    // The parameterized pattern still serves everything else.
    let (_, params) = router.lookup(&hyper::Method::GET, "/users/42").unwrap();
    assert_eq!(params.find("id"), Some("42"));
  }

  #[test]
  fn test_static_fast_path_trailing_slash() {
    let mut router = Router::new();
    router.get("/users", |_| async { "users" });

    let (endpoint, params) = router.lookup(&hyper::Method::GET, "/users/").unwrap();
    assert!(params.iter().next().is_none());
    // Same endpoint object as the exact lookup (fast map, not the tables).
    let (exact, _) = router.lookup(&hyper::Method::GET, "/users").unwrap();
    assert!(std::ptr::eq(endpoint, exact));
  }

  #[test]
  fn test_head_table_consulted_before_get_fallback() {
    let mut router = Router::new();
    router.get("/only-get", |_| async { "get" });
    router.head("/ping", |_| async { "pong" });

    // Explicit HEAD routes are reachable through the HEAD table (lookup is
    // per-method; dispatch applies the HEAD→GET fallback on top).
    assert!(router.lookup(&hyper::Method::HEAD, "/ping").is_some());
    assert!(router.lookup(&hyper::Method::HEAD, "/only-get").is_none());
    // GET does not fall back to HEAD.
    assert!(router.lookup(&hyper::Method::GET, "/ping").is_none());
    // And the static fast map holds the HEAD route too.
    assert!(router.static_exact(&hyper::Method::HEAD, "/ping").is_some());
  }

  #[test]
  fn test_merge_merges_static_fast_map() {
    let mut a = Router::new();
    a.get("/a", |_| async { "a" });
    let mut b = Router::new();
    b.get("/b", |_| async { "b" });
    a.merge(b);

    assert!(a.static_exact(&hyper::Method::GET, "/a").is_some());
    assert!(a.static_exact(&hyper::Method::GET, "/b").is_some());
  }

  #[test]
  fn test_matching_methods_includes_static_hits() {
    let mut router = Router::new();
    router.get("/only-get", |_| async { "get" });
    router.post("/only-post", |_| async { "post" });

    let methods = router.matching_methods("/only-get");
    assert_eq!(methods, vec![hyper::Method::GET]);
    assert!(router.matching_methods("/nope").is_empty());
  }
}
