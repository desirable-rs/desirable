# desirable

> desirable is a minimal and pragmatic Rust web application framework.

[![Crates.io](https://img.shields.io/crates/v/desirable.svg)](https://crates.io/crates/desirable)
[![Documentation](https://docs.rs/desirable/badge.svg)](https://docs.rs/desirable)
[![Build](https://github.com/desirable-rs/desirable/actions/workflows/rust.yml/badge.svg)](https://github.com/desirable-rs/desirable/actions)
[![License](https://img.shields.io/crates/l/desirable.svg)](https://github.com/desirable-rs/desirable#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue)](https://github.com/desirable-rs/desirable#install)

Built on [hyper](https://github.com/hyperium/hyper) and [tokio](https://github.com/tokio-rs/tokio), desirable keeps the concepts you already know — plain functions as handlers, one-line middleware, typed state — and skips the rest. No macros, no extractor generics, no tower ecosystem required.

## Quick start

```toml
[dependencies]
desirable = "3"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

```rust
use desirable::{Router, Request, Result};

#[derive(serde::Serialize)]
struct User { id: i32, name: String }

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = Router::new().with_state(vec![
        User { id: 1, name: "Alice".into() },
    ]);

    app.get("/", |_| async { "Hello, World!" });
    app.get("/users/:id", |req: Request| async move {
        let users = req.state::<Vec<User>>().unwrap();
        let id: i32 = req.param("id").unwrap_or(1);
        let user = &users[id as usize - 1];
        desirable::Response::json(user)
    });

    desirable::new("127.0.0.1:3000").run(app).await
}
```

## Highlights

**Handlers** — plain functions or closures. To use `?` in a handler, write it
as a named `async fn` returning `desirable::Result`; the concrete return type
lets inference do the rest (anonymous `async move` blocks need a
`Result`-turbofish):

```rust,ignore
async fn get_user(req: Request) -> Result {
    let id: i32 = req.param("id")?;
    let user = load_user(id).await?;
    Ok(desirable::Response::json(user))
}
app.get("/users/:id", |req: Request| get_user(req));
```

**Routing** — path parameters, all HTTP methods, trailing-slash tolerance, correct `404`/`405 + Allow` semantics. Static paths match in O(1) (exact-match index in front of the pattern tables); explicit HEAD routes are honored, and HEAD falls back to GET routes otherwise:

```rust,ignore
app.get("/users/:id", handler);        // GET /users/42 → 200
app.post("/users", handler);           // GET /users/   → 200 (trailing slash tolerated)
                                       // DELETE /users → 405 + Allow: GET
app.head("/health", handler);          // explicit HEAD; HEAD /users → GET's headers, no body
```

**Built-in middleware** — one line each (gzip compression behind the
optional `compression` feature; everything else dependency-free):

```rust,ignore
app.with(desirable::Logger);                    // GET /users → 200 3ms (tracing)
app.with(desirable::Cors::new());               // CORS, builder-configurable
app.with(desirable::Timeout::new(Duration::from_secs(30)));   // → 408
app.with(desirable::BodyLimit::new(1024 * 1024));             // → 413
app.with(desirable::RateLimit::per_second(100));              // → 429 + Retry-After
app.with(desirable::RequestId);                  // X-Request-Id on every response
// feature = "compression":
app.with(desirable::Compression::new());          // gzip eligible responses
```

**Typed state** — share DB pools and config without generics:

```rust,ignore
let app = Router::new().with_state(DbPool::new());
// in handlers:
let db = req.state::<DbPool>()?;
```

**Static files** — `ServeFile` / `ServeDir` with extension-based `Content-Type`, directory `index.html` fallback, path-traversal protection, conditional requests (`ETag`, `Last-Modified`, `304`), streaming responses with exact `Content-Length`, precompressed `.gz`/`.br` sibling serving, and `Cache-Control` configuration:

```rust,ignore
app.get("/static/*file",
    desirable::ServeDir::new(dir)
        .precompressed(true)
        .cache_control("public, max-age=31536000, immutable"));
```

**Streaming bodies** — buffered or streamed, one body type. `Body::channel`
makes server-sent events a few lines (the response ends when the sender drops):

```rust,ignore
app.get("/events", |_| async {
    let (sender, body) = desirable::Body::channel(16);
    tokio::spawn(async move {
        for i in 0.. {
            if sender.send(format!("data: tick {i}\n\n")).await.is_err() {
                break; // client went away
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    desirable::Response::builder().text(body)
});
```

**WebSocket** (feature `websocket`) — handshake, upgrade, and connection
wrapper handled by the framework:

```rust,ignore
// features = ["websocket"]
use desirable::{Router, WebSocketConn};
use desirable::websocket::Message;

let mut app = Router::new();
app.websocket("/ws", |mut conn: WebSocketConn| async move {
    while let Some(Ok(Message::Text(text))) = conn.recv().await {
        conn.send_text(text.to_string()).await.unwrap(); // echo
    }
});
```

**TLS + HTTP/2** (feature `tls`) — rustls-based, ALPN negotiates h2/h1:

```rust,ignore
let config = desirable::tls::server_config_from_pem(&cert_pem, &key_pem)?;
desirable::Server::try_bind("0.0.0.0:443")?
    .tls_config(Arc::new(config))
    .run(app)
    .await?;
```

**Sessions** — one line to enable; handlers mutate `req.session()` and the
`Set-Cookie` header is emitted automatically, only when the session changed
(`session.destroy()` emits a deletion cookie for logout).
Cookies are HMAC-SHA256 signed (tamper-proof), configurable via
`SessionConfig::new(key).secure(true).max_age_secs(86400)`:

```rust,ignore
app.with(SessionLayer::new(SessionManager::new(config)));

app.post("/login", |req: Request| async move {
    req.session().lock().unwrap().insert("user_id", 42)?;
    Ok::<_, desirable::Error>("logged in".into())
});
```

**Trusted proxies** — behind nginx/ALB, declare which peers may set `X-Forwarded-For`; the rightmost non-trusted address becomes `req.client_ip()` (also the rate-limit key). Off by default — a direct client's spoofed header is ignored:

```rust,ignore
desirable::Server::try_bind("0.0.0.0:3000")?
    .trusted_proxies(["10.0.0.0/8", "172.17.0.1"])
    .run(app)
    .await?;
// in handlers:
let ip = req.client_ip();
```

**Sensible errors** — client mistakes map to `400`, oversized bodies to `413`; `5xx` bodies never leak internals (they're logged instead). Render all errors your way with `set_error_handler(|err| ...)`.

**Safe by default** — request bodies are capped at 2 MiB (`413` beyond), so one huge upload can't exhaust memory. Raise or lower it per server (`server.body_limit(16 * 1024 * 1024)`, `server.no_body_limit()`) or per route with the `BodyLimit` middleware, which overrides the server default. Optionally cap concurrent connections (`server.max_connections(10_000)` — overflow is closed fail-fast). Every request is also wrapped in a `tracing` span carrying `http.method`/`http.path`/`http.request_id`, so handler logs are auto-correlated.

**Graceful shutdown** — Ctrl+C **and** SIGTERM stop the accept loop, let in-flight requests finish (configurable drain timeout, default 10s), then exit.

```rust,ignore
server.run(app).await?;                    // graceful on Ctrl+C / SIGTERM
server.run_with_shutdown(app, my_signal).await?;   // programmable
```

**Server capabilities** — Unix domain sockets, pre-bound listeners (systemd
socket activation, multi-listener deployments with a shared shutdown), and
slow-client protection:

```rust,ignore
desirable::Server::bind_unix("/tmp/app.sock")           // UDS instead of TCP
    .http1_header_read_timeout(Duration::from_secs(10)) // slow-loris guard
    .max_connections(10_000)                            // fail-fast cap
    .run(app).await?;

server.run_tcp_listener(app, listener).await?;          // pre-bound listener
```

**Testing** — `desirable::test` spawns your router on an ephemeral port and
speaks real HTTP to it; zero extra dependencies:

```rust,ignore
#[tokio::test]
async fn hello_world() {
    let mut app = desirable::Router::new();
    app.get("/hello", |_| async { "hi" });

    let server = desirable::test::TestServer::spawn(app).await;
    let res = server.get("/hello").await;
    assert_eq!(res.status(), desirable::http::StatusCode::OK);
    assert_eq!(res.text(), "hi");
}
```

## Install

Requires Rust **1.88** or newer (`rust-version = "1.88"`; let-chains are used
throughout).

```toml
[dependencies]
desirable = "3"
```

## Documentation

- [docs.rs/desirable](https://docs.rs/desirable) — full API reference
- [CHANGELOG](CHANGELOG.md) — release notes for every version
- [ROADMAP](ROADMAP.md) — what shipped, what is deliberately deferred, and why
- [examples/](examples/) — one focused runnable demo per feature: SSE, sessions, static files, middleware tour, WebSocket, TLS

## Performance

The release profile ships with LTO, `opt-level = "z"`, and stripped binaries.
Default builds pull in 26 dependencies (31 entries with the optional
`compression`/`websocket`/`tls` feature subtrees — every convenience feature
is cargo-feature gated, and each gated dependency is mature and widely
audited). Static routes match in O(1); parameterized routes use
`route_recognizer`. `cargo bench` runs the router/response microbenchmarks
plus `middleware_overhead`, which measures the middleware chain's per-layer
cost (~38 ns/layer — the evidence behind keeping the `dyn Middleware` trait
in 3.0).

## References

This framework draws inspiration from:

- [axum](https://github.com/tokio-rs/axum)
- [tide](https://github.com/http-rs/tide)
- [tinyweb](https://github.com/zzzdong/tinyweb)
- [基于 hyper 构建一个 Web 框架](https://zhuanlan.zhihu.com/p/164920970)

## License

Licensed under [Apache-2.0](LICENSE).
