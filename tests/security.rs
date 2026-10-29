//! End-to-end tests for the security middlewares (feature-free): CSRF
//! double-submit protection and SecurityHeaders.

use desirable::{Csrf, Request, Response, Router, SecurityHeaders, test::TestServer};

fn csrf_app() -> Router {
  let mut app = Router::new();
  app.with(Csrf::new());
  app.get("/form", |_| async { "form" });
  app.post("/submit", |_| async { "submitted" });
  app
}

fn cookie_from(res: &desirable::test::TestResponse, name: &str) -> Option<String> {
  let value = res.header("set-cookie")?.to_str().ok()?;
  let pair = value.split(';').next()?.trim();
  pair.starts_with(name).then(|| pair.to_string())
}

#[tokio::test]
async fn csrf_arms_on_first_get_and_blocks_unsafe_without_token() {
  let server = TestServer::spawn(csrf_app()).await;

  // GET arms the protection: a token cookie comes back.
  let res = server.get("/form").await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  let cookie = cookie_from(&res, "csrf_token=").expect("GET must set the csrf cookie");
  let token = cookie.split_once('=').unwrap().1.to_string();
  assert_eq!(token.len(), 64);

  // POST without the header → 403 (a fresh token cookie is attached).
  let res = server.post("/submit", "").await;
  assert_eq!(res.status(), hyper::StatusCode::FORBIDDEN);
  assert!(cookie_from(&res, "csrf_token=").is_some());

  // POST with the matching header → passes.
  let res = server
    .request_with(
      "POST",
      "/submit",
      &[
        ("Cookie", cookie.as_str()),
        ("X-CSRF-Token", token.as_str()),
      ],
      Some(bytes::Bytes::new()),
    )
    .await;
  assert_eq!(res.status(), hyper::StatusCode::OK, "got: {}", res.text());

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn csrf_rejects_wrong_token_and_get_is_never_blocked() {
  let server = TestServer::spawn(csrf_app()).await;

  // A tampered token is rejected with 403 even though a cookie exists.
  let res = server
    .request_with(
      "POST",
      "/submit",
      &[("Cookie", "csrf_token=aaaa"), ("X-CSRF-Token", "bbbb")],
      Some(bytes::Bytes::new()),
    )
    .await;
  assert_eq!(res.status(), hyper::StatusCode::FORBIDDEN);

  // Mismatched-but-present cookie without header: still 403.
  let res = server
    .request_with("POST", "/submit", &[("Cookie", "csrf_token=aaaa")], None)
    .await;
  assert_eq!(res.status(), hyper::StatusCode::FORBIDDEN);

  // Safe methods never blocked, even with no cookie at all.
  let res = server.get("/form").await;
  assert_eq!(res.status(), hyper::StatusCode::OK);

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn csrf_cookies_are_readable_and_not_httponly() {
  let server = TestServer::spawn(csrf_app()).await;
  let res = server.get("/form").await;
  let raw = res.header("set-cookie").unwrap().to_str().unwrap();
  assert!(raw.starts_with("csrf_token="));
  assert!(!raw.contains("HttpOnly"), "frontend must read the token");
  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn security_headers_are_applied_to_every_response() {
  let mut app = Router::new();
  app.with(
    SecurityHeaders::new()
      .nosniff()
      .frame_options_deny()
      .referrer_policy("no-referrer")
      .hsts(31_536_000)
      .content_security_policy("default-src 'self'"),
  );
  app.get("/", |_| async { "ok" });
  app.post("/", |_| async { "ok" });

  let server = TestServer::spawn(app).await;
  let res = server.get("/").await;
  assert_eq!(res.header("x-content-type-options").unwrap(), "nosniff");
  assert_eq!(res.header("x-frame-options").unwrap(), "DENY");
  assert_eq!(res.header("referrer-policy").unwrap(), "no-referrer");
  assert_eq!(
    res.header("strict-transport-security").unwrap(),
    "max-age=31536000; includeSubDomains"
  );
  assert_eq!(
    res.header("content-security-policy").unwrap(),
    "default-src 'self'"
  );

  // Headers not configured are absent.
  let bare = TestServer::spawn(Router::new()).await;
  let res = bare.get("/").await;
  assert_eq!(res.status(), hyper::StatusCode::NOT_FOUND);
  assert!(res.header("x-frame-options").is_none());
  bare.shutdown().await.unwrap();

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_handlers_and_csrf_coexist() {
  // Both middlewares set cookies on the same response without clobbering.
  use desirable::{SessionConfig, SessionLayer, SessionManager};

  let manager = SessionManager::new(SessionConfig::new(b"coexist-key-32-bytes-0123456789!"));
  let mut app = Router::new();
  app.with(Csrf::new());
  app.with(SessionLayer::new(manager));
  app.get("/", |req: Request| async move {
    req.session().lock().unwrap().insert("k", "v")?;
    Ok::<_, desirable::Error>(Response::builder().text("ok"))
  });

  let server = TestServer::spawn(app).await;
  let res = server.get("/").await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  // Append semantics: the session cookie AND the csrf cookie both arrive.
  let set_cookies = res.header_all("set-cookie");
  assert!(
    set_cookies
      .iter()
      .any(|v| v.to_str().unwrap().starts_with("desirable_session=")),
    "session cookie missing, got: {set_cookies:?}"
  );
  assert!(
    set_cookies
      .iter()
      .any(|v| v.to_str().unwrap().starts_with("csrf_token=")),
    "csrf cookie missing, got: {set_cookies:?}"
  );
  server.shutdown().await.unwrap();
}
