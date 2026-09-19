//! WebSocket echo server.
//!
//! Run: `cargo run -p example-websocket`, then connect any WebSocket client:
//!
//! ```sh
//! npx wscat -c ws://127.0.0.1:3000/ws   # or your favorite WS client
//! ```
//!
//! The handshake and HTTP upgrade are handled by the framework: a
//! non-WebSocket request to `/ws` gets 400, an upgrade request runs the
//! callback with the established connection. The connection task counts
//! toward graceful shutdown.

use desirable::websocket::Message;
use desirable::{Router, WebSocketConn};

#[tokio::main]
async fn main() -> desirable::Result<()> {
  let mut app = Router::new();

  app.websocket("/ws", |mut conn: WebSocketConn| async move {
    while let Some(Ok(msg)) = conn.recv().await {
      match msg {
        Message::Text(text) => {
          if conn.send_text(text.to_string()).await.is_err() {
            break;
          }
        }
        Message::Close(_) => break,
        _ => {}
      }
    }
  });

  desirable::new("127.0.0.1:3000").run(app).await
}
