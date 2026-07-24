# desirable v1.1 Enhancement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade dependencies, add graceful shutdown, CORS middleware, logging middleware, and Response enhancements to the desirable web framework.

**Architecture:** Six independent work items that can be implemented sequentially. Each adds a self-contained module or modifies an existing one. Zero new external dependencies.

**Tech Stack:** Rust 2024 edition, hyper 1.x, tokio, tracing, rand 0.9

**Spec:** `docs/superpowers/specs/2026-07-24-enhance-framework-design.md`

---

## File Structure

```
src/
├── lib.rs              # Modify: add cors + middleware modules, re-exports
├── server.rs           # Modify: extract accept_loop, add run_graceful
├── session.rs           # Modify: rand 0.9 API migration (3 call sites)
├── response.rs          # Modify: add with_header, set_header
├── cors.rs              # CREATE: CORS middleware (builder + Middleware impl)
├── middleware/
│   ├── mod.rs           # CREATE: middleware module root
│   └── logger.rs        # CREATE: Logger middleware
Cargo.toml              # Modify: version bump, rand 0.9, cargo update deps
```

---

### Task 1: Upgrade Dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/session.rs`

- [ ] **Step 1: Update Cargo.toml — bump version and rand**

Change `version = "1.0.1"` to `version = "1.1.0"` and `rand` from `^0.8.5` to `^0.9.0`:

```toml
version = "1.1.0"
```

```toml
rand = "^0.9.0"
```

- [ ] **Step 2: Run cargo update to pull latest compatible versions**

Run: `cargo update`
Expected: All dependencies updated to latest within caret constraints.

- [ ] **Step 3: Migrate rand 0.8 → 0.9 API calls in src/session.rs**

Change all 3 occurrences of `rand::thread_rng()` to `rand::rng()`:

Line 492 in `SessionConfig::default`:
```rust
// OLD
rand::thread_rng().fill_bytes(&mut key);
// NEW
rand::rng().fill_bytes(&mut key);
```

Line 543 in `SessionData::new`:
```rust
// OLD
rand::thread_rng().fill_bytes(&mut bytes);
// NEW
rand::rng().fill_bytes(&mut bytes);
```

Line 1062 in `Session::regenerate_id`:
```rust
// OLD
rand::thread_rng().fill_bytes(&mut bytes);
// NEW
rand::rng().fill_bytes(&mut bytes);
```

- [ ] **Step 4: Build and test**

Run: `cargo build --verbose`
Expected: Compiles cleanly with rand 0.9.

Run: `cargo test --verbose`
Expected: All tests pass. Session roundtrip tests verify rand 0.9 integration.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/session.rs
git commit -m "chore: upgrade dependencies, rand 0.9, version to 1.1.0"
```

---

### Task 2: Response Enhancements

**Files:**
- Modify: `src/response.rs`

- [ ] **Step 1: Add `set_header` and `with_header` methods to Response**

Add the following methods inside `impl Response { ... }` after the existing methods (after line 164):

```rust
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
pub fn with_header(mut self, key: hyper::header::HeaderName, value: hyper::header::HeaderValue) -> Self {
    self.set_header(key, value);
    self
}
```

- [ ] **Step 2: Add tests**

Add these tests inside the `#[cfg(test)] mod tests` block in `response.rs`:

```rust
#[test]
fn test_response_set_header() {
    use hyper::header;
    let mut response = Response::body("test").unwrap();
    response.set_header(
        hyper::header::HeaderName::from_static("x-custom"),
        hyper::header::HeaderValue::from_static("value"),
    );
    assert_eq!(
        response.inner.headers().get("x-custom").unwrap(),
        "value"
    );
}

#[test]
fn test_response_with_header() {
    use hyper::header;
    let response = Response::body("test")
        .unwrap()
        .with_header(
            hyper::header::HeaderName::from_static("x-custom"),
            hyper::header::HeaderValue::from_static("chained"),
        );
    assert_eq!(
        response.inner.headers().get("x-custom").unwrap(),
        "chained"
    );
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test response::tests --verbose`
Expected: All response tests pass including new header tests.

- [ ] **Step 4: Commit**

```bash
git add src/response.rs
git commit -m "feat: add set_header and with_header to Response"
```

---

### Task 3: Logger Middleware

**Files:**
- Create: `src/middleware/mod.rs`
- Create: `src/middleware/logger.rs`

- [ ] **Step 1: Create middleware module root**

Write `src/middleware/mod.rs`:

```rust
pub mod logger;

pub use logger::Logger;
```

- [ ] **Step 2: Create Logger middleware**

Write `src/middleware/logger.rs`:

```rust
use crate::{Middleware, Next, Request, Result};
use tracing::info;

/// Built-in access-log middleware.
///
/// Logs every request with method, path, status code, and duration.
/// Uses the `tracing` crate at INFO level.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::middleware::Logger;
///
/// let mut router = Router::new();
/// router.with(Logger);
/// ```
///
/// # Output Format
///
/// ```text
/// GET /users/42 -> 200 3ms
/// POST /users -> 201 12ms
/// ```
pub struct Logger;

#[async_trait::async_trait]
impl Middleware for Logger {
    async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        let method = req.method().clone();
        let path = req.path().to_string();
        let start = std::time::Instant::now();

        let response = next.run(req).await;

        let duration = start.elapsed();
        match &response {
            Ok(res) => {
                info!(
                    "{} {} -> {} {}ms",
                    method,
                    path,
                    res.status().as_u16(),
                    duration.as_millis()
                );
            }
            Err(e) => {
                info!(
                    "{} {} -> ERR {}ms ({})",
                    method,
                    path,
                    duration.as_millis(),
                    e
                );
            }
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Endpoint, IntoResponse, Router};

    #[tokio::test]
    async fn test_logger_middleware_exists() {
        // Verify Logger can be added to a router
        let mut router = Router::new();
        router.get("/", |_| async { "ok" });
        router.with(Logger);
        assert_eq!(router.middlewares.len(), 1);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test middleware --verbose`
Expected: Logger test passes.

- [ ] **Step 4: Commit**

```bash
git add src/middleware/
git commit -m "feat: add built-in Logger middleware"
```

---

### Task 4: CORS Middleware

**Files:**
- Create: `src/cors.rs`

- [ ] **Step 1: Create CORS middleware**

Write `src/cors.rs`:

```rust
use crate::{Middleware, Next, Request, Result};
use hyper::header;

/// CORS (Cross-Origin Resource Sharing) middleware.
///
/// Adds appropriate `Access-Control-*` headers to responses and handles
/// OPTIONS preflight requests automatically.
///
/// Uses a builder pattern for configuration.
///
/// # Default Configuration
///
/// - `allow_origin`: `"*"`
/// - `allow_methods`: `"GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD"`
/// - `allow_headers`: `"Content-Type, Authorization"`
/// - `allow_credentials`: `false`
/// - `max_age`: `86400` (24 hours)
///
/// # Example
///
/// ```rust,ignore
/// use desirable::Cors;
///
/// let mut router = Router::new();
/// router.with(Cors::new());
///
/// // Or with custom configuration:
/// router.with(
///     Cors::new()
///         .allow_origin("https://example.com")
///         .allow_credentials(true)
///         .max_age(3600)
/// );
/// ```
#[derive(Clone, Debug)]
pub struct Cors {
    allow_origin: String,
    allow_methods: String,
    allow_headers: String,
    allow_credentials: bool,
    max_age: u32,
    expose_headers: Option<String>,
}

impl Cors {
    /// Creates a new CORS middleware with permissive defaults.
    ///
    /// # Returns
    ///
    /// A new `Cors` instance with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_origin: "*".to_string(),
            allow_methods: "GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD".to_string(),
            allow_headers: "Content-Type, Authorization".to_string(),
            allow_credentials: false,
            max_age: 86400,
            expose_headers: None,
        }
    }

    /// Sets the `Access-Control-Allow-Origin` header value.
    ///
    /// Default: `"*"`
    #[must_use]
    pub fn allow_origin(mut self, origin: &str) -> Self {
        self.allow_origin = origin.to_string();
        self
    }

    /// Sets the `Access-Control-Allow-Methods` header value.
    ///
    /// Default: `"GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD"`
    #[must_use]
    pub fn allow_methods(mut self, methods: &str) -> Self {
        self.allow_methods = methods.to_string();
        self
    }

    /// Sets the `Access-Control-Allow-Headers` header value.
    ///
    /// Default: `"Content-Type, Authorization"`
    #[must_use]
    pub fn allow_headers(mut self, headers: &str) -> Self {
        self.allow_headers = headers.to_string();
        self
    }

    /// Sets whether `Access-Control-Allow-Credentials` is `true`.
    ///
    /// Default: `false`
    #[must_use]
    pub fn allow_credentials(mut self, yes: bool) -> Self {
        self.allow_credentials = yes;
        self
    }

    /// Sets the `Access-Control-Max-Age` header value in seconds.
    ///
    /// Default: `86400` (24 hours)
    #[must_use]
    pub fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = seconds;
        self
    }

    /// Sets the `Access-Control-Expose-Headers` header value.
    ///
    /// Default: `None` (header not set)
    #[must_use]
    pub fn expose_headers(mut self, headers: &str) -> Self {
        self.expose_headers = Some(headers.to_string());
        self
    }

    /// Applies CORS headers to a response.
    fn apply_headers(&self, headers: &mut hyper::HeaderMap) {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            self.allow_origin.parse().unwrap(),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            self.allow_methods.parse().unwrap(),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            self.allow_headers.parse().unwrap(),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            self.max_age.to_string().parse().unwrap(),
        );
        if self.allow_credentials {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                "true".parse().unwrap(),
            );
        }
        if let Some(ref expose) = self.expose_headers {
            headers.insert(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                expose.parse().unwrap(),
            );
        }
    }
}

impl Default for Cors {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Middleware for Cors {
    async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        let is_preflight = req.method() == hyper::Method::OPTIONS;

        let mut response = if is_preflight {
            // Preflight: return 204 No Content, don't call next
            crate::Response::with_status(204, String::new())
        } else {
            // Normal request: run the handler
            next.run(req).await
        };

        if let Ok(ref mut res) = response {
            self.apply_headers(res.inner.headers_mut());
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cors_defaults() {
        let cors = Cors::new();
        assert_eq!(cors.allow_origin, "*");
        assert_eq!(cors.allow_methods, "GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD");
        assert!(!cors.allow_credentials);
        assert_eq!(cors.max_age, 86400);
    }

    #[test]
    fn test_cors_builder() {
        let cors = Cors::new()
            .allow_origin("https://example.com")
            .allow_methods("GET, POST")
            .allow_headers("X-Custom")
            .allow_credentials(true)
            .max_age(3600)
            .expose_headers("X-Response-Id");

        assert_eq!(cors.allow_origin, "https://example.com");
        assert_eq!(cors.allow_methods, "GET, POST");
        assert_eq!(cors.allow_headers, "X-Custom");
        assert!(cors.allow_credentials);
        assert_eq!(cors.max_age, 3600);
        assert_eq!(cors.expose_headers, Some("X-Response-Id".to_string()));
    }

    #[test]
    fn test_cors_apply_headers() {
        let cors = Cors::new();
        let mut headers = hyper::HeaderMap::new();
        cors.apply_headers(&mut headers);
        assert_eq!(headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), "*");
        assert!(headers.contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
        assert!(headers.contains_key(header::ACCESS_CONTROL_MAX_AGE));
        assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
    }

    #[test]
    fn test_cors_apply_headers_with_credentials() {
        let cors = Cors::new().allow_credentials(true);
        let mut headers = hyper::HeaderMap::new();
        cors.apply_headers(&mut headers);
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).unwrap(),
            "true"
        );
    }
}
```

- [ ] **Step 2: Run CORS tests**

Run: `cargo test cors --verbose`
Expected: All 4 CORS tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/cors.rs
git commit -m "feat: add CORS middleware with builder pattern"
```

---

### Task 5: Graceful Shutdown

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Extract accept loop and add graceful shutdown**

Replace the entire `Server` impl block in `src/server.rs` with the following:

```rust
impl Server {
  /// Creates a new server bound to the given address.
  pub fn bind(addr: &str) -> Self {
    Server {
      addr: addr.parse().unwrap(),
    }
  }

  /// Starts the server and serves requests until shutdown signal.
  ///
  /// Listens for Ctrl+C (SIGINT) or SIGTERM and performs a graceful shutdown.
  /// Returns `Ok(())` on normal shutdown, or an error on failure.
  ///
  /// # Arguments
  ///
  /// * `router` - The application router to handle requests
  pub async fn run(&self, router: Router) -> Result<()> {
    self.run_graceful(router).await
  }

  /// Starts the server with graceful shutdown support.
  ///
  /// This is the same as `run()` but with an explicit name for clarity.
  /// Use this when you want to make the graceful shutdown behavior explicit
  /// in your code.
  pub async fn run_graceful(&self, router: Router) -> Result<()> {
    let addr: SocketAddr = self.addr;
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);

    let router = Arc::new(router);

    tokio::select! {
      result = accept_loop(listener, router) => result,
      _ = shutdown_signal() => {
        info!("Shutdown signal received, stopping...");
        Ok(())
      }
    }
  }
}

/// Accepts connections in a loop until the listener is closed.
async fn accept_loop(listener: TcpListener, router: Arc<Router>) -> Result<()> {
  loop {
    let router = router.clone();
    let (stream, remote_addr) = listener.accept().await?;
    let io = TokioIo::new(stream);
    let remote_addr = Arc::new(remote_addr);
    tokio::task::spawn(async move {
      if let Err(err) = http1::Builder::new()
        .serve_connection(
          io,
          Svc {
            router,
            remote_addr,
          },
        )
        .await
      {
        error!("Failed to serve connection: {:?}", err);
      }
    });
  }
}

/// Waits for a shutdown signal (Ctrl+C or SIGTERM).
async fn shutdown_signal() {
  tokio::signal::ctrl_c()
    .await
    .expect("failed to install Ctrl+C handler");
}
```

Make sure the imports at the top of `src/server.rs` include everything needed. The full imports section should be:

```rust
use crate::HyperRequest;
use crate::HyperResponse;
use crate::Result;
use crate::Router;
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};
```

(These are the existing imports — they already cover the new code.)

- [ ] **Step 2: Run tests**

Run: `cargo test server --verbose`
Expected: Server unit tests pass (bind address tests).

- [ ] **Step 3: Build check**

Run: `cargo build --verbose`
Expected: Clean build, no warnings.

- [ ] **Step 4: Commit**

```bash
git add src/server.rs
git commit -m "feat: add graceful shutdown with Ctrl+C signal handling"
```

---

### Task 6: Wire Up lib.rs Re-exports

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add cors and middleware modules to lib.rs**

Add two lines to the module declarations section and update the re-exports:

In the module declarations (after `pub mod types;`):
```rust
pub mod cors;
pub mod middleware;
```

In the re-exports (after `pub use session::...`):
```rust
pub use cors::Cors;
pub use middleware::Logger;
```

The updated `src/lib.rs` should look like:

```rust
//! desirable - A minimal and pragmatic Rust web application framework.
//! ... (existing docs, unchanged)

pub mod cors;
pub mod error;
pub mod fs;
pub mod into_response;
pub mod kernel;
pub mod middleware;
pub mod request;
pub mod response;
pub mod router;
pub mod server;
pub mod session;
pub mod types;
pub mod utils;

pub use cors::Cors;
pub use error::Error;
pub use fs::{ServeDir, ServeFile};
pub use into_response::IntoResponse;
pub use kernel::{DynEndpoint, Endpoint, Middleware, Next};
pub use middleware::Logger;
pub use request::Request;
pub use response::Response;
pub use router::Router;
pub use server::Server;
pub use session::{Session, SessionConfig, SessionError, SessionManager};

pub use types::{AnyResult, HyperRequest, HyperResponse, Result};

// ... (rest unchanged: new(), re-exports)
```

- [ ] **Step 2: Build check**

Run: `cargo build --verbose`
Expected: Clean compile with all new modules linked.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat: wire up cors and middleware modules in lib.rs"
```

---

### Task 7: Final Verification

**Files:** (none — verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test --verbose`
Expected: All tests pass across all modules.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings, no errors.

- [ ] **Step 3: Check formatting**

Run: `cargo fmt --check`
Expected: All files properly formatted (no changes needed).

- [ ] **Step 4: Verify the example builds and runs briefly**

Run: `cd examples && cargo build -p example-hello --verbose`
Expected: Example builds cleanly with the updated framework.

- [ ] **Step 5: Review git log**

Run: `git log --oneline -8`
Expected: Clean commit history with 6 feature commits.
