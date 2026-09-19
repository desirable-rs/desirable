//! desirable - A minimal and pragmatic Rust web application framework.
//!
//! This crate provides a lightweight HTTP server framework built on top of [hyper](https://github.com/hyperium/hyper).
//! It is inspired by [axum](https://github.com/tokio-rs/axum), [tide](https://github.com/http-rs/tide), and [tinyweb](https://github.com/zzzdong/tinyweb).
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use desirable::{Router, Result};
//! use std::env;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!   let mut router = Router::new();
//!   router.get("/", |_| async { "Hello, World!" });
//!
//!   let addr = env::args()
//!     .nth(1)
//!     .unwrap_or_else(|| "127.0.0.1:3000".to_string());
//!
//!   desirable::new(&addr).run(router).await
//! }
//! ```
//!
//! # Features
//!
//! - **Router** - HTTP method routing with path parameter support
//! - **Middleware** - composable middleware stack
//! - **Request/Response** - ergonomic HTTP types
//! - **Static Files** - file and directory serving
//! - **Async/Await** - fully asynchronous throughout

pub mod body;
pub mod cors;
pub mod error;
pub mod fs;
pub mod into_response;
pub mod kernel;
pub mod middleware;
#[cfg(feature = "multipart")]
pub mod multipart;
pub mod request;
pub mod response;
pub mod router;
pub mod server;
pub mod session;
/// Testing helpers: [`TestServer`](test::TestServer) spawns a router on an
/// ephemeral port and speaks real HTTP to it, with zero extra dependencies.
pub mod test;
#[cfg(feature = "tls")]
pub mod tls;
pub mod types;
#[cfg(feature = "websocket")]
pub mod websocket;

pub use body::{Body, BodySender, BoxError};
pub use cors::Cors;
pub use error::{Error, error_msg, invalid_param, missing_param, set_error_handler};
pub use fs::{ServeDir, ServeFile};
pub use into_response::IntoResponse;
pub use kernel::{DynEndpoint, Endpoint, Middleware, Next};
#[cfg(feature = "compression")]
pub use middleware::Compression;
pub use middleware::{
  BodyLimit, Logger, RateLimit, RequestId, RequestIdValue, SessionHandle, SessionLayer, Timeout,
};
pub use request::{Request, RequestBodyStream};
pub use response::Response;
pub use router::Router;
pub use server::Server;
pub use session::{
  MemorySessionStore, Session, SessionConfig, SessionError, SessionManager, SessionStore,
};
#[cfg(feature = "tls")]
pub use tls::server_config_from_pem;
#[cfg(feature = "websocket")]
pub use websocket::{WebSocketConn, WebSocketUpgrade};

pub use types::{AnyResult, HyperRequest, HyperResponse, Result};

/// Creates a new [`Server`] bound to the given address.
///
/// This is a convenience function for quickly setting up a server.
/// For more control over server configuration, use [`Server::bind`] directly.
///
/// # Examples
///
/// ```rust,ignore
/// use desirable::{Router, Result};
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///   let router = Router::new();
///   desirable::new("127.0.0.1:8080").run(router).await
/// }
/// ```
#[must_use]
pub fn new(addr: &str) -> Server {
  Server::bind(addr)
}

// re-export
pub use hyper::header;
pub use hyper::http;
