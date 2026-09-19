//! Every built-in middleware, one line each.
//!
//! Run: `cargo run -p example-middleware-tour`, then try:
//!
//! ```sh
//! curl -i http://127.0.0.1:3000/              # watch the request id + CORS headers
//! for i in 1 2 3 4 5 6; do curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:3000/; done
//!                                             # 6th call → 429 (rate limit)
//! curl -i -X POST --data-binary @bigfile http://127.0.0.1:3000/upload   # → 413
//! ```
//!
//! Middleware is order-dependent: `with` applies to routes registered after
//! it. Each middleware travels with its routes through `merge`.

use std::time::Duration;

use desirable::{
  BodyLimit, Cors, Logger, RateLimit, Request, RequestId, Response, Router, Timeout,
};

#[tokio::main]
async fn main() -> desirable::Result<()> {
  let mut app = Router::new().with_state(std::collections::HashMap::<String, String>::new());

  app.with(Logger); // access log: "GET / -> 200 1ms"
  app.with(RequestId); // echoes X-Request-Id on every response
  app.with(Cors::new().allow_origin("http://localhost:5173"));
  app.with(Timeout::new(Duration::from_secs(30))); // slow handler → 408
  app.with(BodyLimit::new(1024 * 1024)); // >1 MiB body → 413
  app.with(RateLimit::per_second(5)); // burst of 5, then 429 + Retry-After
  app.with(desirable::Compression::new()); // gzip eligible responses (feature)

  app.get("/", |_| async { "middleware tour" });

  app.post("/upload", |mut req: Request| async move {
    let _bytes = req.body::<serde_json::Value>().await?;
    Ok::<_, desirable::Error>(Response::builder().text("uploaded"))
  });

  desirable::new("127.0.0.1:3000").run(app).await
}
