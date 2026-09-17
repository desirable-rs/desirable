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
