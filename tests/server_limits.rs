//! End-to-end tests for server-level protections introduced in 3.2.0:
//! the default request-body limit and the concurrent-connection cap.

use desirable::{BodyLimit, Request, Response, Router, test::TestServer};

/// POSTs `body` to /echo on a handler that buffers and parses the body.
async fn echo_status(server: &TestServer, body: String) -> hyper::StatusCode {
  server.post("/echo", body).await.status()
}

#[tokio::test]
async fn default_body_limit_rejects_oversized_bodies_with_413() {
  // Default server: 2 MiB limit. Send 3 MiB (of valid JSON — the limit
  // trips while buffering, before parsing matters).
  let mut app = Router::new();
  app.post("/echo", |mut req: Request| async move {
    let value: serde_json::Value = req.body().await?;
    Ok::<_, desirable::Error>(Response::builder().text(format!("{}", value.is_object())))
  });

  let server = TestServer::spawn(app).await;
  let big = serde_json::json!({ "data": "x".repeat(3 * 1024 * 1024) }).to_string();
  assert_eq!(
    echo_status(&server, big).await,
    hyper::StatusCode::PAYLOAD_TOO_LARGE
  );
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn body_limit_setter_lowers_and_raises_the_cap() {
  fn app() -> Router {
    let mut app = Router::new();
    app.post("/echo", |mut req: Request| async move {
      let _: serde_json::Value = req.body().await?;
      Ok::<_, desirable::Error>(Response::builder().text("ok"))
    });
    app
  }

  let body = serde_json::json!({ "data": "y".repeat(32) }).to_string();

  // 16-byte server limit: the ~50-byte JSON body is rejected.
  let server = TestServer::spawn_with(app(), |s| s.body_limit(16)).await;
  assert_eq!(
    echo_status(&server, body.clone()).await,
    hyper::StatusCode::PAYLOAD_TOO_LARGE
  );
  server.shutdown().await.unwrap();

  // Raised limit: the same body parses and succeeds.
  let server = TestServer::spawn_with(app(), |s| s.body_limit(64)).await;
  assert_eq!(echo_status(&server, body).await, hyper::StatusCode::OK);
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn body_limit_middleware_overrides_the_server_default() {
  // Server limit 10 bytes, middleware limit 1 MiB: the middleware's value
  // replaces the server's extension, so a ~50-byte body passes.
  let mut app = Router::new();
  app.with(BodyLimit::new(1024 * 1024));
  app.post("/echo", |mut req: Request| async move {
    let _: serde_json::Value = req.body().await?;
    Ok::<_, desirable::Error>(Response::builder().text("ok"))
  });

  let server = TestServer::spawn_with(app, |s| s.body_limit(10)).await;
  let body = serde_json::json!({ "data": "z".repeat(16) }).to_string();
  assert_eq!(echo_status(&server, body).await, hyper::StatusCode::OK);
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn max_connections_rejects_overflow_and_recovers() {
  let mut app = Router::new();
  app.get("/", |_| async { "ok" });

  let server = TestServer::spawn_with(app, |s| s.max_connections(2)).await;

  // Two idle connections occupy both permits (they are accepted and then
  // wait for a request).
  let mut holders = Vec::new();
  for _ in 0..2 {
    holders.push(tokio::net::TcpStream::connect(server.addr()).await.unwrap());
  }
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;

  // Third connection is rejected: the server closes it without a response.
  // The reset may surface on write or on read depending on timing; either
  // way the client must not receive a single response byte.
  use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
  let mut third = tokio::net::TcpStream::connect(server.addr()).await.unwrap();
  let _ = third.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
  let mut buf = Vec::new();
  let _ = third.read_to_end(&mut buf).await;
  assert!(
    buf.is_empty(),
    "overflow connection must be closed with no response, got: {buf:?}"
  );

  // Dropping the holders frees the permits; the server serves again.
  drop(holders);
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  let res = server.get("/").await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  assert_eq!(res.text(), "ok");
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn no_body_limit_restores_unbounded_buffering() {
  let mut app = Router::new();
  app.post("/echo", |mut req: Request| async move {
    let _: serde_json::Value = req.body().await?;
    Ok::<_, desirable::Error>(Response::builder().text("ok"))
  });

  // A 64 KiB body far above any test-sized limit succeeds when the limit
  // is disabled (proving the server default is really off).
  let server = TestServer::spawn_with(app, |s| s.no_body_limit()).await;
  let body = serde_json::json!({ "data": "w".repeat(64 * 1024) }).to_string();
  assert_eq!(echo_status(&server, body).await, hyper::StatusCode::OK);
  server.shutdown().await.unwrap();
}
