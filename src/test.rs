//! Testing helpers: run a [`Router`] on an ephemeral port and talk real
//! HTTP to it — no new dependencies, no raw-socket parsing.
//!
//! The module lives behind nothing: it uses machinery the framework already
//! depends on (tokio, hyper, hyper-util), so it costs nothing until used.
//!
//! # Example
//!
//! ```rust,ignore
//! use desirable::{Router, test::TestServer};
//!
//! #[tokio::test]
//! async fn hello_world() {
//!   let mut app = Router::new();
//!   app.get("/hello", |_| async { "hi" });
//!
//!   let server = TestServer::spawn(app).await;
//!   let res = server.get("/hello").await;
//!   assert_eq!(res.status(), hyper::StatusCode::OK);
//!   assert_eq!(res.text(), "hi");
//! }
//! ```

use crate::Router;
use bytes::Bytes;
use http_body_util::Full;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;

/// A running server on an ephemeral port, for integration tests.
///
/// Dropping it detaches the server task; prefer [`TestServer::shutdown`]
/// to wait for a clean drain.
pub struct TestServer {
  addr: SocketAddr,
  shutdown: CancellationToken,
  task: tokio::task::JoinHandle<crate::Result<()>>,
}

impl TestServer {
  /// Spawns `router` on `127.0.0.1:<ephemeral>` and waits until the port
  /// accepts connections.
  pub async fn spawn(router: Router) -> Self {
    Self::spawn_with(router, |server| server).await
  }

  /// Like [`TestServer::spawn`], but lets the test configure the
  /// [`Server`](crate::Server) (body limit, connection cap, ...) before it
  /// binds.
  pub async fn spawn_with(
    router: Router,
    configure: impl FnOnce(crate::Server) -> crate::Server,
  ) -> Self {
    // Reserve a free port, release it, then hand it to the server.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let shutdown = CancellationToken::new();
    let server = configure(
      crate::Server::try_bind(&addr.to_string()).expect("ephemeral port re-bind cannot fail"),
    );
    let token = shutdown.clone();
    let task = tokio::spawn(async move {
      server
        .run_with_shutdown(router, async move { token.cancelled().await })
        .await
    });

    // Wait until the port accepts connections instead of guessing a delay.
    for _ in 0..100 {
      if tokio::net::TcpStream::connect(addr).await.is_ok() {
        return Self {
          addr,
          shutdown,
          task,
        };
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("test server did not start listening within 1s");
  }

  /// The address the server is listening on.
  pub fn addr(&self) -> SocketAddr {
    self.addr
  }

  /// Absolute URL for `path` on this server.
  pub fn url(&self, path: &str) -> String {
    format!("http://{}{}", self.addr, path)
  }

  /// Sends a GET and returns the response.
  pub async fn get(&self, path: &str) -> TestResponse {
    self.request("GET", path, None).await
  }

  /// Sends a POST with a body and returns the response.
  pub async fn post(&self, path: &str, body: impl Into<Bytes>) -> TestResponse {
    self.request("POST", path, Some(body.into())).await
  }

  /// Sends `method` to `path` with an optional body and returns the
  /// response. Each call opens its own connection (no keep-alive bookkeeping
  /// in tests).
  pub async fn request(&self, method: &str, path: &str, body: Option<Bytes>) -> TestResponse {
    use http_body_util::BodyExt as _;

    let stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
      .await
      .expect("test client handshake cannot fail on a fresh connection");
    tokio::spawn(async move {
      let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
      .method(method)
      .uri(self.url(path))
      .header(hyper::header::CONNECTION, "close");
    if body.is_some() {
      builder = builder.header(hyper::header::CONTENT_TYPE, "application/octet-stream");
    }
    let req = builder
      .body(Full::new(body.unwrap_or_default()))
      .expect("static test request cannot fail to build");

    let res = sender
      .send_request(req)
      .await
      .expect("in-process test server always answers");
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    TestResponse {
      status,
      headers,
      body,
    }
  }

  /// Triggers graceful shutdown and waits for the server task to finish.
  pub async fn shutdown(self) -> crate::Result<()> {
    self.shutdown.cancel();
    self
      .task
      .await
      .expect("server task must not panic during shutdown")
  }
}

/// A response from a [`TestServer`].
pub struct TestResponse {
  status: hyper::StatusCode,
  headers: hyper::HeaderMap,
  body: Bytes,
}

impl TestResponse {
  /// The response status code.
  pub fn status(&self) -> hyper::StatusCode {
    self.status
  }

  /// The first value of the named response header.
  pub fn header(&self, name: &str) -> Option<&hyper::header::HeaderValue> {
    self.headers.get(name)
  }

  /// The response body.
  pub fn bytes(self) -> Bytes {
    self.body
  }

  /// The response body as UTF-8 (lossy).
  pub fn text(self) -> String {
    String::from_utf8_lossy(&self.body).into_owned()
  }

  /// Deserializes the response body as JSON.
  ///
  /// # Panics
  ///
  /// Panics if the body is not the expected JSON shape — acceptable in a
  /// test assertion.
  pub fn json<T: serde::de::DeserializeOwned>(&self) -> T {
    serde_json::from_slice(&self.body).expect("test response body must be the expected JSON")
  }
}

/// Sends a raw HTTP/1.1 request string over a fresh connection and returns
/// the full response as text — for exercising malformed input, exact header
/// bytes, and other cases a client abstraction would sanitize.
pub async fn raw_request(addr: SocketAddr, request: &str) -> String {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut buf = Vec::new();
  // Connection: close in the request makes read_to_end terminate.
  stream.read_to_end(&mut buf).await.unwrap();
  String::from_utf8(buf).expect("response is utf-8")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Router;

  #[tokio::test]
  async fn test_server_get_roundtrip() {
    let mut app = Router::new();
    app.get("/hello", |_| async { "hi there" });

    let server = TestServer::spawn(app).await;
    let res = server.get("/hello").await;
    assert_eq!(res.status(), hyper::StatusCode::OK);
    assert_eq!(res.text(), "hi there");
    server.shutdown().await.unwrap();
  }

  #[tokio::test]
  async fn test_server_post_body() {
    use serde_json::json;

    let mut app = Router::new();
    app.post("/echo", |mut req: crate::Request| async move {
      let value: serde_json::Value = req.body().await?;
      Ok::<_, crate::Error>(crate::Response::json(value))
    });

    let server = TestServer::spawn(app).await;
    let res = server
      .post("/echo", json!({"msg": "round trip"}).to_string())
      .await;
    assert_eq!(res.status(), hyper::StatusCode::OK);
    assert_eq!(
      res.json::<serde_json::Value>(),
      json!({"msg": "round trip"})
    );
    server.shutdown().await.unwrap();
  }

  #[tokio::test]
  async fn test_server_404() {
    let app = Router::new();
    let server = TestServer::spawn(app).await;
    let res = server.get("/nope").await;
    assert_eq!(res.status(), hyper::StatusCode::NOT_FOUND);
    server.shutdown().await.unwrap();
  }
}
