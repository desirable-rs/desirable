//! End-to-end tests for router nesting, scoped middleware, and HEAD fallback.
//!
//! These tests spawn a real server on an ephemeral port and speak raw HTTP,
//! verifying behavior that unit tests cannot reach (dispatch requires a
//! hyper `Incoming` body that is only produced by a live connection).

use std::sync::atomic::{AtomicUsize, Ordering};

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
