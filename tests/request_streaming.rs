//! End-to-end tests for request-side streaming: chunked consumption and
//! streaming uploads to disk, honoring the body limit.

use desirable::{Request, Response, Router, test::TestServer};

#[tokio::test]
async fn save_body_to_streams_to_disk_without_buffering() {
  fn app(path: std::path::PathBuf) -> Router {
    let mut app = Router::new();
    app.post("/upload", move |mut req: Request| {
      let path = path.clone();
      async move {
        let written = req.save_body_to(&path).await?;
        Ok::<_, desirable::Error>(Response::builder().text(format!("{written}")))
      }
    });
    app
  }

  let payload = "0123456789".repeat(10_000); // 100 KB
  let path = std::env::temp_dir().join(format!("desirable-stream-{}", std::process::id()));

  let server = TestServer::spawn(app(path.clone())).await;
  let res = server.post("/upload", payload.clone()).await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  assert_eq!(res.text(), payload.len().to_string());

  // The file on disk matches byte for byte.
  let stored = tokio::fs::read(&path).await.unwrap();
  assert_eq!(stored.len(), payload.len());
  assert_eq!(stored, payload.as_bytes());
  tokio::fs::remove_file(&path).await.unwrap();

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn body_stream_yields_chunks_in_order() {
  let mut app = Router::new();
  app.post("/hash", |mut req: Request| async move {
    // Consume the stream chunk by chunk and report count + total size.
    let mut stream = std::pin::pin!(req.body_stream());
    let (mut count, mut total) = (0u64, 0u64);
    loop {
      let chunk = std::future::poll_fn(|cx| {
        use futures_core::Stream as _;
        std::pin::Pin::new(&mut stream).poll_next(cx)
      })
      .await;
      match chunk {
        Some(Ok(chunk)) => {
          count += 1;
          total += chunk.len() as u64;
        }
        Some(Err(err)) => return Err(err),
        None => break,
      }
    }
    Ok::<_, desirable::Error>(Response::builder().text(format!("{count}:{total}")))
  });

  let server = TestServer::spawn(app).await;
  let payload = "abcdefghij".repeat(5_000); // 50 KB → many 8-64 KB chunks
  let res = server.post("/hash", payload.clone()).await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  let text = res.text();
  let (_, total) = text.split_once(':').unwrap();
  assert_eq!(total, payload.len().to_string());
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn body_stream_enforces_body_limit_mid_stream() {
  fn app() -> Router {
    let mut app = Router::new();
    app.post("/upload", move |mut req: Request| async move {
      let written = req.save_body_to("/tmp/should-not-complete").await?;
      Ok::<_, desirable::Error>(Response::builder().text(format!("{written}")))
    });
    app
  }

  // 4 KB body against a 1 KB server limit: the stream aborts with 413.
  let server = TestServer::spawn_with(app(), |s| s.body_limit(1024)).await;
  let res = server.post("/upload", "x".repeat(4 * 1024)).await;
  assert_eq!(res.status(), hyper::StatusCode::PAYLOAD_TOO_LARGE);
  server.shutdown().await.unwrap();
}
