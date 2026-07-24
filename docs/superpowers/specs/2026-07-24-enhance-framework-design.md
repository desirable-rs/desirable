# Design: desirable v1.1 Enhancement

**Date**: 2026-07-24
**Scope**: Conservative enhancement — upgrade deps, graceful shutdown, CORS, logging middleware

---

## Overview

Enhance the `desirable` Rust web framework from v1.0.1 to v1.1.0 with production-essential features while maintaining its minimal, lightweight philosophy. Total new code: ~250 lines.

---

## 1. Dependency Upgrade

### Strategy
Update all crates to latest compatible versions. One breaking change: `rand` from 0.8 → 0.9.

### Version Targets

| Crate | From | To | Notes |
|-------|------|----|-------|
| anyhow | 1.0.100 | 1 (latest) | patch |
| async-trait | 0.1.89 | 0.1 (latest) | patch |
| base64 | 0.22.0 | 0.22 (latest) | patch |
| bytes | 1.11.0 | 1 (latest) | minor |
| chrono | 0.4.43 | 0.4 (latest) | patch |
| cookie | 0.18.0 | 0.18 (latest) | patch |
| hmac | 0.12.1 | 0.12 (latest) | patch |
| http-body | 1.0.1 | 1 (latest) | patch |
| http-body-util | 0.1.3 | 0.1 (latest) | patch |
| hyper | 1.8.1 | 1 (latest) | minor |
| hyper-util | 0.1.19 | 0.1 (latest) | patch |
| mime | 0.3.17 | 0.3 (latest) | patch |
| **rand** | **0.8.5** | **0.9** | **breaking: API adapt** |
| route-recognizer | 0.3.1 | 0.3 (latest) | patch |
| serde | 1.0.228 | 1 (latest) | patch |
| serde_json | 1.0.149 | 1 (latest) | patch |
| serde_urlencoded | 0.7.1 | 0.7 (latest) | patch |
| sha2 | 0.10.8 | 0.10 (latest) | patch |
| thiserror | 2.0.18 | 2 (latest) | patch |
| time | 0.3.36 | 0.3 (latest) | patch |
| tokio | 1.49.0 | 1 (latest) | minor |
| tracing | 0.1.44 | 0.1 (latest) | patch |

### rand 0.9 API Migration
- `rand::thread_rng()` → `rand::rng()`
- `RngCore::fill_bytes()` stays compatible
- Affected: `src/session.rs` (3 call sites in `SessionData::new`, `Session::regenerate_id`, `SessionConfig::default`)

---

## 2. Graceful Shutdown

### Current State
`Server::run()` has an infinite `loop` over `listener.accept()`. No signal handling. Process killed by SIGINT.

### Design
Add `Server::run_graceful()` using `tokio::select!` + `tokio::signal::ctrl_c()`:

```rust
pub async fn run_graceful(&self, router: Router) -> Result<()> {
    let listener = TcpListener::bind(self.addr).await?;
    info!("Listening on http://{}", self.addr);

    let router = Arc::new(router);
    let shutdown = shutdown_signal();

    tokio::select! {
        result = accept_loop(listener, router) => result,
        _ = shutdown => {
            info!("Shutdown signal received, stopping...");
            Ok(())
        }
    }
}
```

### Decisions
- `run()` delegates to `run_graceful()` for backward compatibility
- `accept_loop()` extracted as a separate async fn for readability
- Zero new dependencies (tokio signal is in `full` feature)
- No connection draining in v1 — future enhancement

### Files Changed
- `src/server.rs`: +40 lines, ~10 modified

---

## 3. CORS Middleware

### Design
New module `src/cors.rs` implementing `Middleware`. Builder pattern, zero external deps.

```rust
pub struct Cors {
    allow_origin: String,
    allow_methods: String,
    allow_headers: String,
    allow_credentials: bool,
    max_age: u32,
    expose_headers: Option<String>,
}

impl Cors {
    pub fn new() -> Self { /* defaults */ }
    pub fn allow_origin(mut self, origin: &str) -> Self { .. }
    pub fn allow_methods(mut self, methods: &str) -> Self { .. }
    pub fn allow_headers(mut self, headers: &str) -> Self { .. }
    pub fn allow_credentials(mut self, yes: bool) -> Self { .. }
    pub fn max_age(mut self, seconds: u32) -> Self { .. }
    pub fn expose_headers(mut self, headers: &str) -> Self { .. }
}
```

### Default Configuration
- `allow_origin`: `"*"`
- `allow_methods`: `"GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD"`
- `allow_headers`: `"Content-Type, Authorization"`
- `allow_credentials`: `false`
- `max_age`: `86400` (24h)
- `expose_headers`: `None`

### Middleware Behavior
```
handle(req, next):
  1. Run next(req) to get response
  2. Add Access-Control-* headers to response based on config
  3. If req is OPTIONS (preflight), return 204 directly
  4. Return modified response
```

### Public API
- `Cors::new()` — default permissive CORS
- Builder methods for customization
- Re-exported from `lib.rs`: `pub use cors::Cors;`

### Files Changed
- `src/cors.rs`: +120 lines (new)
- `src/lib.rs`: +2 lines (module + re-export)

---

## 4. Built-in Logging Middleware

### Design
New module `src/middleware/logger.rs`. A simple access-log middleware using `tracing`.

```rust
pub struct Logger;

#[async_trait::async_trait]
impl Middleware for Logger {
    async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        let method = req.method().clone();
        let path = req.path().to_string();
        let start = std::time::Instant::now();

        let response = next.run(req).await;

        let status = response.as_ref().map(|r| r.status().as_u16()).unwrap_or(500);
        let duration = start.elapsed();
        info!("{} {} → {} {}ms", method, path, status, duration.as_millis());

        response
    }
}
```

### Output Format
```
GET /users/42 → 200 3ms
POST /users → 201 12ms
GET /nonexistent → 404 1ms
```

### Usage
```rust
app.with(middleware::Logger);
```

### Files Changed
- `src/middleware/mod.rs`: +5 lines (new)
- `src/middleware/logger.rs`: +50 lines (new)
- `src/lib.rs`: +1 line (module declaration)

---

## 5. Response Enhancements

### New Methods on `Response`

```rust
impl Response {
    /// Set a response header (takes ownership, returns Self for chaining)
    pub fn with_header(mut self, key: HeaderName, value: HeaderValue) -> Self { .. }

    /// Set a response header by mutable reference
    pub fn set_header(&mut self, key: HeaderName, value: HeaderValue) { .. }
}
```

### Files Changed
- `src/response.rs`: +20 lines

---

## File Change Summary

| File | Action | Approx Lines |
|------|--------|-------------|
| `Cargo.toml` | Modify | ~20 |
| `src/lib.rs` | Modify | +5 |
| `src/server.rs` | Modify | +40 |
| `src/session.rs` | Modify | ~6 |
| `src/response.rs` | Modify | +20 |
| `src/cors.rs` | **New** | +120 |
| `src/middleware/mod.rs` | **New** | +5 |
| `src/middleware/logger.rs` | **New** | +50 |
| **Total** | | **~260 lines** |

Zero new external dependencies. All features use existing crates.

---

## Version Bump

`Cargo.toml`: `version = "1.0.1"` → `version = "1.1.0"`

---

## Testing Strategy

- Unit tests for `Cors` builder and header generation
- Unit tests for `Logger` middleware (status code capture)
- Unit tests for `Response::with_header` / `set_header`
- Existing tests must pass without modification
- Manual integration test: run example server, verify Ctrl+C graceful shutdown
