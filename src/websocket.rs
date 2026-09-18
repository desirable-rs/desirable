//! WebSocket upgrade support.
//!
//! Enabled by the `websocket` cargo feature (backed by `tokio-tungstenite`,
//! no TLS — terminate WSS at a reverse proxy).
//!
//! # Example
//!
//! ```rust,ignore
//! use desirable::{Router, WebSocketConn};
//! use desirable::websocket::Message;
//!
//! let mut app = Router::new();
//! app.websocket("/ws", |mut conn: WebSocketConn| async move {
//!     while let Some(Ok(msg)) = conn.recv().await {
//!         if let Message::Text(text) = msg {
//!             conn.send_text(text.to_string()).await.unwrap();
//!         }
//!     }
//! });
//! ```

use crate::body::Body;
use crate::{Request, Response, Result};
use futures_util::{SinkExt as _, StreamExt as _};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::Error as WsError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_util::task::TaskTracker;

/// A WebSocket message (re-exported from `tungstenite`).
pub use tokio_tungstenite::tungstenite::Message;

/// An established WebSocket connection.
///
/// Returned to the handler registered via [`Router::websocket`](crate::Router::websocket).
/// Dropping the connection closes the underlying stream.
pub struct WebSocketConn {
  inner: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
}

impl WebSocketConn {
  fn new(inner: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>) -> Self {
    Self { inner }
  }

  /// Receives the next message, or `None` when the peer closed the
  /// connection.
  pub async fn recv(&mut self) -> Option<std::result::Result<Message, WsError>> {
    self.inner.next().await
  }

  /// Sends a message.
  pub async fn send(&mut self, message: Message) -> std::result::Result<(), WsError> {
    self.inner.send(message).await
  }

  /// Sends a text message.
  pub async fn send_text(&mut self, text: impl Into<String>) -> std::result::Result<(), WsError> {
    self.send(Message::from(text.into())).await
  }

  /// Completes the closing handshake.
  ///
  /// Call this after receiving a `Close` message: `tungstenite` has already
  /// queued the reply, and this flushes it before the stream is dropped.
  pub async fn close(&mut self) -> std::result::Result<(), WsError> {
    self.inner.flush().await
  }
}

/// A validated WebSocket upgrade request.
///
/// Produced by [`WebSocketUpgrade::from_request`]; consumed by
/// [`WebSocketUpgrade::on_upgrade`], which replies `101 Switching Protocols`
/// and hands the established connection to the callback.
pub struct WebSocketUpgrade {
  key: String,
}

impl WebSocketUpgrade {
  /// Validates a request as a WebSocket handshake.
  ///
  /// Returns a `400 Bad Request` response when the required headers
  /// (`Upgrade: websocket`, `Sec-WebSocket-Key`, `Sec-WebSocket-Version: 13`)
  /// are missing or wrong.
  pub fn from_request(req: &Request) -> std::result::Result<Self, Box<Response>> {
    let bad_request = || {
      Box::new(Response::with_status_code(
        hyper::StatusCode::BAD_REQUEST,
        "expected a websocket upgrade".to_string(),
      ))
    };
    let is_websocket = req
      .header("upgrade")
      .and_then(|v| v.to_str().ok())
      .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let version_ok = req
      .header("sec-websocket-version")
      .and_then(|v| v.to_str().ok())
      .is_some_and(|v| v.trim() == "13");
    let Some(key) = req
      .header("sec-websocket-key")
      .and_then(|v| v.to_str().ok())
      .map(str::trim)
      .filter(|k| !k.is_empty())
    else {
      return Err(bad_request());
    };
    if !is_websocket || !version_ok {
      return Err(bad_request());
    }
    Ok(Self {
      key: key.to_string(),
    })
  }

  /// Replies `101 Switching Protocols` and runs `callback` with the
  /// established connection once the HTTP upgrade completes.
  ///
  /// The callback runs on a spawned task; this method returns immediately
  /// with the handshake response.
  pub fn on_upgrade<F, Fut>(self, req: Request, callback: F) -> Result<Response>
  where
    F: FnOnce(WebSocketConn) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    let accept = derive_accept_key(self.key.as_bytes());
    // Grab the connection task tracker (if the server provided one) before
    // moving the request into the session future.
    let tracker = req.extensions().get::<Arc<TaskTracker>>().cloned();
    let mut inner = req.inner;
    let ws_fut = async move {
      let Ok(upgraded) = hyper::upgrade::on(&mut inner).await else {
        // The request or connection was aborted before the handshake
        // completed; nothing to do.
        return;
      };
      let ws = WebSocketStream::from_raw_socket(
        TokioIo::new(upgraded),
        Role::Server,
        Some(WebSocketConfig::default()),
      )
      .await;
      callback(WebSocketConn::new(ws)).await;
    };

    // Run the session on the connection TaskTracker when one is available
    // (graceful shutdown then waits for live WebSocket sessions).
    if let Some(tracker) = tracker {
      tracker.spawn(ws_fut);
    } else {
      tokio::spawn(ws_fut);
    }

    let response: hyper::Response<Body> = hyper::Response::builder()
      .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
      .header(hyper::header::CONNECTION, "upgrade")
      .header(hyper::header::UPGRADE, "websocket")
      .header("Sec-WebSocket-Accept", accept)
      .body(Body::empty())
      .expect("static 101 response cannot fail to build");
    Ok(response.into())
  }
}
