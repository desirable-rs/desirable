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
//!   router.get("/", || async { "Hello, World!" });
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

pub mod cors;
pub mod error;
pub mod fs;
pub mod into_response;
pub mod kernel;
pub mod middleware;
pub mod request;
pub mod response;
pub mod router;
pub mod server;
pub mod session;
pub mod types;
pub mod utils;

pub use error::Error;
pub use fs::{ServeDir, ServeFile};
pub use into_response::IntoResponse;
pub use kernel::{DynEndpoint, Endpoint, Middleware, Next};
pub use request::Request;
pub use response::Response;
pub use router::Router;
pub use server::Server;
pub use session::{Session, SessionConfig, SessionError, SessionManager};

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
pub use hyper::body;
pub use hyper::header;
pub use hyper::http;
