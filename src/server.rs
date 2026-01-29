use crate::HyperRequest;
use crate::HyperResponse;
use crate::Result;
use crate::Router;
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

/// Internal service type for hyper integration.
///
/// Adapts the router to hyper's Service trait.
pub struct Svc {
  /// The router for handling requests
  pub router: Arc<Router>,
  /// The remote address of the client
  pub remote_addr: Arc<SocketAddr>,
}

impl Service<HyperRequest> for Svc {
  type Response = HyperResponse;
  type Error = crate::error::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response>> + Send>>;

  fn call(&self, req: HyperRequest) -> Self::Future {
    let router = self.router.clone();
    let remote_addr = self.remote_addr.clone();
    let res = async { dispatch(req, remote_addr, router).await };
    Box::pin(res)
  }
}

/// Dispatches a hyper request through the router.
///
/// This is an internal function that converts hyper types to framework types
/// and routes the request through the application router.
///
/// # Arguments
///
/// * `req` - The incoming hyper request
/// * `remote_addr` - The client's socket address
/// * `router` - The application router
///
/// # Returns
///
/// The hyper response or an error
pub async fn dispatch(
  req: HyperRequest,
  remote_addr: Arc<SocketAddr>,
  router: Arc<Router>,
) -> Result<HyperResponse> {
  let response = router.dispatch(req.into(), remote_addr).await?;
  Ok(response.inner)
}

/// The HTTP server.
///
/// Binds to a socket address and serves requests using a router.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{Router, Result};
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///   let router = Router::new();
///   router.get("/", || async { "Hello!" });
///
///   let server = desirable::new("127.0.0.1:8080");
///   server.run(router).await
/// }
/// ```
pub struct Server {
  /// The address to bind to
  addr: SocketAddr,
}

impl Server {
  /// Creates a new server bound to the given address.
  ///
  /// # Arguments
  ///
  /// * `addr` - A string representation of the socket address (e.g., "127.0.0.1:8080")
  ///
  /// # Returns
  ///
  /// A new Server instance
  ///
  /// # Panics
  ///
  /// Panics if the address string is invalid
  pub fn bind(addr: &str) -> Self {
    Server {
      addr: addr.parse().unwrap(),
    }
  }

  /// Starts the server and serves requests indefinitely.
  ///
  /// This method blocks until the server is stopped (e.g., via signal).
  ///
  /// # Arguments
  ///
  /// * `router` - The application router to handle requests
  ///
  /// # Returns
  ///
  /// `Ok(())` on normal shutdown, or an error on failure
  ///
  /// # Behavior
  ///
  /// - Binds a TCP listener to the configured address
  /// - Logs the listening address
  /// - Accepts connections in a loop
  /// - Spawns each connection as an async task
  /// - Logs connection errors without propagating them
  pub async fn run(&self, router: Router) -> Result<()> {
    let addr: SocketAddr = self.addr;
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);
    let router = Arc::new(router);
    loop {
      let router = router.clone();
      let (stream, remote_addr) = listener.accept().await?;
      let io = TokioIo::new(stream);
      let remote_addr = Arc::new(remote_addr);
      tokio::task::spawn(async move {
        if let Err(err) = http1::Builder::new()
          .serve_connection(
            io,
            Svc {
              router,
              remote_addr,
            },
          )
          .await
        {
          error!("Failed to serve connection: {:?}", err);
        }
      });
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_server_bind() {
    let server = Server::bind("127.0.0.1:8080");
    assert_eq!(server.addr.to_string(), "127.0.0.1:8080");
  }

  #[test]
  fn test_server_bind_different_ports() {
    let server1 = Server::bind("0.0.0.0:3000");
    let server2 = Server::bind("0.0.0.0:8080");
    assert_eq!(server1.addr.port(), 3000);
    assert_eq!(server2.addr.port(), 8080);
  }
}
