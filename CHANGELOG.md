# Changelog

All notable changes to the desirable web framework will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [3.1.0] - 2026-09-19

### Added

- **Focused runnable examples** — one feature per crate under `examples/`,
  all compile-verified and smoke-tested: `example-sse` (streaming via
  `Body::channel`), `example-sessions` (login/whoami/logout with auto
  `Set-Cookie`), `example-static-files` (`ServeDir` + cache/ranges),
  `example-middleware-tour` (every built-in middleware, feature `compression`),
  `example-websocket` (echo, feature `websocket`), and `example-tls`
  (rustls + ALPN h2, feature `tls`, embedding the repo's test certs).
  `examples/README.md` indexes them with run instructions.
- Fixed 4 clippy warnings in `example-hello` (enum variant naming) so the
  examples workspace is also clean under `-D warnings`.

## [3.0.1] - 2026-09-19

### Fixed

- **Documentation-only.** README refreshed for the 3.x line: version refs
  updated from `1.7` to `3`, MSRV badge + requirement (1.88), dependency
  count corrected (26 default-active), and the missing capabilities
  documented (streaming bodies / SSE via `Body::channel`, trusted proxies +
  `client_ip()`, Unix domain sockets / pre-bound listeners / header read
  timeout, O(1) static-route matching, explicit HEAD routing).
- Fixed 5 copy-paste-broken doc examples across README, crate docs, and
  rustdoc that showed zero-argument handler closures (`|| async { ... }`);
  handlers take the `Request` argument (`|_| async { ... }` — the
  `Endpoint` impl requires `Fn(Request) -> Fut`), and the SSE example is
  now compile-verified.

## [3.0.0] - 2026-09-19

The breaking-convergence release planned since 2.0. The breaking surface is
deliberately small: internal types that were never meant to be public, and
one dead constructor.

### Removed (breaking)

- **`server::Svc` is no longer public API.** It is the internal hyper
  `Service` adapter; nothing outside the crate can use it meaningfully.
- **`server::dispatch` is no longer public API.** Same reasoning — requests
  enter through `Server::run`/`Router`, never through this function.
- **`Request::mk_request` removed.** A zero-use convenience alias for
  `Request::new(request, None)`.

### Added

- **Middleware-chain boxing benchmark** (`benches/middleware_overhead.rs`):
  measures `#[async_trait]`'s per-layer boxed-future cost in the exact
  `Next::run` chain shape. Result: ~38 ns per middleware layer (0 layers:
  40 ns, 3: 153 ns, 8: 378 ns).

### Decisions recorded (ROADMAP)

- **The `Middleware`/`Endpoint` traits keep `dyn` dispatch.** The redesign
  that would remove per-layer boxing (generic axum-style layering) rewrites
  every user's middleware signature and the router's scoped-chain/`merge`
  machinery — for <0.01% of a real request's cost. Declined on evidence.
- **Extractor decision finalized: none.** The named-`async fn`-returning-
  `Result` handler pattern is the permanent API; `req.state::<T>()` /
  `req.param::<T>()` stay the way data reaches handlers. No macros.
- **MSRV stays 1.88, edition 2024** (let-chains floor the toolchain
  requirement). Nothing was deprecated during 2.x, so there is nothing
  else to remove.

---

## [2.9.0] - 2026-09-19

### Fixed

- **Explicit `Router::head()` routes are now reachable**: HEAD requests
  consult the HEAD route table (static and parameterized) before falling
  back to GET. Previously the HEAD table was never consulted at dispatch,
  so every `head()` route silently returned `405` instead of running its
  handler.

### Performance

- **O(1) static-route matching**: routes without `:param`/`*wildcard`
  segments are additionally indexed in an exact-match HashMap consulted
  before the linear pattern tables. With many routes, a static request
  costs one hash lookup instead of one pattern match per registered route;
  parameterized matching, trailing-slash tolerance, `merge`, and the 405
  `Allow` computation keep their semantics. Static routes stay visible in
  the public `routes` field.

### Changed

- **Static routes take precedence over parameterized/wildcard routes**
  regardless of registration order (axum/matchit-style). Previously the
  first-registered route won, so a `/:id` pattern registered before
  `/users/new` would capture `/users/new`. Affects only routers where a
  static path collides with a pattern registered earlier.

### Performance (static files)

- **Strong ETags hash files in 64 KiB chunks** instead of buffering the
  whole file into memory — a 1 GB asset no longer causes a 1 GB spike on
  first request.
- `client_ip()` resolution short-circuits when no trusted proxies are
  configured (the common direct-exposure deployment).

---

## [2.8.1] - 2026-09-19

### Performance

- **Encoding/content-type negotiation is allocation-free**: the
  `Accept-Encoding` scan in static-file serving (and the gzip middleware's
  negotiation and content-type checks) compares case-insensitively in
  place instead of building a lowercased copy of the header value on every
  request.

### Fixed

- CHANGELOG reference links: added the missing compare-link definitions for
  2.7.1/2.8.0, corrected the 1.0.0 date, and backfilled `v1.0.0`–`v1.2.0`
  tags + GitHub releases so every version's links resolve.

---

## [2.8.0] - 2026-09-19

### Performance

- **`Request::cookie()` scans names without allocating and parses only the
  matching cookie.** Previously every cookie in the `Cookie` header was
  parsed (two allocations each) just to find one; the scan now compares
  names zero-copy, so cost no longer grows with the number of cookies the
  client sends.
- **Static-file serving makes fewer syscalls per request**: the redundant
  `metadata()` probe before precompressed-sibling negotiation was removed
  (the directory-index flag from the open already answers it), and the
  `fstat` for the served length is skipped when no precompressed sibling
  was selected (the open-time metadata is reused).
- **`ServeDir` resolves its base directory once** (at construction, or on
  the first request if the directory did not exist yet) instead of running
  `canonicalize` on the base for every request. The per-request
  canonicalize of the *resolved* path — the actual symlink-escape check —
  is unchanged.
- **`Cache-Control` values are parsed into `HeaderValue` at builder time**
  for `ServeFile`/`ServeDir` instead of re-parsed on every response.
- **`SessionManager::make_deletion_cookie()` precomputes the constant
  cookie** on first use instead of rebuilding the cookie string on every
  destroyed session.
- **Rate-limit 429 responses are allocation-free**: static body and a
  `from_static` `Retry-After` — the throttled path is the one that gets
  hammered under load.
- **`mime_for_path` avoids the lowercase copy** for already-lowercase
  extensions (the common case).
- **Strong-ETag hashing builds its hex digest without per-byte
  `format!` allocations.**
- **Fixed-text responses skip body allocation**: a `static_text`
  constructor (zero-copy `&'static str` body) now backs the default 500,
  404/403 static-file replies, rate-limit 429, timeout 408, and CORS
  preflight 204.
- **`IntoResponse` for `Cow<'static, str>` and `Bytes` build the response
  in a single builder pass** (content type set up front) instead of
  building and then patching headers; `Body` gained the corresponding
  `From<Cow<'static, str>>` impl.

### Changed

- Invalid `Cors` and `Cache-Control` configuration values now fail fast at
  construction (`panic` with the offending value) instead of failing at
  request time.

---

## [2.7.0] - 2026-09-18

### Fixed

- **Fatal accept errors no longer orphan in-flight connections**: a listener
  failure (e.g. fd exhaustion) previously returned before the drain phase,
  killing in-flight requests. The error is now recorded, in-flight
  connections are drained, and only then is the error surfaced. Transient
  accept errors (`ConnectionAborted`, `Interrupted`, `WouldBlock`) are
  logged and retried instead of terminating the loop.
- **TLS handshakes are bounded and shutdown-aware**: a handshake may take at
  most 10 seconds and is aborted when the server is shutting down, so
  stalled handshakes can no longer pin the drain phase.
- **WebSocket sessions are tracked by graceful shutdown**: the callback task
  runs on the connection `TaskTracker`, so SIGTERM waits for live WS
  sessions (bounded by the drain timeout) instead of killing them instantly.
- **Middleware errors render as responses**: a middleware returning `Err` —
  or an `into_response` failure — is now rendered through the error pipeline
  (respecting `set_error_handler`) instead of dropping the connection with
  no response.

### Changed

- `rust-version = "1.88"` declared (required for let-chains / edition 2024).
- Jenkins pipeline now actually runs tests (`cargo test --all-features`)
  and strict clippy; CI gained the same coverage earlier.

---

## [2.7.1] - 2026-09-18

### Changed

- **Documentation-only.** Aligned docs with behavior: error status mapping
  (400/413/500), header read timeout default (30s, hyper built-in),
  rate-limit keying on the resolved client IP, and a session-layer doctest
  that referenced a non-existent constructor. No code changes.

---

## [2.6.0] - 2026-09-18

### Fixed

- **Malformed JSON/urlencoded bodies returned 500 instead of 400** (BREAKING):
  `Request::body`/`body_json`/`form`/`query`/`query_or_default` now return
  the framework `Result` with typed errors — serde failures map to
  `400 Bad Request` (`Error::Json`/`Error::Urlencoded`), oversized chunked
  bodies map to `413` (`Error::BodyTooLarge`) instead of 500. Previously the
  typed errors were erased by `anyhow` on the `AnyResult` boundary.
- **`Router::head()` routes were unreachable**: HEAD requests only looked
  up the GET table, so explicit HEAD handlers never ran and the router
  advertised `Allow: HEAD` on a 405. Explicit HEAD routes are now matched
  first (GET remains the fallback).
- **HEAD responses no longer lose `Content-Length`**: the manual body strip
  was removed — hyper already suppresses HEAD bodies while emitting the
  headers a GET would have.
- **Range/conditional handling is method-gated** (RFC 9110): `Range` on
  non-GET is ignored (full 200); matching `If-None-Match` on non-GET/HEAD
  yields `412 Precondition Failed`; `If-Modified-Since` is ignored for
  non-GET/HEAD.
- **Directory-index fallback serves the correct `Content-Type`**: previously
  `index.html` reached through a directory path was labeled
  `application/octet-stream`.
- **Middleware/`into_response` errors no longer drop the connection with no
  response**: they are rendered through the error pipeline
  (`set_error_handler` aware).

### Changed

- `ServeDir` now implements `Clone` (two routes can share one configuration).

---

## [2.5.1] - 2026-09-18

### Fixed

- **`X-Forwarded-For` spoofing** (security): `client_ip()` honored the
  forwarded chain for EVERY client — the immediate peer was never checked
  against `trusted_proxies`, so any direct client could rotate fake IPs and
  bypass `RateLimit` entirely. The chain is now honored only when the peer
  is inside a trusted network (matching the documented behavior).
- **Session cookies never expired server-side**: `max_age_secs` was only a
  browser cookie attribute. `read_session` now rejects cookies older than
  the configured max age (`SessionError::Expired`); `max_age_secs = None`
  still means no server-side expiry.
- **Static files followed symlinks**: planted symlinks (uploaded files,
  extracted archives) could serve content from outside the base directory.
  `ServeDir` now rejects both file and directory symlinks via full
  canonicalization against the base directory.
- **RateLimit bucket flush**: a full bucket map (65,536 clients) triggered a
  global `clear()` on every request, letting an attacker disable rate
  limiting for everyone. Idle buckets are now evicted first, then the least
  recently active client, keeping a hard cap on memory without resetting
  legitimate clients.

### Security

- Documented the BREACH consideration for the `Compression` middleware.

---

## [2.5.0] - 2026-09-18

### Added

- **TLS support** (feature `tls`): `Server::tls_config(Arc<ServerConfig>)`
  enables TLS on the TCP listener. Helper
  `tls::server_config_from_pem(certs_pem, key_pem)` builds a rustls config
  from PEM bytes with ALPN pre-set to negotiate HTTP/2 and HTTP/1.1
  (ring provider). WSS is expected to terminate at a reverse proxy.
- **HTTP/2 over ALPN**: the auto connection builder now serves h2 when the
  TLS handshake negotiates it — zero application changes (the 2.0 `Body`
  already satisfies h2). Both the HTTP/1.1 and HTTP/2 paths receive the
  configured timers. Note: WebSocket upgrade is served on HTTP/1.1 only
  (h2 requires the extended-CONNECT flow, future work).
- Test fixtures: self-signed CA + server certificate under `tests/certs/`.

### Dependencies

- `tls` feature adds `tokio-rustls` 0.26 (ring provider, no default
  features) and `rustls-pemfile` 2 — the first genuinely new dependency
  subtree, isolated behind the feature so default builds are unchanged.

---

## [2.4.0] - 2026-09-18

### Added

- **Unix domain socket servers** (unix): `Server::bind_unix(path)`; stale
  socket files are removed before binding. Serves like TCP, with
  `Request::client_ip()` returning `None`.
- **`Server::run_tcp_listener(router, listener)`** — serve an already-bound
  TCP listener (systemd socket activation, tests, pre-bound fd handoff).
- **Multiple listeners**: spawn one `run_with_shutdown` task per listener
  with independent shutdown triggers; connections drain per server.
- **`Server::http1_header_read_timeout(Duration)`** — protects against
  slow-loris clients that open connections and stall before sending headers.
  Also doubles as an idle keep-alive timeout.

### Breaking

- `server::dispatch` takes `Option<Arc<SocketAddr>>` for the peer address
  (Unix sockets have no IP peer); `Svc::remote_addr` likewise.
- `Server` fields are now private behind `tcp_addr()` (they were private
  before 2.2's trusted_proxies round except `addr`).

---

## [2.3.0] - 2026-09-18

### Added

- **WebSocket support** (feature `websocket`): `Router::websocket(path,
  handler)` validates the handshake, replies `101 Switching Protocols`, and
  hands the handler an established `WebSocketConn` (`recv` / `send` /
  `send_text` / `close`). Non-WebSocket requests to a websocket route
  receive `400 Bad Request`. Backed by `tokio-tungstenite` (no TLS —
  terminate WSS at a reverse proxy); the hyper `Upgraded` IO is bridged to
  tokio's IO traits for tungstenite. New optional dependencies:
  `tokio-tungstenite`, `futures-util` (both feature-gated).
- Re-exports: `desirable::websocket::Message`, `WebSocketConn`,
  `WebSocketUpgrade`.

---

## [2.2.0] - 2026-09-18

### Added

- **Range requests** for `ServeFile`/`ServeDir`: single-range
  `bytes=start-end` / `bytes=start-` / `bytes=-suffix` are served as
  `206 Partial Content` with `Content-Range` and an exact `Content-Length`
  (seek + bounded stream). Unsatisfiable ranges return `416` with
  `Content-Range: bytes */total`; multi-range and non-`bytes` units are
  ignored (full 200), as RFC 9110 permits. `If-Range` mismatches fall back
  to the full body.
- **Strong ETag option**: `ServeDir::strong_etag(true)` /
  `ServeFile::strong_etag(true)` replaces the default weak `mtime`+`size`
  validator with a SHA-256 content hash (`"<hash>-<size>"`). Computed once
  per file version and cached by path+mtime+size.
- **Trusted proxies + client IP**: `Server::trusted_proxies(["10.0.0.0/8",
  "127.0.0.1"])` — when the peer is inside a trusted network, the client IP
  is resolved by walking `X-Forwarded-For` right-to-left and taking the
  first non-trusted address. Exposed via `Request::client_ip()` and used as
  the `RateLimit` bucket key. Off by default.

### Breaking

- `server::dispatch` and the `Svc` struct gained a `trusted_proxies`
  parameter/field (they were always documented as internal, but are `pub`).
- `RateLimit` buckets are keyed on the resolved client IP (previously always
  the peer address).

---

## [2.1.0] - 2026-09-18

### Added

- **Compression middleware** (feature `compression`): `Compression::new()`
  gzips eligible buffered responses (`Accept-Encoding: gzip`, 2xx status,
  compressible content type, >= 256 bytes) and recomputes `Content-Length`.
  Streaming bodies pass through. New optional dependency: `async-compression`
  (`gzip` + `tokio` features).
- **Precompressed static assets**: `ServeDir::precompressed(true)` /
  `ServeFile::precompressed(true)` serve `.gz`/`.br` siblings (brotli
  preferred) with `Content-Encoding` and `Vary: Accept-Encoding`, keeping the
  plain file's `Content-Type` and validators.
- **`Cache-Control` for static files**: `ServeDir::cache_control(...)` /
  `ServeFile::cache_control(...)`, emitted on 200 and 304 responses.
- **`Session::destroy()`** clears the session and makes `SessionLayer` emit a
  `Max-Age=0` deletion cookie (logout). Stateless signed cookies remain
  non-revocable by design; server-side revocation needs a store (future).

### Changed

- CI now also builds and tests with `--all-features`, so feature-gated code
  is covered.

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

## [1.0.0] - 2026-01-13

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
[2.8.0]: https://github.com/desirable-rs/desirable/compare/v2.7.1...v2.8.0
[2.7.1]: https://github.com/desirable-rs/desirable/compare/v2.7.0...v2.7.1
[2.7.0]: https://github.com/desirable-rs/desirable/compare/v2.6.0...v2.7.0
[2.6.0]: https://github.com/desirable-rs/desirable/compare/v2.5.1...v2.6.0
[2.5.1]: https://github.com/desirable-rs/desirable/compare/v2.5.0...v2.5.1
[2.5.0]: https://github.com/desirable-rs/desirable/compare/v2.4.0...v2.5.0
[2.4.0]: https://github.com/desirable-rs/desirable/compare/v2.3.0...v2.4.0
[2.3.0]: https://github.com/desirable-rs/desirable/compare/v2.2.0...v2.3.0
[2.2.0]: https://github.com/desirable-rs/desirable/compare/v2.1.0...v2.2.0
[2.1.0]: https://github.com/desirable-rs/desirable/compare/v2.0.0...v2.1.0
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

[2.8.1]: https://github.com/desirable-rs/desirable/compare/v2.8.0...v2.8.1

[2.9.0]: https://github.com/desirable-rs/desirable/compare/v2.8.1...v2.9.0

[3.0.0]: https://github.com/desirable-rs/desirable/compare/v2.9.0...v3.0.0
[3.0.1]: https://github.com/desirable-rs/desirable/compare/v3.0.0...v3.0.1
[3.1.0]: https://github.com/desirable-rs/desirable/compare/v3.0.1...v3.1.0
