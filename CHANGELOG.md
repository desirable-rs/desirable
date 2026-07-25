# Changelog

All notable changes to the desirable web framework will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[1.2.0]: https://github.com/desirable-rs/desirable/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/desirable-rs/desirable/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/desirable-rs/desirable/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/desirable-rs/desirable/releases/tag/v1.0.0
