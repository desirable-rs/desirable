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
| 2.1.0 | Content negotiation & caching | `Compression` middleware (feature-gated gzip), precompressed `.gz`/`.br` static assets, `Cache-Control` config, `Session::destroy()` with deletion-cookie integration, CI `--all-features` coverage |
| 2.2.0 | Static files & proxy environments | Single-range requests (206/416 + If-Range), strong SHA-256 ETag option with caching, `trusted_proxies` + `Request::client_ip()` (rightmost-untrusted X-Forwarded-For), RateLimit keyed on resolved client IP |
| 2.3.0 | WebSocket | `Router::websocket` + `WebSocketConn`/`WebSocketUpgrade` behind the `websocket` feature (tokio-tungstenite); hyper `Upgraded` bridged to tokio IO for tungstenite |
| 2.4.0 | Server capabilities | Unix domain socket servers (`bind_unix`), `run_tcp_listener` for pre-bound listeners, multi-listener deployments with shared shutdown, `http1_header_read_timeout` slow-loris protection |
| 2.5.0 | TLS + HTTP/2 | `tls` feature: rustls acceptor with ALPN h2/h1 negotiation (e2e-verified h2 over TLS), PEM config helper; `tls` dependency subtree isolated behind the feature |
| 2.5.1 | Security patch | Session expiry enforced server-side (cookies replayable past `max_age`); rate-limit map eviction hardened against flush attacks |
| 2.6.0 | HTTP semantics | Malformed JSON/urlencoded bodies map to 400 (typed errors preserved through the `Result` boundary), chunked 413, `Router::head()` routes reachable, method-gated Range/304 (412 for other methods), dir-index MIME |
| 2.7.0 | Operational hardening | Fatal accept errors no longer orphan in-flight connections (drain runs either way, transient errors retried), bounded shutdown-aware TLS handshakes, WS sessions tracked by graceful shutdown, declared MSRV 1.88, CI/Jenkins test coverage fixed |
| 2.7.1 | Docs only | Documentation aligned with behavior (error status mapping, header read timeout default, rate-limit keying, session doctest) |
| 2.8.0 | Hot-path allocations & syscalls | Zero-alloc cookie lookup (parse only the match), fewer static-file syscalls (redundant `metadata`/`fstat` removed, `ServeDir` base resolved once), builder-time `HeaderValue` parsing (`Cache-Control`), precomputed deletion cookie, allocation-free 429, zero-copy fixed-text responses, single-pass `Cow`/`Bytes` responses |

## Roadmap: 2.x → 3.0

Strategy: **2.x grows additively** — protocol capabilities and polish, each
minor independently shippable, new dependencies only behind cargo features so
default builds stay light. **3.0 is the convergence point** for breaking
changes: MSRV/edition, deprecated removals, and the extractor decision.

### v2.1 — Content negotiation & caching ✅ shipped

- **Compression middleware** (feature `compression`): gzip via
  `async-compression` — the first feature-gated direct dependency. Applies to
  `Body::Full` responses; skips already-compressed content types.
- **Precompressed static assets**: serve `.gz`/`.br` siblings when the client
  advertises support (`Content-Encoding` negotiation), zero runtime cost.
- **Static-file `Cache-Control`** configuration, including an immutable mode
  for hashed assets.
- **`Session::destroy()`** integrated with `SessionLayer` (deletion cookie).

### v2.2 — Static files & proxy environments ✅ shipped

- **Range requests** (206 Partial Content, single range) for `ServeFile` /
  `ServeDir` — video seeking and resumable downloads.
- **Trusted proxies**: explicit `trusted_proxies` configuration; extract the
  real client IP from `X-Forwarded-For` into `Request::client_ip()`, shared
  by RateLimit, RequestId and logging. Off by default (spoofable).
- **Strong ETag option** (content hash, computed lazily and cached by
  path+mtime) alongside the default weak validator.

### v2.3 — WebSocket (feature `websocket`) ✅ shipped

- Hyper upgrade plumbing + `tokio-tungstenite` handshake.
- `app.websocket("/ws", handler)`; the handler receives a lightweight
  `WebSocketConn` wrapper. Pairs naturally with `Body::channel` for
  WS↔SSE bridges.
- First dependency outside the existing tree — isolated behind its own
  feature so it stays opt-in.

### v2.4 — Server capabilities ✅ shipped

- **Unix domain sockets** (`Server::bind_unix`) alongside TCP.
- **Multiple listeners** served concurrently with one shared shutdown/drain.
- **Fine-grained timeouts**: header read timeout, keep-alive tuning, per-
  connection socket options.
- **`run_listener`** — accept any listener (test-friendly, fd-passing
  friendly).

### v2.5 — TLS + HTTP/2 (feature `tls`) ✅ shipped

- **rustls** acceptor with a minimal certificate surface.
- Switch to hyper-util's `auto` connection builder: **ALPN negotiates
  HTTP/1.1 ↔ HTTP/2** with zero application changes (the 2.0 `Body` already
  satisfies h2 requirements).
- Note: WebSocket-over-h2 needs the extended-CONNECT flow — documented as
  h1-only until then.

### v3.0 — Breaking convergence

- **MSRV** moves to the stable released ~2 years prior; edition re-evaluated.
- Removal of everything deprecated during 2.x.
- **Extractor decision point** — one of:
  1. keep the named-`async fn`-returning-`Result` pattern as the permanent
     API (current recommendation; zero macros),
  2. a limited extractor trait (no macros) for `State<T>` / path tuples,
  3. a `#[handler]` macro (breaks the no-macros principle — requires strong
     justification).
- Public-API audit: de-`pub` internal types (`Svc`, `dispatch`), plus any
  accumulated small renames.
- Request-side streaming evaluation, only if real demand exists.

### Dependency policy for all of 2.x

Default features stay dependency-light (current: 24 crates). Every new
capability ships behind a cargo feature; each gated dependency must be a
mature, widely-audited crate.

## Deferred (still conscious trade-offs)

| Item | Status |
|------|--------|
| Function-parameter extractors (axum-style) | v3.0 decision point, see above |
| Framework-level fix for `?` in anonymous closures | Rust inference limitation; documented named-fn guidance stands |
| Strong content-hash ETags by default | Optional in v2.2; weak validator remains default |

## Principles

These constraints shaped every release and should shape future ones:

1. **Default builds stay dependency-light.** New capabilities ship behind cargo features; ungated additions must already be in the dependency tree.
2. **No macros** for handlers; plain functions and closures.
3. **Correct HTTP semantics first**: status codes, `Allow`, HEAD, conditional requests, and not leaking internals.
4. **Behavior changes are documented** in the CHANGELOG, including internal-type changes.
5. **End-to-end tests over real sockets** for anything behavioral; unit tests for pure logic.
