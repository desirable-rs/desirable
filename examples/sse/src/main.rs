//! Server-sent events with `Body::channel`.
//!
//! Run: `cargo run -p example-sse`, then
//! `curl -N http://127.0.0.1:3000/events` (the -N disables curl buffering).
//!
//! The response body streams frame by frame; it ends when the sender is
//! dropped (here: when the client disconnects, which makes `send` fail).

use desirable::{Body, Response, Router};

#[tokio::main]
async fn main() -> desirable::Result<()> {
  let mut app = Router::new();

  app.get("/events", |_| async {
    let (sender, body) = Body::channel(16);
    tokio::spawn(async move {
      for i in 0.. {
        let tick = format!("data: tick {i}\n\n");
        if sender.send(tick).await.is_err() {
          break; // client went away — dropping the sender ends the body
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
      }
    });
    Response::builder().text(body)
  });

  desirable::new("127.0.0.1:3000").run(app).await
}
