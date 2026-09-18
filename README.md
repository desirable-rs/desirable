# desirable

> desirable is a minimal and pragmatic Rust web application framework.

[![Crates.io](https://img.shields.io/crates/v/desirable.svg)](https://crates.io/crates/desirable)
[![Documentation](https://docs.rs/desirable/badge.svg)](https://docs.rs/desirable)
[![Build](https://github.com/desirable-rs/desirable/actions/workflows/rust.yml/badge.svg)](https://github.com/desirable-rs/desirable/actions)
[![License](https://img.shields.io/crates/l/desirable.svg)](https://github.com/desirable-rs/desirable#license)

Built on [hyper](https://github.com/hyperium/hyper) and [tokio](https://github.com/tokio-rs/tokio), desirable keeps the concepts you already know — plain functions as handlers, one-line middleware, typed state — and skips the rest. No macros, no extractor generics, no tower ecosystem required.

## Quick start

```toml
[dependencies]
desirable = "1.7"
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

**Routing** — path parameters, all HTTP methods, trailing-slash tolerance, correct `404`/`405 + Allow` semantics:

```rust,ignore
app.get("/users/:id", handler);        // GET /users/42 → 200
app.post("/users", handler);           // GET /users/   → 200 (trailing slash tolerated)
                                       // DELETE /users → 405 + Allow: GET
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

**Sensible errors** — client mistakes map to `400`, oversized bodies to `413`; `5xx` bodies never leak internals (they're logged instead). Render all errors your way with `set_error_handler(|err| ...)`.

**Graceful shutdown** — Ctrl+C **and** SIGTERM stop the accept loop, let in-flight requests finish (configurable drain timeout, default 10s), then exit.

```rust,ignore
server.run(app).await?;                    // graceful on Ctrl+C / SIGTERM
server.run_with_shutdown(app, my_signal).await?;   // programmable
```

## Install

```toml
[dependencies]
desirable = "1.7"
```

## Documentation

- [docs.rs/desirable](https://docs.rs/desirable) — full API reference
- [CHANGELOG](CHANGELOG.md) — release notes for every version
- [examples/](examples/) — a small application using routing, middleware, sessions, and static files

## Performance

The release profile ships with LTO, `opt-level = "z"`, and stripped binaries. Core crate dependencies: 24, zero of them added for convenience features. Run `cargo bench` for router/response microbenchmarks.

## References

This framework draws inspiration from:

- [axum](https://github.com/tokio-rs/axum)
- [tide](https://github.com/http-rs/tide)
- [tinyweb](https://github.com/zzzdong/tinyweb)
- [基于 hyper 构建一个 Web 框架](https://zhuanlan.zhihu.com/p/164920970)

## License

Licensed under [Apache-2.0](LICENSE).
