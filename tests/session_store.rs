//! End-to-end tests for server-side sessions (`SessionStore` / store mode):
//! revocation, large session data, and direct manager round-trips.

use desirable::{
  MemorySessionStore, Request, Response, Router, SessionConfig, SessionLayer, SessionManager,
  test::TestServer,
};

const KEY: &[u8] = b"test-key-32-bytes-0123456789abcd!";

fn store_app() -> Router {
  let manager = SessionManager::new(SessionConfig::new(KEY).max_age_secs(3600))
    .with_store(MemorySessionStore::new());
  let mut app = Router::new();
  app.with(SessionLayer::new(manager));

  app.post("/login", |req: Request| async move {
    req.session().lock().unwrap().insert("user", "alice")?;
    Ok::<_, desirable::Error>(Response::builder().text("logged in"))
  });
  app.get("/me", |req: Request| async move {
    let user: Option<String> = req.session().lock().unwrap().get("user")?;
    match user {
      Some(name) => Ok::<_, desirable::Error>(Response::builder().text(format!("hi {name}"))),
      None => Ok(
        Response::builder()
          .status(desirable::http::StatusCode::NOT_FOUND)
          .text("no session"),
      ),
    }
  });
  app.post("/logout", |req: Request| async move {
    req.session().lock().unwrap().destroy();
    Ok::<_, desirable::Error>(Response::builder().text("logged out"))
  });
  app
}

fn session_cookie(res: &desirable::test::TestResponse) -> Option<String> {
  let value = res.header("set-cookie")?.to_str().ok()?;
  let pair = value.split(';').next()?.trim();
  pair
    .starts_with("desirable_session=")
    .then(|| pair.to_string())
}

#[tokio::test]
async fn store_mode_revokes_destroyed_sessions() {
  let server = TestServer::spawn(store_app()).await;

  // Login: the Set-Cookie now carries a signed session ID, not the data.
  let res = server.post("/login", "").await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  let cookie = session_cookie(&res).expect("login must set the session cookie");
  assert!(
    cookie.len() < 300,
    "store-mode cookie stays tiny: {cookie:?}"
  );

  // The session resolves.
  let res = server
    .request_with("GET", "/me", &[("Cookie", cookie.as_str())], None)
    .await;
  assert_eq!(res.status(), hyper::StatusCode::OK, "got: {}", res.text());
  assert_eq!(res.text(), "hi alice");

  // Logout destroys the session server-side.
  let res = server
    .request_with("POST", "/logout", &[("Cookie", cookie.as_str())], None)
    .await;
  assert_eq!(res.status(), hyper::StatusCode::OK);

  // THE REVOCATION TEST: replaying the pre-logout cookie must NOT restore
  // the session. In client-cookie mode this old cookie would still verify
  // (signature intact) and carry the data; in store mode the entry is gone.
  let res = server
    .request_with("GET", "/me", &[("Cookie", cookie.as_str())], None)
    .await;
  assert_eq!(
    res.status(),
    hyper::StatusCode::NOT_FOUND,
    "revoked cookie must start a fresh session"
  );

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn store_mode_supports_large_session_data() {
  let manager = SessionManager::new(SessionConfig::new(KEY)).with_store(MemorySessionStore::new());
  let mut app = Router::new();
  app.with(SessionLayer::new(manager));

  const BLOB: usize = 64 * 1024;
  app.post("/stash", |mut req: Request| async move {
    let payload: serde_json::Value = req.body().await?;
    let big = payload["big"].as_str().unwrap_or_default().to_string();
    req.session().lock().unwrap().insert("big", big)?;
    Ok::<_, desirable::Error>(Response::builder().text("stashed"))
  });
  app.get("/size", |req: Request| async move {
    let big: Option<String> = req.session().lock().unwrap().get("big")?;
    Ok::<_, desirable::Error>(
      Response::builder().text(format!("{}", big.map(|s| s.len()).unwrap_or(0))),
    )
  });

  let server = TestServer::spawn(app).await;

  let body = serde_json::json!({ "big": "x".repeat(BLOB) }).to_string();
  let res = server.post("/stash", body).await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  let cookie = session_cookie(&res).expect("stash must set the session cookie");

  // 64 KB round-trips through the store while the cookie stays small —
  // impossible in client-cookie mode (headers cap out around 4-8 KB).
  let res = server
    .request_with("GET", "/size", &[("Cookie", cookie.as_str())], None)
    .await;
  assert_eq!(res.status(), hyper::StatusCode::OK);
  assert_eq!(res.text(), BLOB.to_string());
  assert!(cookie.len() < 300, "cookie stays tiny: {}", cookie.len());

  server.shutdown().await.unwrap();
}

#[tokio::test]
async fn manager_store_roundtrip_and_revoke() {
  let manager = SessionManager::new(SessionConfig::new(KEY)).with_store(MemorySessionStore::new());

  let mut session = manager.create_session();
  session.insert("user_id", 7).unwrap();
  let header = manager.persist_session(&session).await;
  let header = header.to_str().unwrap();
  let cookie_value = header.split(';').next().unwrap().split_once('=').unwrap().1;

  // Round-trips through the store.
  let loaded = manager.read_session(cookie_value).await.unwrap().unwrap();
  let user_id: Option<i32> = loaded.get("user_id").unwrap();
  assert_eq!(user_id, Some(7));

  // Revocation: the same cookie now resolves to nothing.
  manager.revoke_session(session.id()).await;
  assert!(manager.read_session(cookie_value).await.unwrap().is_none());
}
