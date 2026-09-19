//! End-to-end tests for multipart/form-data uploads (feature `multipart`):
//! streaming fields to disk, 415 on wrong content type, 400 on a missing
//! boundary, and 413 when the body limit trips mid-parse.

#![cfg(feature = "multipart")]

use desirable::{Request, Response, Router, test::TestServer};

const BOUNDARY: &str = "X-DESERVABLE-BOUNDARY";

/// Builds a multipart body with a text field and a file field.
fn multipart_body(file_content: &str) -> String {
  format!(
    "--{BOUNDARY}\r\n\
     Content-Disposition: form-data; name=\"note\"\r\n\
     \r\n\
     hello multipart\r\n\
     --{BOUNDARY}\r\n\
     Content-Disposition: form-data; name=\"file\"; filename=\"data.txt\"\r\n\
     Content-Type: text/plain\r\n\
     \r\n\
     {file_content}\r\n\
     --{BOUNDARY}--\r\n"
  )
}

fn app() -> Router {
  let mut app = Router::new();
  app.post("/upload", |mut req: Request| async move {
    let mut mp = req.multipart()?;
    let mut note = String::new();
    let mut file_len = 0usize;
    let mut file_name = String::new();
    let mut chunks = 0usize;
    while let Some(mut field) = mp.next_field().await? {
      match field.name() {
        Some("note") => note = field.text().await?,
        Some("file") => {
          file_name = field.file_name().unwrap_or_default().to_string();
          while let Some(chunk) = field.chunk().await? {
            file_len += chunk.len();
            chunks += 1;
          }
        }
        _ => {}
      }
    }
    Ok::<_, desirable::Error>(
      Response::builder().text(format!("{note}|{file_name}|{file_len}|{chunks}")),
    )
  });
  app
}

async fn post_multipart(
  server: &TestServer,
  body: String,
  content_type: &str,
) -> desirable::test::TestResponse {
  server
    .request_with(
      "POST",
      "/upload",
      &[("Content-Type", content_type)],
      Some(bytes::Bytes::from(body)),
    )
    .await
}

#[tokio::test]
async fn multipart_fields_stream_through() {
  let server = TestServer::spawn(app()).await;

  // 300 KB file content — multer delivers it in multiple chunks.
  let file_content = "abcdefghij".repeat(30_000);
  let body = multipart_body(&file_content);
  let content_type = format!("multipart/form-data; boundary={BOUNDARY}");

  let res = post_multipart(&server, body, &content_type).await;
  assert_eq!(res.status(), hyper::StatusCode::OK, "got: {}", res.text());
  let text = res.text();
  let parts: Vec<&str> = text.split('|').collect();
  assert_eq!(parts[0], "hello multipart");
  assert_eq!(parts[1], "data.txt");
  assert_eq!(parts[2], file_content.len().to_string());
  assert!(
    parts[3].parse::<usize>().unwrap() > 1,
    "300 KB must arrive as multiple chunks, got {}",
    parts[3]
  );

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn multipart_wrong_content_type_is_415() {
  let server = TestServer::spawn(app()).await;
  let res = post_multipart(&server, "not multipart".into(), "text/plain").await;
  assert_eq!(res.status(), hyper::StatusCode::UNSUPPORTED_MEDIA_TYPE);
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn multipart_missing_boundary_is_400() {
  let server = TestServer::spawn(app()).await;
  let res = post_multipart(&server, multipart_body("x"), "multipart/form-data").await;
  assert_eq!(res.status(), hyper::StatusCode::BAD_REQUEST);
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn multipart_body_limit_trips_mid_parse() {
  // 1 KB limit against a 32 KB file field: the parse aborts with 413.
  let server = TestServer::spawn_with(app(), |s| s.body_limit(1024)).await;
  let body = multipart_body(&"y".repeat(32 * 1024));
  let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
  let res = post_multipart(&server, body, &content_type).await;
  assert_eq!(res.status(), hyper::StatusCode::PAYLOAD_TOO_LARGE);
  server.shutdown().await.unwrap();
}
