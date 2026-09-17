# Roadmap

This document tracks what has been delivered, what is deliberately deferred,
and the reasoning behind each decision. Anything marked **deferred** is a
conscious trade-off for the framework's lightweight positioning — not an
oversight. Each deferred item is worth its own design round when a real need
arrives.

## Released

| Version | Theme | Delivered |
|---------|-------|-----------|
| 1.1.0 | Dependencies + infrastructure | 21 crates upgraded (rand 0.9 migration), graceful shutdown (Ctrl+C), CORS middleware, Logger middleware, `Response::set_header`/`with_header` |
| 1.2.0 | Developer experience | `Response::json` without `'static` bounds (returns `Response` directly — **breaking**), `Response::builder()`, `query_or_default()`, `body_json()`, `param_str()`, `IntoResponse` for `serde_json::Value` |
| 1.3.0 | Security + correctness | `ServeDir` path-traversal protection (403), default 404 status fix, `405 + Allow` semantics, static-file `Content-Type` detection, `form()` parsing, `header()`/`cookie()` accessors, `Response::html()`, RequestId middleware |
| 1.4.0 | Routing maturity | Router nesting (`prefix()` + `merge()` with scoped middleware), HEAD→GET fallback with body stripping, directory `index.html`, cookie API (`set_cookie`/`remove_cookie`/`append_header`), Timeout middleware, **fix**: `merge()` no longer drops parent routes of shared methods |
| 1.5.0 | Production operations | Connection draining with configurable timeout (`TaskTracker` + hyper `graceful_shutdown`), `try_bind()`, `run_with_shutdown()`, TCP_NODELAY, static-file `ETag`/`Last-Modified`/`304` |
| 1.6.0 | Semantics + protection | Error→status mapping (400/413/500), 5xx bodies no longer leak internals, static-file 404, BodyLimit middleware (413, incl. chunked via `Limited`), RateLimit middleware (429 + Retry-After, per-IP token buckets), connection-error logging |
| 1.7.0 | DX completion | Typed app state (`with_state` + `req.state::<T>()`), SIGTERM graceful shutdown, trailing-slash tolerance, process-wide `set_error_handler()`, `Next::run` error propagation |
| 1.8.0 | Session automation | `SessionLayer` middleware (load-from-cookie, auto `Set-Cookie` only when modified, tamper resilience), `Request::session()` handle |
| 1.7.1 | Docs only | README refresh (badges, compile-verified quick start, middleware overview) |
| 1.9.0 | Performance + architecture | Zero-alloc dispatch, cookie-padding truncation fix (security), single-open static files, `session.rs` split into a directory module, content-type constant unification |
| 1.10.0 | Middleware hot paths | Cors precomputed header values, borrowing session-cookie lookup, no redundant stat for static files, wrapper-free routes without middleware, allocation-free RequestId generation |
| 2.0.0 | Streaming bodies | Concrete `Body` enum (`Full` / `Streaming`), `Body::stream` + `Body::channel` (SSE-capable), static files stream by default with exact `Content-Length`; `utils` removed |

## Deferred (conscious trade-offs)

| Item | Why deferred |
|------|--------------|
| WebSocket | Needs a protocol-upgrade stack (hyper upgrades or tungstenite). Large, self-contained project. The 2.0 streaming `Body::channel` removes the biggest prerequisite. |
| TLS (rustls) | Needs certificate configuration surface plus an acceptor layer. Common production answer today is terminating TLS at a reverse proxy. |
| HTTP/2 | Requires rewiring the server layer onto hyper's h2 service. |
| Compression middleware (gzip) | Requires adding `async-compression` — the first new direct dependency. Conflicts with the zero-new-deps principle; acceptable behind a cargo feature flag if demand appears. |
| Function-parameter extractors (axum-style `State<T>` args) | Requires macros or heavyweight trait machinery. `req.state::<T>()` covers the primary use case. |
| Framework-level fix for `?` in anonymous closures | Rust inference limitation, not fixable without specialization. Documented guidance: use named `async fn` handlers returning `desirable::Result`. |
| Fine-grained server options (header read timeout, keep-alive tuning) | No concrete demand yet; `http1::Builder` is currently left at defaults. |
| Static-file `Cache-Control` config / strong content-hash ETags | Weak ETag (`mtime`+`size`) covers the common case; revisit when immutable-asset versioning is needed. |
| `Session::destroy()` deletion-cookie helper | Natural follow-up to `SessionLayer`; small, planned when a use case lands. |
| `X-Forwarded-For`-based rate limiting | Spoofable header; trusting it needs an explicit opt-in proxy configuration. RateLimit keys on the peer address. |
| Unix domain sockets / multiple listeners | Niche deployment shapes; `run_with_shutdown` accepts any shutdown signal, and a `run_listener`-style API is the likely shape when needed. |

## Principles

These constraints shaped every release and should shape future ones:

1. **Zero new dependencies** for convenience features (two exceptions so far — `httpdate` and `tokio-util` — were already transitive).
2. **No macros** for handlers; plain functions and closures.
3. **Correct HTTP semantics first**: status codes, `Allow`, HEAD, conditional requests, and not leaking internals.
4. **Behavior changes are documented** in the CHANGELOG, including internal-type changes.
5. **End-to-end tests over real sockets** for anything behavioral; unit tests for pure logic.
