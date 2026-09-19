//! Cookie sessions in three routes: login, whoami, logout.
//!
//! Run: `cargo run -p example-sessions`, then
//!
//! ```sh
//! curl -i -c jar.txt -X POST http://127.0.0.1:3000/login   # sets the cookie
//! curl -i -b jar.txt http://127.0.0.1:3000/me               # who are you?
//! curl -i -b jar.txt -X POST http://127.0.0.1:3000/logout   # deletes it
//! ```
//!
//! The `Set-Cookie` header is emitted automatically — and only when the
//! session was actually modified. The cookie value is HMAC-SHA256 signed;
//! tampered cookies are replaced with a fresh session, not an error.

use desirable::{Request, Response, Router, SessionConfig, SessionLayer, SessionManager};

#[tokio::main]
async fn main() -> desirable::Result<()> {
  // 32+ bytes; load from a secret store in production, not from source.
  let config = SessionConfig::new(b"an-example-signing-key-of-32-bytes!!").max_age_secs(3600);
  let manager = SessionManager::new(config);

  let mut app = Router::new();
  app.with(SessionLayer::new(manager));

  app.post("/login", |req: Request| async move {
    // Any serializable value works; the signed cookie carries it all.
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
    // Emits a Max-Age=0 deletion cookie.
    Ok::<_, desirable::Error>(Response::builder().text("logged out"))
  });

  desirable::new("127.0.0.1:3000").run(app).await
}
