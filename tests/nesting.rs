//! End-to-end tests for router nesting, scoped middleware, and HEAD fallback.
//!
//! These tests spawn a real server on an ephemeral port and speak raw HTTP,
//! verifying behavior that unit tests cannot reach (dispatch requires a
//! hyper `Incoming` body that is only produced by a live connection).

use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use desirable::{Middleware, Next, Request, Result, Router};

static SCOPED_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Middleware that tags responses, proving it ran.
struct ScopedTag;

#[async_trait::async_trait]
impl Middleware for ScopedTag {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    SCOPED_CALLS.fetch_add(1, Ordering::SeqCst);
    let mut response = next.run(req).await?;
    response.set_header(
      hyper::header::HeaderName::from_static("x-scoped"),
      hyper::header::HeaderValue::from_static("yes"),
    );
    Ok(response)
  }
}

/// Spawns a server on an ephemeral port and returns its address plus the
/// server's task handle, once the listener is actually accepting connections.
async fn spawn_server(router: Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
  // Reserve a free port, release it, then hand it to the server.
  let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let handle = tokio::spawn(async move {
    desirable::new(&addr.to_string()).run(router).await.unwrap();
  });

  // Wait until the port accepts connections instead of guessing a delay.
  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      return (addr, handle);
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }
  panic!("server did not start listening within 1s");
}

/// Sends a raw HTTP request and returns the full response as bytes.
#[cfg_attr(not(feature = "compression"), allow(dead_code))]
async fn raw_request_bytes(addr: std::net::SocketAddr, request: &str) -> Vec<u8> {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut buf = Vec::new();
  stream.read_to_end(&mut buf).await.unwrap();
  buf
}

/// Sends a raw HTTP request and returns the full response text.
async fn raw_request(addr: std::net::SocketAddr, request: &str) -> String {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut buf = Vec::new();
  // Connection: close makes read_to_end terminate.
  stream.read_to_end(&mut buf).await.unwrap();
  String::from_utf8(buf).expect("response is utf-8")
}

fn get_request(path: &str) -> String {
  format!(
    "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    path
  )
}

#[tokio::test]
async fn nested_router_scopes_middleware_and_falls_back_head_to_get() {
  let mut api = Router::new().prefix("/api");
  api.with(ScopedTag);
  api.get("/users", |_| async { "users" });

  let mut app = Router::new();
  app.get("/plain", |_| async { "plain" });
  app.merge(api);

  let (addr, _server) = spawn_server(app).await;

  // Nested route: scoped middleware applied.
  let res = raw_request(addr, &get_request("/api/users")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.contains("x-scoped: yes"), "got: {}", res);
  assert_eq!(SCOPED_CALLS.load(Ordering::SeqCst), 1);

  // Parent route: scoped middleware NOT applied.
  let res = raw_request(addr, &get_request("/plain")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(!res.contains("x-scoped"), "got: {}", res);
  assert_eq!(SCOPED_CALLS.load(Ordering::SeqCst), 1);

  // HEAD falls back to the GET route; body is stripped.
  let head_req = "HEAD /api/users HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, head_req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  assert!(
    body.is_empty(),
    "HEAD body should be empty, got: {:?}",
    body
  );
  assert_eq!(SCOPED_CALLS.load(Ordering::SeqCst), 2);

  // Method mismatch on a nested path still yields 405 with Allow.
  let post_req =
    "POST /api/users HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, post_req).await;
  assert!(res.starts_with("HTTP/1.1 405"), "got: {}", res);
  assert!(
    res.to_ascii_lowercase().contains("allow: get"),
    "got: {}",
    res
  );

  // Unknown path yields 404.
  let res = raw_request(addr, &get_request("/definitely-missing")).await;
  assert!(res.starts_with("HTTP/1.1 404"), "got: {}", res);
}

#[tokio::test]
async fn head_falls_back_to_get_on_plain_router() {
  let mut app = Router::new();
  app.get("/health", |_| async { "healthy" });

  let (addr, _server) = spawn_server(app).await;

  let head_req = "HEAD /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, head_req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  assert!(
    body.is_empty(),
    "HEAD body should be empty, got: {:?}",
    body
  );
}

#[tokio::test]
async fn timeout_middleware_returns_408_for_slow_handlers() {
  use std::time::Duration;

  async fn slow_handler() -> &'static str {
    tokio::time::sleep(Duration::from_secs(2)).await;
    "finally"
  }

  let mut app = Router::new();
  app.with(desirable::Timeout::new(Duration::from_millis(50)));
  app.get("/slow", |_| slow_handler());
  app.get("/fast", |_| async { "quick" });

  let (addr, _server) = spawn_server(app).await;

  // Slow handler is aborted at the deadline.
  let res = raw_request(addr, &get_request("/slow")).await;
  assert!(res.starts_with("HTTP/1.1 408"), "got: {}", res);

  // Fast handler is unaffected.
  let res = raw_request(addr, &get_request("/fast")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("quick"), "got: {}", res);
}

#[tokio::test]
async fn body_limit_returns_413_for_oversized_bodies() {
  use std::time::Duration;

  let mut app = Router::new();
  app.with(desirable::BodyLimit::new(16));
  app.with(desirable::Timeout::new(Duration::from_secs(5)));
  app.post("/upload", |mut req: desirable::Request| async move {
    let data: serde_json::Value = req.body_json().await?;
    let response: desirable::Response = desirable::Response::json(&data);
    Ok::<desirable::Response, desirable::Error>(response)
  });

  let (addr, _server) = spawn_server(app).await;

  // Declared oversized body: rejected before reading.
  let big_body = "x".repeat(64);
  let req = format!(
    "POST /upload HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
    big_body.len(),
    big_body
  );
  let res = raw_request(addr, &req).await;
  assert!(res.starts_with("HTTP/1.1 413"), "got: {}", res);

  // Small body passes.
  let req = "POST /upload HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 9\r\nConnection: close\r\n\r\n{\"a\":123}";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
}

#[tokio::test]
async fn rate_limit_returns_429_when_exhausted() {
  let mut app = Router::new();
  app.with(desirable::RateLimit::per_second(2));
  app.get("/", |_| async { "ok" });

  let (addr, _server) = spawn_server(app).await;

  // First two requests pass, third is rejected.
  let res = raw_request(addr, &get_request("/")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let res = raw_request(addr, &get_request("/")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let res = raw_request(addr, &get_request("/")).await;
  assert!(res.starts_with("HTTP/1.1 429"), "got: {}", res);
  assert!(
    res.to_ascii_lowercase().contains("retry-after:"),
    "got: {}",
    res
  );
}

#[tokio::test]
async fn static_file_supports_conditional_requests() {
  // Create a temp file to serve.
  let dir = std::env::temp_dir().join(format!("desirable-e2e-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let file_path = dir.join("app.js");
  std::fs::write(&file_path, b"console.log(1);").unwrap();

  let mut app = Router::new();
  app.get("/static/*file", desirable::ServeDir::new(dir.clone()));

  let (addr, _server) = spawn_server(app).await;

  // First request: full response with ETag and Last-Modified.
  let res = raw_request(addr, &get_request("/static/app.js")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.contains("etag: W/"), "got: {}", res);
  assert!(res.contains("last-modified:"), "got: {}", res);
  assert!(res.contains("text/javascript"), "got: {}", res);

  let etag = res
    .lines()
    .find(|l| l.to_ascii_lowercase().starts_with("etag:"))
    .and_then(|l| l.split(':').nth(1))
    .unwrap_or_default()
    .trim()
    .to_string();
  assert!(!etag.is_empty());

  // Second request with If-None-Match: 304, empty body.
  let conditional = format!(
    "GET /static/app.js HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: {}\r\nConnection: close\r\n\r\n",
    etag
  );
  let res = raw_request(addr, &conditional).await;
  assert!(res.starts_with("HTTP/1.1 304"), "got: {}", res);
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  assert!(body.is_empty(), "304 body should be empty, got: {:?}", body);

  std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn run_with_shutdown_drains_and_returns() {
  let mut app = Router::new();
  app.get("/", |_| async { "hello" });

  let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let (tx, rx) = tokio::sync::oneshot::channel::<()>();
  let server = desirable::Server::try_bind(&addr.to_string()).unwrap();
  let server_task = tokio::spawn(async move {
    server
      .run_with_shutdown(app, async move {
        let _ = rx.await;
      })
      .await
      .unwrap();
  });

  // Wait for readiness, trigger shutdown, and confirm the server returns.
  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  // Serve one request before shutting down.
  let res = raw_request(addr, &get_request("/")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);

  tx.send(()).unwrap();
  let result = tokio::time::timeout(std::time::Duration::from_secs(5), server_task).await;
  assert!(result.is_ok(), "server task should finish after shutdown");
  assert!(res.starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn typed_state_is_injected_into_handlers() {
  /// Fake "database" shared as application state.
  #[derive(Debug)]
  struct FakeDb {
    users: Vec<&'static str>,
  }

  let db = FakeDb {
    users: vec!["alice", "bob"],
  };
  let mut app = Router::new().with_state(db);
  app.get("/users/:id", |req: desirable::Request| async move {
    let db = req
      .state::<FakeDb>()
      .ok_or_else(|| desirable::error_msg("state missing"))?;
    let id: usize = req.param("id")?;
    db.users
      .get(id)
      .map(|name| (*name).to_string())
      .ok_or_else(|| desirable::error_msg("user not found"))
  });

  let (addr, _server) = spawn_server(app).await;

  let res = raw_request(addr, &get_request("/users/0")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("alice"), "got: {}", res);
}

#[tokio::test]
async fn error_handler_renders_custom_responses() {
  desirable::set_error_handler(|err| {
    desirable::Response::builder()
      .status(err.status())
      .json(serde_json::json!({
        "error": err.to_string(),
        "status": err.status().as_u16(),
      }))
  });

  let mut app = Router::new();
  app.get("/boom", |_| async {
    Err::<desirable::Response, desirable::Error>(desirable::error_msg("exploded"))
  });

  let (addr, _server) = spawn_server(app).await;

  let res = raw_request(addr, &get_request("/boom")).await;
  assert!(res.starts_with("HTTP/1.1 500"), "got: {}", res);
  assert!(res.contains("application/json"), "got: {}", res);
  assert!(
    res.contains("\"error\":\"error msg \\\"exploded\\\"\""),
    "got: {}",
    res
  );
}

#[tokio::test]
async fn trailing_slash_matches_registered_route() {
  let mut app = Router::new();
  app.get("/users", |_| async { "users" });

  let (addr, _server) = spawn_server(app).await;

  // Exact match still works.
  let res = raw_request(addr, &get_request("/users")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);

  // Trailing slash falls back to the same route.
  let res = raw_request(addr, &get_request("/users/")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("users"), "got: {}", res);
}

#[tokio::test]
async fn session_layer_roundtrip_and_tamper_handling() {
  use desirable::{Request, Result, SessionConfig, SessionLayer, SessionManager};

  async fn login(mut req: Request) -> Result {
    let user: String = req.body_json().await?;
    req.session().lock().unwrap().insert("user", user)?;
    Ok::<_, desirable::Error>("ok".into())
  }

  async fn whoami(req: Request) -> &'static str {
    let name: Option<String> = req.session().lock().unwrap().get("user").unwrap();
    if name.is_some() { "known" } else { "anonymous" }
  }

  let manager = SessionManager::new(SessionConfig::new(b"test-key-32-bytes-long!!!!!12345"));
  let mut app = Router::new();
  app.with(SessionLayer::new(manager));
  app.post("/login", |req: Request| login(req));
  app.get("/me", whoami);

  let (addr, _server) = spawn_server(app).await;

  // (d) Anonymous request: no Set-Cookie, session untouched.
  let res = raw_request(addr, &get_request("/me")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("anonymous"), "got: {}", res);
  assert!(
    !res.to_ascii_lowercase().contains("set-cookie"),
    "got: {}",
    res
  );

  // (a) Modified session: response carries Set-Cookie.
  let body = r#""alice""#;
  let login_req = format!(
    "POST /login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
    body.len(),
    body
  );
  let res = raw_request(addr, &login_req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let set_cookie = res
    .lines()
    .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
    .expect("login must set session cookie")
    .to_string();
  let cookie_pair = set_cookie
    .split(':')
    .nth(1)
    .unwrap_or_default()
    .split(';')
    .next()
    .unwrap_or_default()
    .trim()
    .to_string();
  assert!(
    cookie_pair.starts_with("desirable_session="),
    "got: {}",
    cookie_pair
  );

  // (b) Cookie roundtrip: session value readable on the next request.
  let req = format!(
    "GET /me HTTP/1.1\r\nHost: localhost\r\nCookie: {}\r\nConnection: close\r\n\r\n",
    cookie_pair
  );
  let res = raw_request(addr, &req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("known"), "got: {}", res);

  // (c) Tampered cookie: fresh session, request still succeeds.
  let req = "GET /me HTTP/1.1\r\nHost: localhost\r\nCookie: desirable_session=GARBAGEVALUE\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("anonymous"), "got: {}", res);
}

#[tokio::test]
async fn streaming_body_roundtrips_over_chunked_encoding() {
  let mut app = Router::new();
  app.get("/events", |_| async {
    let (sender, body) = desirable::Body::channel(4);
    tokio::spawn(async move {
      for i in 0..3 {
        sender
          .send(Bytes::from(format!("event-{i}\n")))
          .await
          .unwrap();
      }
      // sender dropped -> body ends
    });
    desirable::Response::builder().body(body)
  });

  let (addr, _server) = spawn_server(app).await;

  let res = raw_request(addr, &get_request("/events")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  // Raw chunked framing: size lines between chunks and a final "0" terminator.
  assert!(body.contains("event-0"), "got: {:?}", body);
  assert!(body.contains("event-2"), "got: {:?}", body);
  // Chunked size lines present ("8" per chunk); hyper may omit the final
  // CRLF pair when the connection close delimits the end.
  assert!(
    body.contains("\r\n8\r\n"),
    "missing chunked framing: {:?}",
    body
  );
  assert!(
    body.trim_end().ends_with('0'),
    "missing last-chunk marker: {:?}",
    body
  );
}

#[tokio::test]
async fn static_file_streams_with_exact_content_length() {
  let dir = std::env::temp_dir().join(format!("desirable-stream-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  // A body large enough to span several 64 KiB stream chunks.
  let big = vec![b'x'; 200 * 1024];
  std::fs::write(dir.join("big.bin"), &big).unwrap();

  let mut app = Router::new();
  app.get("/static/*file", desirable::ServeDir::new(dir.clone()));

  let (addr, _server) = spawn_server(app).await;

  let res = raw_request(addr, &get_request("/static/big.bin")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  // Exact fstat length -> Content-Length (not chunked), preserved end-to-end.
  assert!(
    res.contains(&format!("content-length: {}", big.len())),
    "got: {}",
    res
  );
  // Full body survives streaming.
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  assert_eq!(body.len(), big.len(), "streamed body must be complete");
  assert!(body.bytes().all(|b| b == b'x'));

  std::fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "compression")]
#[tokio::test]
async fn compression_gzips_eligible_responses() {
  let mut app = Router::new();
  app.with(desirable::Compression::new());
  // > 256 bytes so the size threshold passes.
  let text = "a".repeat(1024);
  app.get("/text", move |_| {
    let text = text.clone();
    async move { desirable::Response::builder().text(text) }
  });

  let (addr, _server) = spawn_server(app).await;

  // Client accepts gzip: headers say gzip and the body is real gzip data
  // (binary — parse headers separately from the payload).
  let req =
    "GET /text HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n";
  let raw = raw_request_bytes(addr, req).await;
  let split = raw
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .expect("response must contain a header/body separator");
  let (headers, body) = raw.split_at(split + 4);
  let headers = String::from_utf8(headers.to_vec()).unwrap();
  assert!(headers.starts_with("HTTP/1.1 200"), "got: {}", headers);
  assert!(
    headers
      .to_ascii_lowercase()
      .contains("content-encoding: gzip"),
    "got: {}",
    headers
  );

  // Decompress and verify the payload round-trips.
  let mut decoder = async_compression::tokio::write::GzipDecoder::new(Vec::new());
  use tokio::io::AsyncWriteExt as _;
  decoder.write_all(body).await.unwrap();
  decoder.shutdown().await.unwrap();
  assert_eq!(decoder.into_inner(), "a".repeat(1024).as_bytes());

  // Client without gzip support: plain body.
  let res = raw_request(addr, &get_request("/text")).await;
  assert!(
    !res.to_ascii_lowercase().contains("content-encoding"),
    "got: {}",
    res
  );
  assert!(res.ends_with(&"a".repeat(1024)), "got: {}", res);
}

#[tokio::test]
async fn precompressed_static_assets_are_negotiated() {
  let dir = std::env::temp_dir().join(format!("desirable-precomp-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("app.js"), b"console.log('plain');").unwrap();
  std::fs::write(dir.join("app.js.gz"), b"gzip-bytes").unwrap();

  let mut app = Router::new();
  app.get(
    "/static/*file",
    desirable::ServeDir::new(dir.clone())
      .precompressed(true)
      .cache_control("public, max-age=3600"),
  );

  let (addr, _server) = spawn_server(app).await;

  // Accepts gzip: served the .gz sibling verbatim, labeled + cached.
  let req = "GET /static/app.js HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(
    res.to_ascii_lowercase().contains("content-encoding: gzip"),
    "got: {}",
    res
  );
  assert!(
    res.to_ascii_lowercase().contains("vary: accept-encoding"),
    "got: {}",
    res
  );
  assert!(
    res.contains("cache-control: public, max-age=3600"),
    "got: {}",
    res
  );
  assert!(
    res.contains("content-type: text/javascript"),
    "got: {}",
    res
  );
  assert!(res.ends_with("gzip-bytes"), "got: {}", res);

  // No Accept-Encoding: plain file, no Content-Encoding.
  let res = raw_request(addr, &get_request("/static/app.js")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(
    !res.to_ascii_lowercase().contains("content-encoding"),
    "got: {}",
    res
  );
  assert!(res.ends_with("console.log('plain');"), "got: {}", res);

  std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn session_destroy_sends_deletion_cookie() {
  use desirable::{Request, Result, SessionConfig, SessionLayer, SessionManager};

  async fn login(mut req: Request) -> Result {
    let user: String = req.body_json().await?;
    req.session().lock().unwrap().insert("user", user)?;
    Ok::<_, desirable::Error>("ok".into())
  }

  async fn logout(req: Request) -> Result {
    req.session().lock().unwrap().destroy();
    Ok::<_, desirable::Error>("bye".into())
  }

  async fn whoami(req: Request) -> &'static str {
    let name: Option<String> = req.session().lock().unwrap().get("user").unwrap();
    if name.is_some() { "known" } else { "anonymous" }
  }

  let manager = SessionManager::new(SessionConfig::new(b"destroy-test-key-32-bytes!!!1234"));
  let mut app = Router::new();
  app.with(SessionLayer::new(manager));
  app.post("/login", |req: Request| login(req));
  app.post("/logout", |req: Request| logout(req));
  app.get("/me", whoami);

  let (addr, _server) = spawn_server(app).await;

  // Login, keep the session cookie.
  let body = "\"alice\"";
  let req = format!(
    "POST /login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
    body.len(),
    body
  );
  let res = raw_request(addr, &req).await;
  let cookie_pair = res
    .lines()
    .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
    .and_then(|l| l.split(':').nth(1))
    .and_then(|v| v.split(';').next())
    .unwrap_or_default()
    .trim()
    .to_string();

  // Session is live.
  let req = format!(
    "GET /me HTTP/1.1\r\nHost: localhost\r\nCookie: {}\r\nConnection: close\r\n\r\n",
    cookie_pair
  );
  let res = raw_request(addr, &req).await;
  assert!(res.ends_with("known"), "got: {}", res);

  // Destroy: response carries a Max-Age=0 deletion cookie.
  let req = format!(
    "POST /logout HTTP/1.1\r\nHost: localhost\r\nCookie: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    cookie_pair
  );
  let res = raw_request(addr, &req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let set_cookie = res
    .lines()
    .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
    .expect("destroy must emit a deletion cookie")
    .to_ascii_lowercase();
  assert!(set_cookie.contains("max-age=0"), "got: {}", set_cookie);

  // A compliant browser drops the cookie; the deletion cookie value
  // (empty) starts a fresh anonymous session.
  let req = "GET /me HTTP/1.1\r\nHost: localhost\r\nCookie: desirable_session=\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.ends_with("anonymous"), "got: {}", res);
  // NOTE: stateless signed cookies cannot be revoked server-side — replaying
  // the old value still verifies. Server-side revocation needs a store.
}

#[tokio::test]
async fn range_requests_serve_partial_content() {
  let dir = std::env::temp_dir().join(format!("desirable-range-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let content: Vec<u8> = (0..=255u8).collect();
  std::fs::write(dir.join("data.bin"), &content).unwrap();

  let mut app = Router::new();
  app.get(
    "/static/*file",
    desirable::ServeDir::new(dir.clone()).cache_control("public, max-age=60"),
  );

  let (addr, _server) = spawn_server(app).await;

  // bytes=0-9: first ten bytes, 206 + Content-Range.
  let req = "GET /static/data.bin HTTP/1.1\r\nHost: localhost\r\nRange: bytes=0-9\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 206"), "got: {}", res);
  assert!(res.contains("content-range: bytes 0-9/256"), "got: {}", res);
  assert!(res.contains("content-length: 10"), "got: {}", res);
  let body = res.split("\r\n\r\n").nth(1).unwrap_or("");
  assert_eq!(body.as_bytes(), &content[0..10]);

  // bytes=-4: last four bytes (binary body -> byte-level request).
  let req = "GET /static/data.bin HTTP/1.1\r\nHost: localhost\r\nRange: bytes=-4\r\nConnection: close\r\n\r\n";
  let raw = raw_request_bytes(addr, req).await;
  let split = raw
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .expect("response must contain a header/body separator");
  let (headers, body) = raw.split_at(split + 4);
  let headers = String::from_utf8(headers.to_vec()).unwrap();
  assert!(headers.starts_with("HTTP/1.1 206"), "got: {}", headers);
  assert!(
    headers.contains("content-range: bytes 252-255/256"),
    "got: {}",
    headers
  );
  assert_eq!(body, &content[252..256]);

  // bytes=300- : unsatisfiable -> 416 with bytes */256.
  let req = "GET /static/data.bin HTTP/1.1\r\nHost: localhost\r\nRange: bytes=300-\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 416"), "got: {}", res);
  assert!(res.contains("content-range: bytes */256"), "got: {}", res);

  // No Range: full 200 (binary body -> byte-level request).
  let raw = raw_request_bytes(addr, &get_request("/static/data.bin")).await;
  let split = raw
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .expect("response must contain a header/body separator");
  let (headers, body) = raw.split_at(split + 4);
  let headers = String::from_utf8(headers.to_vec()).unwrap();
  assert!(headers.starts_with("HTTP/1.1 200"), "got: {}", headers);
  assert!(
    headers.contains("cache-control: public, max-age=60"),
    "got: {}",
    headers
  );
  assert_eq!(body, &content[..]);

  std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn strong_etag_supports_conditional_requests() {
  let dir = std::env::temp_dir().join(format!("desirable-etag-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("page.html"), b"<h1>stable</h1>").unwrap();

  let mut app = Router::new();
  app.get(
    "/static/*file",
    desirable::ServeDir::new(dir.clone()).strong_etag(true),
  );

  let (addr, _server) = spawn_server(app).await;

  let res = raw_request(addr, &get_request("/static/page.html")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  let etag_line = res
    .lines()
    .find(|l| l.to_ascii_lowercase().starts_with("etag:"))
    .expect("etag must be present")
    .to_string();
  // Strong: no W/ prefix.
  assert!(!etag_line.contains("W/"), "got: {}", etag_line);

  let etag = etag_line
    .split(':')
    .nth(1)
    .unwrap_or_default()
    .trim()
    .to_string();
  let conditional = format!(
    "GET /static/page.html HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: {}\r\nConnection: close\r\n\r\n",
    etag
  );
  let res = raw_request(addr, &conditional).await;
  assert!(res.starts_with("HTTP/1.1 304"), "got: {}", res);

  std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn trusted_proxies_resolve_client_ip() {
  // Rate limit to 1 request per client IP: lets us observe which IP the
  // limiter sees (forwarded vs peer).
  let mut app = Router::new();
  app.with(desirable::RateLimit::per_second(1));
  app.get("/ip", |req: Request| async move {
    let ip = req.client_ip().map(|i| i.to_string()).unwrap_or_default();
    desirable::Response::builder().text(ip)
  });

  let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let server = desirable::Server::try_bind(&addr.to_string())
    .unwrap()
    .trusted_proxies(["127.0.0.1"]);
  let server_task = tokio::spawn(async move {
    server
      .run_with_shutdown(app, std::future::pending::<()>())
      .await
      .unwrap();
  });
  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  // Peer 127.0.0.1 is trusted: the X-Forwarded-For IP wins and the limiter
  // buckets on it (first request for 203.0.113.7 passes).
  let req = "GET /ip HTTP/1.1\r\nHost: localhost\r\nX-Forwarded-For: 203.0.113.7\r\nConnection: close\r\n\r\n";
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("203.0.113.7"), "got: {}", res);

  // Second request: rate limited on the forwarded IP.
  let res = raw_request(addr, req).await;
  assert!(res.starts_with("HTTP/1.1 429"), "got: {}", res);

  server_task.abort();
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn websocket_echo_and_bad_request_fallback() {
  use desirable::websocket::Message;
  use desirable::{Router, WebSocketConn};
  use futures_util::{SinkExt as _, StreamExt as _};

  async fn echo(mut conn: WebSocketConn) {
    while let Some(Ok(msg)) = conn.recv().await {
      if msg.is_text() || msg.is_binary() {
        if conn.send(msg).await.is_err() {
          break;
        }
      } else if msg.is_close() {
        // Flush tungstenite's automatic Close reply.
        let _ = conn.close().await;
        break;
      }
    }
  }

  let mut app = Router::new();
  app.websocket("/ws", |conn: WebSocketConn| echo(conn));
  app.get("/plain", |_| async { "plain" });

  let (addr, _server) = spawn_server(app).await;

  // Happy path: full handshake + two-way echo + close.
  let (mut ws, response) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
    .await
    .unwrap();
  assert_eq!(response.status(), hyper::StatusCode::SWITCHING_PROTOCOLS);

  ws.send(Message::text("hello desirable")).await.unwrap();
  let echoed = ws.next().await.unwrap().unwrap();
  assert!(matches!(echoed, Message::Text(ref t) if t.contains("hello desirable")));

  ws.send(Message::Close(None)).await.unwrap();
  let close = ws.next().await.unwrap().unwrap();
  assert!(close.is_close(), "expected close, got {:?}", close);

  // Non-upgrade request to the websocket route: 400.
  let res = raw_request(addr, &get_request("/ws")).await;
  assert!(res.starts_with("HTTP/1.1 400"), "got: {}", res);

  // Plain routes unaffected.
  let res = raw_request(addr, &get_request("/plain")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
}

#[cfg(unix)]
#[tokio::test]
async fn unix_socket_serves_requests() {
  let mut app = Router::new();
  app.get("/uds", |req: Request| async move {
    match req.client_ip() {
      Some(_) => desirable::Response::builder().text("tcp"),
      None => desirable::Response::builder().text("unix"),
    }
  });

  let path = std::env::temp_dir().join(format!("desirable-uds-{}.sock", std::process::id()));
  let server = desirable::Server::bind_unix(&path);
  let server_task = tokio::spawn(async move {
    server
      .run_with_shutdown(app, std::future::pending::<()>())
      .await
      .unwrap();
  });

  // Wait for the socket file to appear.
  for _ in 0..100 {
    if path.exists() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
  let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
  stream
    .write_all(b"GET /uds HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .await
    .unwrap();
  let mut buf = Vec::new();
  stream.read_to_end(&mut buf).await.unwrap();
  let res = String::from_utf8(buf).unwrap();
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("unix"), "uds has no client ip, got: {}", res);

  // Graceful shutdown works for UDS servers too (abort = immediate here;
  // the run_with_shutdown path is covered by the TCP tests).
  server_task.abort();

  std::fs::remove_file(&path).ok();
}

#[tokio::test]
async fn run_tcp_listener_serves_prebound_listener() {
  let mut app = Router::new();
  app.get("/", |_| async { "prebound" });

  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let server = desirable::Server::try_bind(&addr.to_string()).unwrap();

  let server_task = tokio::spawn(async move {
    server.run_tcp_listener(app, listener).await.unwrap();
  });

  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  let res = raw_request(addr, &get_request("/")).await;
  assert!(res.starts_with("HTTP/1.1 200"), "got: {}", res);
  assert!(res.ends_with("prebound"), "got: {}", res);

  server_task.abort();
}

#[tokio::test]
async fn multiple_listeners_share_shutdown() {
  async fn make_server() -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
  ) {
    let mut app = Router::new();
    app.get("/", |_| async { "ok" });

    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let server = desirable::Server::try_bind(&addr.to_string()).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
      server
        .run_with_shutdown(app, async move {
          let _ = rx.await;
        })
        .await
        .unwrap();
    });
    (addr, tx, task)
  }

  let (addr_a, tx_a, task_a) = make_server().await;
  let (addr_b, tx_b, task_b) = make_server().await;

  for target in [addr_a, addr_b] {
    for _ in 0..100 {
      if tokio::net::TcpStream::connect(target).await.is_ok() {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  }

  // Both serve independently.
  assert!(
    raw_request(addr_a, &get_request("/"))
      .await
      .starts_with("HTTP/1.1 200")
  );
  assert!(
    raw_request(addr_b, &get_request("/"))
      .await
      .starts_with("HTTP/1.1 200")
  );

  // Shutting down server A does NOT affect server B.
  tx_a.send(()).unwrap();
  let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task_a).await;
  let res = std::panic::AssertUnwindSafe(raw_request(addr_b, &get_request("/")).await);
  assert!(
    res.starts_with("HTTP/1.1 200"),
    "server B must keep serving"
  );

  // Then shutdown B.
  tx_b.send(()).unwrap();
  let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task_b).await;
}

#[tokio::test]
async fn header_read_timeout_closes_slow_clients() {
  use std::time::Duration;

  let mut app = Router::new();
  app.get("/", |_| async { "ok" });

  let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let server = desirable::Server::try_bind(&addr.to_string())
    .unwrap()
    .http1_header_read_timeout(Duration::from_millis(100));
  let server_task = tokio::spawn(async move {
    server
      .run_with_shutdown(app, std::future::pending::<()>())
      .await
      .unwrap();
  });

  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  // Send a partial request line, then go quiet: the server must close the
  // connection after the header read timeout instead of waiting forever.
  use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
  let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
  stream.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
  let started = std::time::Instant::now();
  let mut buf = Vec::new();
  let read = stream.read_to_end(&mut buf).await;

  assert!(read.is_ok(), "connection should close cleanly");
  let elapsed = started.elapsed();
  assert!(
    elapsed >= Duration::from_millis(50) && elapsed < Duration::from_secs(3),
    "close should happen after ~100ms, took {:?}",
    elapsed
  );
  assert!(
    buf.is_empty(),
    "no response expected, got {:?}",
    String::from_utf8_lossy(&buf)
  );

  server_task.abort();
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn tls_serves_http1_and_alpn_negotiates_h2() {
  use desirable::tls::server_config_from_pem;
  use hyper::Request;
  use hyper_util::rt::{TokioExecutor, TokioIo};
  use std::sync::Arc;

  let mut app = Router::new();
  app.get("/", |_| async { "hello over tls" });

  // Load the committed self-signed test certificate chain (leaf + CA).
  let cert_pem = std::fs::read("tests/certs/server-cert.pem").unwrap();
  let key_pem = std::fs::read("tests/certs/server-key.pem").unwrap();
  let config = Arc::new(server_config_from_pem(&cert_pem, &key_pem).unwrap());

  let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let server = desirable::Server::try_bind(&addr.to_string())
    .unwrap()
    .tls_config(config);
  let server_task = tokio::spawn(async move {
    server
      .run_with_shutdown(app, std::future::pending::<()>())
      .await
      .unwrap();
  });
  for _ in 0..100 {
    if tokio::net::TcpStream::connect(addr).await.is_ok() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }

  // Client TLS setup: trust the test CA, request ALPN h2.
  let cert_pem = std::fs::read("tests/certs/ca-cert.pem").unwrap();
  let mut roots = tokio_rustls::rustls::RootCertStore::empty();
  for cert in rustls_pemfile::certs(&mut cert_pem.as_slice()) {
    roots.add(cert.unwrap()).unwrap();
  }
  let mut client_config = tokio_rustls::rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();
  client_config.alpn_protocols = vec![b"h2".to_vec()];

  let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
  let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
  let tls_stream = connector
    .connect("localhost".try_into().unwrap(), tcp)
    .await
    .unwrap();
  // ALPN negotiated h2 with the server.
  assert_eq!(
    tls_stream.get_ref().1.alpn_protocol(),
    Some(b"h2".as_slice())
  );

  // Drive an HTTP/2 request over the negotiated connection.
  let (mut sender, conn) =
    hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls_stream))
      .await
      .unwrap();
  tokio::spawn(async move {
    let _ = conn.await;
  });

  let request = Request::builder()
    .uri("https://localhost/")
    .body(http_body_util::Empty::<Bytes>::new())
    .unwrap();
  let response = sender.send_request(request).await.unwrap();
  assert_eq!(response.status(), hyper::StatusCode::OK);
  let body = http_body_util::BodyExt::collect(response.into_body())
    .await
    .unwrap()
    .to_bytes();
  assert_eq!(&body[..], b"hello over tls");

  server_task.abort();
}
