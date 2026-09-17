# Changelog

All notable changes to the desirable web framework will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [2.0.0] - 2026-09-18

### Breaking

- **The response body is now `Body`** (`crate::Body`) — a concrete enum of
  `Full(Bytes)` and `Streaming(BoxBody)` — replacing the previous fixed
  `Full<Bytes>`:
  - `HyperResponse` is now `hyper::Response<Body>`; code that names the
    body type through the alias must adapt.
  - `Response::inner` (public field) now exposes `Response<Body>`.
  - `IntoResponse for Body` added; `IntoResponse for Full<Bytes>` retained.
  - `ResponseBuilder::body` / `Response::body` now accept anything that is
    `Into<Body>` (covers all previous inputs).
- **`desirable::utils` removed** (deprecated since 1.9; never contained code).
- **`desirable::hyper::body` re-export dropped** (the module name now hosts
  `desirable::body`). Use `hyper::body` directly.

### Added

- **`Body::stream(S)`** — stream any `Stream<Item = Result<impl Into<Bytes>,
  E>>` chunk by chunk.
- **`Body::channel(cap)`** — a backpressured `(BodySender, Body)` pair for
  dynamically produced responses (e.g. server-sent events).
- **Static files now stream** instead of buffering the whole file: memory
  use is one 64 KiB chunk regardless of file size. The exact length from
  `fstat` is preserved as the body's size hint, so responses still carry
  `Content-Length` (not chunked encoding).
- New direct dependency: `futures-core` (Stream trait; already present
  transitively). `tokio-util` gains the `io` feature (same crate).

---

## [1.10.0] - 2026-09-18

### Changed

- **Middleware hot-path optimizations** (no behavior changes):
  - **Cors** precomputes its `HeaderValue`s at construction time — the
    request path previously parsed 4–6 header values (including a
    `to_string()` round-trip for `max_age`) on every response. Invalid
    configured values now panic at construction instead of on first use.
  - **SessionLayer** uses a borrowing cookie lookup (`get_cookie_value_str`)
    instead of allocating an owned `String` per request; the public
    `get_cookie_value` is unchanged.
  - **Static files** drop the redundant pre-check stat: a regular file is
    served with one `open` + `fstat` (was 2 stats + 1 open). Directory-index
    fallback happens inside the open flow.
  - **Router** skips the `ScopedEndpoint` wrapper for routes registered
    without middleware, removing one async call layer per request for those
    routes.
  - **RequestId** generates IDs via a hex lookup table (was 16 `format!`
    allocations) and caches its `HeaderName`.

---

## [1.9.0] - 2026-09-18

### Fixed

- **Session cookies were truncated at base64 padding.** `get_cookie_value`
  split on every `=`, so a signed cookie value ending in `=` padding (the
  common case for base64) lost its tail and failed signature verification —
  silently starting a new session. Values are now taken whole via
  `split_once`. Also removes a per-request (per-segment!) `format!`
  allocation from the lookup.

### Changed

- **`Router::dispatch` no longer allocates per request**: the method-clone
  and path-`String` allocations were removed in favor of scoped borrows.
- **Static files are opened once**: `metadata` + `read` (two path lookups)
  became a single `File::open` + `fstat` + `read_to_end`, also narrowing the
  stat/read race window.
- Internal status-code construction no longer re-parses `u16` literals
  (new crate-internal `Response::with_status_code`); static `HeaderValue`
  constants are passed by reference instead of cloned.
- **Architecture:** the 1,620-line `session.rs` module was split into
  `session/{mod,config,data,error,manager}.rs` (public paths unchanged);
  duplicate content-type constants were unified in `response.rs`; six
  duplicated response-builder chains in `into_response.rs` now share one
  helper.

### Deprecated

- **`desirable::utils`** — an empty placeholder module since 1.0. It will be
  removed in 2.0.

---

## [1.8.0] - 2026-09-17

### Added

- **`SessionLayer` middleware** — automatic session management. Install with
  `app.with(SessionLayer::new(manager))`; handlers read and mutate
  `req.session()` (a cloneable handle dereferencing to `Mutex<Session>`).
  The session is loaded from its signed cookie per request, and `Set-Cookie`
  is appended to the response **only when the session was modified** —
  anonymous traffic produces no cookie. Missing/empty cookies start a fresh
  session; tampered cookies are logged and replaced rather than failing the
  request.
- **`Request::session()`** — returns the `SessionHandle`; without the layer
  installed it resolves to a detached no-op session (debug-logged) instead
  of panicking.

---

## [1.7.1] - 2026-09-17

### Changed

- **Documentation:** refreshed README — crates.io/docs.rs/CI badges, a
  compile-verified quick start with typed state, and an overview of the
  built-in middleware, error semantics, and graceful shutdown. No code
  changes.

---

## [1.7.0] - 2026-07-25

### Added

- **Typed application state.** `Router::with_state(state)` stores shared
  state (DB pools, config); handlers read it with
  `req.state::<T>() -> Option<Arc<T>>` (type-checked downcast). State is
  injected into every request's extensions at dispatch time; on `merge()`,
  the parent's state wins.
- **SIGTERM triggers graceful shutdown** (in addition to Ctrl+C/SIGINT), so
  orchestrators like Kubernetes and systemd drain connections properly.
- **Trailing-slash tolerance.** `GET /users/` now matches a route registered
  as `/users` (exact match still wins); 405 `Allow` detection is slash-aware
  too.
- **`set_error_handler()`** — a process-wide custom renderer for every
  handler error, enabling uniform JSON error envelopes:
  `set_error_handler(|err| Response::builder().status(err.status()).json(...))`.

### Changed

- `Next::run` now propagates middleware errors to the caller instead of
  converting them in place, matching its documented behavior and enabling
  outer layers to observe failures.

---

## [1.6.0] - 2026-07-25

### Fixed

- **All errors returned HTTP 500** regardless of cause. Errors now map to
  proper status codes: missing/invalid parameters, malformed query strings
  and JSON bodies → `400 Bad Request`; oversized bodies →
  `413 Payload Too Large`; everything else stays `500`.
- **Server error details were leaked in response bodies** (e.g. raw
  `std::io::Error` debug output). 5xx responses now carry a generic
  `"internal server error"` body while the real error is logged via
  `tracing::error!`. 4xx responses keep their descriptive messages.
- **Missing static files returned 500** with leaked IO details. `ServeFile`
  and `ServeDir` now return a clean `404 Not Found`.
- **Connection errors were silently swallowed** when a response could not be
  produced; they are now logged.

### Added

- **`BodyLimit` middleware** rejects oversized request bodies with
  `413 Payload Too Large` — immediately when `Content-Length` is declared,
  and at read time (via `http_body_util::Limited`) for chunked bodies.
- **`RateLimit` middleware** — in-memory per-client-IP token buckets
  (`RateLimit::per_second(n)` or explicit burst + refill), returning
  `429 Too Many Requests` with `Retry-After` when exhausted. Zero external
  dependencies; process-local state.
- **`Error::status()`** — public method mapping an error to its HTTP status.

---

## [1.5.0] - 2026-07-25

### Added

- **Connection draining on shutdown.** On shutdown the server stops accepting,
  tells each in-flight connection to finish its current request (hyper's
  `graceful_shutdown`), and waits up to `Server::drain_timeout` (default 10s,
  configurable) before exiting. Previously in-flight connections were cut
  off abruptly.
- **`Server::try_bind(addr) -> Result<Server>`** parses the address without
  panicking; `bind()` keeps its existing panic behavior.
- **`Server::run_with_shutdown(router, signal)`** makes shutdown programmable
  — trigger it from tests, orchestration, or custom signals. `run_graceful`
  now delegates to it with a Ctrl+C future.
- **`TCP_NODELAY` is enabled** on accepted connections.
- **Static files support conditional requests.** `ServeFile`/`ServeDir`
  responses carry a weak `ETag` (`W/"{mtime:x}-{size:x}"`) and `Last-Modified`;
  matching `If-None-Match` or `If-Modified-Since` requests receive
  `304 Not Modified` with an empty body.

### Changed

- New dependencies `httpdate` and `tokio-util` (`rt` feature) — both were
  already present in the dependency graph transitively, so the lockfile does
  not grow.

---

## [1.4.0] - 2026-07-25

### Fixed

- **`Router::merge()` silently dropped the parent's routes** for any HTTP
  method registered on both routers (`HashMap::extend` replaced whole route
  tables). Tables are now appended per method.
  **Breaking (internal shape):** `Router::routes` is now
  `HashMap<Method, Vec<route_recognizer::Router<_>>>` — code inspecting the
  field directly must adapt.

### Changed

- **Middleware is now scoped at registration time** (axum-style ordering):
  `Router::with()` applies to routes registered *after* it, and that chain
  travels with the routes through `merge()`. Previously middleware applied to
  the whole router and was discarded on merge. The built-in 404/405 fallbacks
  always run the router-level chain. Register `with()` before routes.

### Added

- **`Router::prefix()`** builder for nesting:
  `app.merge(Router::new().prefix("/api").with(Auth))`.
- **HEAD requests fall back to GET routes**; the response body is stripped
  per HTTP semantics. Previously HEAD returned 405 when only GET existed.
- **`ServeDir` directory index**: when the path resolves to a directory,
  `index.html` inside it is served.
- **`Response::set_cookie()` / `remove_cookie()`** with append semantics
  (multiple `Set-Cookie` headers supported), plus `append_header()` and a
  public `headers_mut()`.
- **`Timeout` middleware** aborts requests past the configured duration and
  responds `408 Request Timeout`.

---

## [1.3.0] - 2026-07-25

### Fixed

- **Path traversal in `ServeDir`** (security). A `:file` parameter containing
  `..` or absolute components (e.g. `/assets/../../etc/passwd`) could read
  arbitrary files. Such requests now receive `403 Forbidden`.
- **Default 404 responses returned HTTP 200.** The built-in not-found handler
  now returns `404 Not Found` with a plain-text body.
- **Method mismatch returned 404 instead of 405.** When a path exists under
  other HTTP methods, the router now responds `405 Method Not Allowed` with
  an `Allow: GET, POST, ...` header listing the supported methods.
- **ServeFile/ServeDir never set Content-Type** despite the docs claiming it.
  The type is now inferred from the file extension (html, css, js, json,
  images, fonts, media); unknown extensions fall back to
  `application/octet-stream`.

### Added

- **`Request::form::<T>()`** parses `application/x-www-form-urlencoded`
  request bodies into any `DeserializeOwned` type.
- **`Request::header(name)`** reads a single request header.
- **`Request::cookie(name)`** parses a named cookie from the `Cookie` header.
- **`Response::html()`** constructs a response with
  `Content-Type: text/html; charset=utf-8`.
- **`RequestId` middleware** — reuses the inbound `X-Request-Id` header or
  generates a random 32-char hex ID, stores it in request extensions as
  `RequestIdValue`, and echoes it on the response.

---

## [1.2.0] - 2026-07-25

### Changed

- **`Response::json()` no longer requires `'static`, `Send`, or `Sync` bounds.**
  Only `Serialize` is needed. You can now pass references (`&T`) directly.
- **`Response::json()` now returns `Response` instead of `Result<Response>`.**
  Serialization errors cause a panic (should never happen for well-formed types).
  **Breaking:** remove `?` or `.unwrap()` from existing `Response::json()` calls.

### Added

- **`Response::builder()`** returns a `ResponseBuilder` with chainable methods:
  `.status()`, `.status_u16()`, `.header()`, `.json()`, `.text()`, `.body()`.
- **`Request::query_or_default()`** parses query strings with `Default` fallback when absent.
- **`Request::body_json()`** alias for `Request::body()` with a clearer name.
- **`Request::param_str()`** extracts a `String` path parameter without turbofish.
- **`IntoResponse` impl for `serde_json::Value`** — return `serde_json::json!(...)` directly from handlers.

---

## [1.1.0] - 2026-07-24

### Added

- **Graceful shutdown.** `Server::run()` now listens for Ctrl+C (SIGINT) and shuts down cleanly.
  `Server::run_graceful()` is also available for explicit control.
- **CORS middleware.** `Cors` struct with builder pattern (`allow_origin`, `allow_methods`, `allow_credentials`, `max_age`, `expose_headers`).
- **Logger middleware.** `Logger` logs `{method} {path} → {status} {duration}ms` via `tracing`.
- **`Response::set_header()` and `Response::with_header()`** for manipulating response headers.

### Changed

- **Dependencies upgraded:** `rand` 0.8 → 0.9 (breaking API: `thread_rng()` → `rng()`),
  `hyper` 1.8 → 1.11, `tokio` 1.49 → 1.53, and all other crates bumped to latest.
- **Edition:** Rust 2024 (already in 1.0.x).

---

## [1.0.1] - 2026-01-29

### Added

- **Session management.** Cookie-based sessions with HMAC-SHA256 signing, Base64URL encoding,
  and full `SessionConfig` builder (cookie name, path, domain, secure, http_only, same_site, max_age).
- **Benchmarks** via `criterion`.

### Changed

- Optimized internal code.
- Improved session documentation.

---

## [1.0.0] - 2025-07-24

### Added

- Initial public release.
- **Router** with HTTP method routing (`get`, `post`, `put`, `delete`, `patch`, `options`, `head`, `trace`, `connect`) and path parameter support (`:id`).
- **Middleware** trait with composable `Next` chain.
- **Request** type with JSON body parsing, query string deserialization, and path parameter extraction.
- **Response** type with `json()`, `body()`, `with_status()`, `redirect()` constructors.
- **IntoResponse** trait with impls for `String`, `&str`, `()`, `Bytes`, `(StatusCode, T)`, `(u16, T)`, `Result<T, E>`.
- **Static file serving** via `ServeFile` and `ServeDir`.
- **Error handling** via `thiserror`-derived `Error` enum.
- Re-exports of `hyper::body`, `hyper::header`, `hyper::http`.

---

[1.7.0]: https://github.com/desirable-rs/desirable/compare/v1.6.0...v1.7.0
[2.0.0]: https://github.com/desirable-rs/desirable/compare/v1.10.0...v2.0.0
[1.10.0]: https://github.com/desirable-rs/desirable/compare/v1.9.0...v1.10.0
[1.9.0]: https://github.com/desirable-rs/desirable/compare/v1.8.0...v1.9.0
[1.8.0]: https://github.com/desirable-rs/desirable/compare/v1.7.1...v1.8.0
[1.7.1]: https://github.com/desirable-rs/desirable/compare/v1.7.0...v1.7.1
[1.7.0]: https://github.com/desirable-rs/desirable/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/desirable-rs/desirable/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/desirable-rs/desirable/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/desirable-rs/desirable/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/desirable-rs/desirable/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/desirable-rs/desirable/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/desirable-rs/desirable/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/desirable-rs/desirable/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/desirable-rs/desirable/releases/tag/v1.0.0
