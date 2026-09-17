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
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

/// Default maximum time in-flight connections are given to finish after a
/// shutdown signal before the server gives up waiting.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

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
  /// Maximum time in-flight connections are given to finish after shutdown
  drain_timeout: Duration,
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
    Server::try_bind(addr).unwrap()
  }

  /// Creates a new server from the given address, returning an error instead
  /// of panicking when the address string is invalid.
  pub fn try_bind(addr: &str) -> Result<Self> {
    let addr: SocketAddr = addr.parse()?;
    Ok(Server {
      addr,
      drain_timeout: DEFAULT_DRAIN_TIMEOUT,
    })
  }

  /// Sets the maximum time in-flight connections are given to finish after
  /// a shutdown signal before the server stops waiting.
  ///
  /// Default: 10 seconds.
  #[must_use]
  pub fn drain_timeout(mut self, timeout: Duration) -> Self {
    self.drain_timeout = timeout;
    self
  }

  /// Starts the server and serves requests until shutdown signal.
  ///
  /// Delegates to [`Server::run_graceful`]. Listens for Ctrl+C (SIGINT) and
  /// performs a graceful shutdown.
  ///
  /// # Arguments
  ///
  /// * `router` - The application router to handle requests
  ///
  /// # Returns
  ///
  /// `Ok(())` on normal shutdown, or an error on failure
  pub async fn run(&self, router: Router) -> Result<()> {
    self.run_graceful(router).await
  }

  /// Starts the server with graceful shutdown on Ctrl+C (SIGINT).
  ///
  /// On shutdown the server stops accepting new connections, signals existing
  /// connections to finish their in-flight requests, and waits (up to the
  /// configured [`Server::drain_timeout`]) for them to complete.
  pub async fn run_graceful(&self, router: Router) -> Result<()> {
    self.run_with_shutdown(router, shutdown_signal()).await
  }

  /// Starts the server and serves requests until the given `signal` future
  /// resolves, then drains connections gracefully.
  ///
  /// This is the programmable form of [`Server::run_graceful`]: use it when
  /// you need to trigger shutdown yourself (tests, orchestration, custom
  /// signals).
  ///
  /// # Arguments
  ///
  /// * `router` - The application router to handle requests
  /// * `signal` - A future whose completion triggers graceful shutdown
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let (tx, rx) = tokio::sync::oneshot::channel::<()>();
  /// server.run_with_shutdown(router, async move {
  ///   let _ = rx.await;
  /// }).await?;
  /// ```
  pub async fn run_with_shutdown(
    &self,
    router: Router,
    signal: impl Future<Output = ()>,
  ) -> Result<()> {
    let listener = TcpListener::bind(self.addr).await?;
    info!("Listening on http://{}", self.addr);

    let router = Arc::new(router);
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();

    tokio::select! {
      result = accept_loop(listener, router, &tracker, shutdown.clone()) => {
        result?;
      }
      _ = signal => {
        info!("Shutdown signal received, draining connections...");
        shutdown.cancel();
      }
    }

    tracker.close();
    match tokio::time::timeout(self.drain_timeout, tracker.wait()).await {
      Ok(()) => info!("All connections drained, shutdown complete"),
      Err(_) => warn!(
        "Drain timeout of {:?} elapsed, abandoning in-flight connections",
        self.drain_timeout
      ),
    }
    Ok(())
  }
}

/// Accepts connections in a loop until the listener is closed, an error
/// occurs, or the shutdown token is cancelled.
async fn accept_loop(
  listener: TcpListener,
  router: Arc<Router>,
  tracker: &TaskTracker,
  shutdown: CancellationToken,
) -> Result<()> {
  loop {
    tokio::select! {
      accepted = listener.accept() => {
        let (stream, remote_addr) = accepted?;
        // Reduce latency for small request/response pairs; failure is benign.
        if let Err(err) = stream.set_nodelay(true) {
          debug!("Failed to set TCP_NODELAY: {}", err);
        }
        let io = TokioIo::new(stream);
        let remote_addr = Arc::new(remote_addr);
        let router = Arc::clone(&router);
        let shutdown = shutdown.clone();
        tracker.spawn(async move {
          let conn = http1::Builder::new().serve_connection(
            io,
            Svc {
              router,
              remote_addr,
            },
          );
          tokio::pin!(conn);
          tokio::select! {
            result = &mut conn => {
              if let Err(err) = result {
                warn!("Connection error: {:?}", err);
              }
            }
            _ = shutdown.cancelled() => {
              // Stop keep-alive, finish the in-flight request, then exit.
              conn.as_mut().graceful_shutdown();
              if let Err(err) = conn.as_mut().await {
                error!("Connection error during drain: {:?}", err);
              }
            }
          }
        });
      }
      _ = shutdown.cancelled() => {
        return Ok(());
      }
    }
  }
}

/// Waits for a shutdown signal: Ctrl+C (SIGINT) or SIGTERM on Unix,
/// Ctrl+C elsewhere.
async fn shutdown_signal() {
  #[cfg(unix)]
  {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    tokio::select! {
      _ = tokio::signal::ctrl_c() => {}
      _ = term.recv() => {}
    }
  }
  #[cfg(not(unix))]
  {
    tokio::signal::ctrl_c()
      .await
      .expect("failed to install Ctrl+C handler");
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
