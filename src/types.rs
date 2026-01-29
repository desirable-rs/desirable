use crate::Error;
use crate::Response;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;

/// A result type using anyhow for error handling.
///
/// This is used for operations that may fail but don't need framework-specific
/// error types, such as JSON parsing or file operations.
///
/// # Type Parameters
///
/// * `T` - The success type
///
/// # Example
///
/// ```rust,ignore
/// use desirable::AnyResult;
///
/// async fn parse_json() -> AnyResult<serde_json::Value> {
///   let data = std::fs::read_to_string("data.json")?;
///   Ok(serde_json::from_str(&data)?)
/// }
/// ```
pub type AnyResult<T> = anyhow::Result<T, anyhow::Error>;

/// The default result type for the framework.
///
/// When used without a type parameter, the success type defaults to [`Response`].
/// This is the standard return type for handlers and most framework operations.
///
/// # Type Parameters
///
/// * `T` - The success type (defaults to `Response`)
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Request, Result};
///
/// async fn handler(req: Request) -> Result {
///   Ok("Hello!".into())
/// }
/// ```
pub type Result<T = Response> = std::result::Result<T, Error>;

/// Type alias for the underlying hyper response type.
///
/// Represents a complete HTTP response with a `Full<Bytes>` body.
pub type HyperResponse = hyper::Response<Full<Bytes>>;

/// Type alias for the underlying hyper request type.
///
/// Represents an incoming HTTP request with an `Incoming` body.
pub type HyperRequest = hyper::Request<Incoming>;
