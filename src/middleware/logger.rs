use crate::{Middleware, Next, Request, Result};
use tracing::info;

/// Built-in access-log middleware.
///
/// Logs every request with method, path, status code, and duration.
/// Uses the `tracing` crate at INFO level.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::middleware::Logger;
///
/// let mut router = Router::new();
/// router.with(Logger);
/// ```
///
/// # Output Format
///
/// ```text
/// GET /users/42 -> 200 3ms
/// POST /users -> 201 12ms
/// ```
pub struct Logger;

#[async_trait::async_trait]
impl Middleware for Logger {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let method = req.method().clone();
    let path = req.path().to_string();
    let start = std::time::Instant::now();

    let response = next.run(req).await;

    let duration = start.elapsed();
    match &response {
      Ok(res) => {
        info!(
          "{} {} -> {} {}ms",
          method,
          path,
          res.status().as_u16(),
          duration.as_millis()
        );
      }
      Err(e) => {
        info!(
          "{} {} -> ERR {}ms ({})",
          method,
          path,
          duration.as_millis(),
          e
        );
      }
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Router;

  #[tokio::test]
  async fn test_logger_middleware_exists() {
    let mut router = Router::new();
    router.get("/", |_| async { "ok" });
    router.with(Logger);
    assert_eq!(router.middlewares.len(), 1);
  }
}
