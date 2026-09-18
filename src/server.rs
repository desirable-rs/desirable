use crate::HyperRequest;
use crate::HyperResponse;
use crate::Result;
use crate::Router;
use hyper::service::Service;
use hyper_util::rt::TokioIo;
use hyper_util::rt::TokioTimer;
use ipnet::IpNet;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

/// Default maximum time in-flight connections are given to finish after a
/// shutdown signal before the server gives up waiting.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the server binds to.
#[derive(Clone, Debug)]
enum BindTarget {
  /// A TCP socket address.
  Tcp(SocketAddr),
  /// A Unix domain socket path (unix only).
  #[cfg(unix)]
  Unix(PathBuf),
}

/// Internal service type for hyper integration.
///
/// Adapts the router to hyper's Service trait.
pub struct Svc {
  /// The router for handling requests
  pub router: Arc<Router>,
  /// The remote address of the client (`None` for Unix domain sockets)
  pub remote_addr: Option<Arc<SocketAddr>>,
  /// Trusted proxy networks used to resolve the real client IP
  pub trusted_proxies: Arc<Vec<IpNet>>,
}

impl Service<HyperRequest> for Svc {
  type Response = HyperResponse;
  type Error = crate::error::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response>> + Send>>;

  fn call(&self, req: HyperRequest) -> Self::Future {
    let router = self.router.clone();
    let remote_addr = self.remote_addr.clone();
    let trusted = self.trusted_proxies.clone();
    let res = async { dispatch(req, remote_addr, trusted, router).await };
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
/// * `remote_addr` - The client's socket address (`None` for Unix sockets)
/// * `trusted_proxies` - Proxy networks trusted to set `X-Forwarded-For`
/// * `router` - The application router
///
/// # Returns
///
/// The hyper response or an error
pub async fn dispatch(
  req: HyperRequest,
  remote_addr: Option<Arc<SocketAddr>>,
  trusted_proxies: Arc<Vec<IpNet>>,
  router: Arc<Router>,
) -> Result<HyperResponse> {
  let mut req = req;
  if let Some(addr) = &remote_addr {
    let client_ip = resolve_client_ip(addr.ip(), &req, &trusted_proxies);
    req
      .extensions_mut()
      .insert(crate::request::ClientIp(client_ip));
  }
  let response = router.dispatch(req.into(), remote_addr).await?;
  Ok(response.inner)
}

/// Resolves the real client IP: the first non-trusted address walking the
/// `X-Forwarded-For` chain from right to left. Falls back to the peer
/// address when absent, malformed, or entirely trusted.
fn resolve_client_ip(
  peer: std::net::IpAddr,
  req: &HyperRequest,
  trusted: &[IpNet],
) -> std::net::IpAddr {
  use std::net::IpAddr;

  let Some(xff) = req
    .headers()
    .get("x-forwarded-for")
    .and_then(|v| v.to_str().ok())
  else {
    return peer;
  };
  let mut client = peer;
  for part in xff.split(',').rev().map(str::trim) {
    match part.parse::<IpAddr>() {
      Ok(ip) => {
        if trusted.iter().any(|net| net.contains(&ip)) {
          client = ip; // trusted hop: keep walking left
        } else {
          return ip; // first untrusted address from the right
        }
      }
      Err(_) => break, // malformed entry: stop trusting the chain
    }
  }
  client
}

/// The HTTP server.
///
/// Binds to a socket address (TCP, or a Unix domain socket on unix) and
/// serves requests using a router.
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
  /// What to bind to
  target: BindTarget,
  /// Maximum time in-flight connections are given to finish after shutdown
  drain_timeout: Duration,
  /// Proxy networks trusted to set `X-Forwarded-For`
  trusted_proxies: Vec<IpNet>,
  /// Maximum time to wait for the client to send complete request headers
  header_read_timeout: Option<Duration>,
  /// TLS configuration (feature `tls`)
  #[cfg(feature = "tls")]
  tls: Option<Arc<tokio_rustls::rustls::ServerConfig>>,
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
      target: BindTarget::Tcp(addr),
      drain_timeout: DEFAULT_DRAIN_TIMEOUT,
      trusted_proxies: Vec::new(),
      header_read_timeout: None,
      #[cfg(feature = "tls")]
      tls: None,
    })
  }

  /// Creates a new server bound to a Unix domain socket (unix only).
  ///
  /// A stale socket file at `path` is removed before binding.
  #[cfg(unix)]
  #[must_use]
  pub fn bind_unix(path: impl Into<PathBuf>) -> Self {
    Server {
      target: BindTarget::Unix(path.into()),
      drain_timeout: DEFAULT_DRAIN_TIMEOUT,
      trusted_proxies: Vec::new(),
      header_read_timeout: None,
      #[cfg(feature = "tls")]
      tls: None,
    }
  }

  /// Returns the TCP address this server binds to, when configured for TCP.
  pub fn tcp_addr(&self) -> Option<SocketAddr> {
    match &self.target {
      BindTarget::Tcp(addr) => Some(*addr),
      #[cfg(unix)]
      BindTarget::Unix(_) => None,
    }
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

  /// Sets the maximum time to wait for a client to send complete request
  /// headers.
  ///
  /// Protects against slow-loris style clients. Default: disabled.
  #[must_use]
  pub fn http1_header_read_timeout(mut self, timeout: Duration) -> Self {
    self.header_read_timeout = Some(timeout);
    self
  }

  /// Returns the TLS acceptor when TLS is configured (feature `tls`).
  #[cfg(feature = "tls")]
  fn tls_acceptor(&self) -> Option<tokio_rustls::TlsAcceptor> {
    self
      .tls
      .as_ref()
      .map(|config| tokio_rustls::TlsAcceptor::from(Arc::clone(config)))
  }

  /// Enables TLS on the TCP listener with the given rustls configuration.
  ///
  /// Set `alpn_protocols` to `["h2", "http/1.1"]` (as
  /// [`tls::server_config_from_pem`](crate::tls::server_config_from_pem)
  /// does) to negotiate HTTP/2 alongside HTTP/1.1. Unix domain socket
  /// listeners are unaffected.
  #[cfg(feature = "tls")]
  #[must_use]
  pub fn tls_config(mut self, config: Arc<tokio_rustls::rustls::ServerConfig>) -> Self {
    self.tls = Some(config);
    self
  }

  /// Declares proxy networks trusted to set `X-Forwarded-For`.
  ///
  /// Accepts CIDR strings and bare IPs. When the immediate peer is inside a
  /// trusted network, the client IP is resolved by walking
  /// `X-Forwarded-For` from right to left and taking the first address that
  /// is NOT in a trusted network; the result is exposed via
  /// [`Request::client_ip`](crate::Request::client_ip) and used by the
  /// [`RateLimit`](crate::RateLimit) middleware.
  ///
  /// Off by default: without trusted proxies the header is never trusted.
  ///
  /// # Panics
  ///
  /// Panics if an entry is not a valid IP or CIDR string.
  #[must_use]
  pub fn trusted_proxies<I, S>(mut self, proxies: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
  {
    self.trusted_proxies = parse_trusted_proxies(proxies);
    self
  }

  /// Starts the server and serves requests until shutdown signal.
  ///
  /// Delegates to [`Server::run_graceful`]. Listens for Ctrl+C (SIGINT) —
  /// and SIGTERM on unix — then drains connections gracefully.
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

  /// Starts the server with graceful shutdown on Ctrl+C (SIGINT) / SIGTERM.
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
    match &self.target {
      BindTarget::Tcp(addr) => {
        let listener = TcpListener::bind(*addr).await?;
        info!("Listening on http://{}", addr);
        self.serve_listener(listener, router, signal).await
      }
      #[cfg(unix)]
      BindTarget::Unix(path) => {
        let listener = bind_unix(path).await?;
        info!("Listening on unix://{}", path.display());
        self.serve_listener(listener, router, signal).await
      }
    }
  }

  /// Starts the server on an already-bound TCP listener and serves requests
  /// until a shutdown signal (Ctrl+C / SIGTERM), then drains connections.
  ///
  /// Useful for systemd socket activation, tests, or serving multiple
  /// listeners: spawn one task per listener sharing a shutdown signal.
  pub async fn run_tcp_listener(&self, router: Router, listener: TcpListener) -> Result<()> {
    self
      .serve_listener(listener, router, shutdown_signal())
      .await
  }

  /// Core serve loop shared by every entry point: accepts connections on
  /// `listener` until the signal resolves, then drains.
  async fn serve_listener<L>(
    &self,
    listener: L,
    router: Router,
    signal: impl Future<Output = ()>,
  ) -> Result<()>
  where
    L: Listener,
  {
    info!("Listening on {}", listener.describe());
    let router = Arc::new(router);
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let trusted = Arc::new(self.trusted_proxies.clone());

    tokio::select! {
      result = accept_loop(listener, router, &tracker, shutdown.clone(), Arc::clone(&trusted), self) => {
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

/// Trait unifying TCP and Unix listeners for the accept loop.
trait Listener: Send {
  fn describe(&self) -> String;
  fn accept(
    &self,
  ) -> impl Future<Output = std::io::Result<(AcceptedIo, Option<Arc<SocketAddr>>)>> + Send;
}

impl Listener for TcpListener {
  fn describe(&self) -> String {
    format!(
      "http://{}",
      self.local_addr().map(|a| a.to_string()).unwrap_or_default()
    )
  }

  fn accept(
    &self,
  ) -> impl Future<Output = std::io::Result<(AcceptedIo, Option<Arc<SocketAddr>>)>> + Send {
    let fut = TcpListener::accept(self);
    async move {
      let (stream, addr) = fut.await?;
      // Reduce latency for small request/response pairs; failure is benign.
      if let Err(err) = stream.set_nodelay(true) {
        debug!("Failed to set TCP_NODELAY: {}", err);
      }
      let remote_addr = Some(Arc::new(addr));
      Ok((AcceptedIo::Tcp(stream), remote_addr))
    }
  }
}

#[cfg(unix)]
impl Listener for tokio::net::UnixListener {
  fn describe(&self) -> String {
    format!(
      "unix://{}",
      self
        .local_addr()
        .map(|a| a
          .as_pathname()
          .map(|p| p.display().to_string())
          .unwrap_or_default())
        .unwrap_or_default()
    )
  }

  fn accept(
    &self,
  ) -> impl Future<Output = std::io::Result<(AcceptedIo, Option<Arc<SocketAddr>>)>> + Send {
    let fut = tokio::net::UnixListener::accept(self);
    async move {
      let (stream, _addr) = fut.await?;
      Ok((AcceptedIo::Unix(stream), None))
    }
  }
}

#[cfg(unix)]
async fn bind_unix(path: &PathBuf) -> std::io::Result<tokio::net::UnixListener> {
  // Remove a stale socket file from a previous run.
  let _ = tokio::fs::remove_file(path).await;
  tokio::net::UnixListener::bind(path)
}

/// A freshly accepted connection (pre-TLS handshake).
enum AcceptedIo {
  Tcp(tokio::net::TcpStream),
  #[cfg(unix)]
  Unix(tokio::net::UnixStream),
}

/// A connection IO from any listener kind, post-TLS.
enum AnyStream {
  Tcp(TokioIo<tokio::net::TcpStream>),
  #[cfg(unix)]
  Unix(TokioIo<tokio::net::UnixStream>),
  #[cfg(feature = "tls")]
  TlsTcp(TokioIo<Box<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>>),
}

impl hyper::rt::Read for AnyStream {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: hyper::rt::ReadBufCursor<'_>,
  ) -> Poll<std::io::Result<()>> {
    match self.get_mut() {
      AnyStream::Tcp(io) => Pin::new(io).poll_read(cx, buf),
      #[cfg(unix)]
      AnyStream::Unix(io) => Pin::new(io).poll_read(cx, buf),
      #[cfg(feature = "tls")]
      AnyStream::TlsTcp(io) => Pin::new(io).poll_read(cx, buf),
    }
  }
}

impl hyper::rt::Write for AnyStream {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<std::io::Result<usize>> {
    match self.get_mut() {
      AnyStream::Tcp(io) => Pin::new(io).poll_write(cx, buf),
      #[cfg(unix)]
      AnyStream::Unix(io) => Pin::new(io).poll_write(cx, buf),
      #[cfg(feature = "tls")]
      AnyStream::TlsTcp(io) => Pin::new(io).poll_write(cx, buf),
    }
  }

  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    match self.get_mut() {
      AnyStream::Tcp(io) => Pin::new(io).poll_flush(cx),
      #[cfg(unix)]
      AnyStream::Unix(io) => Pin::new(io).poll_flush(cx),
      #[cfg(feature = "tls")]
      AnyStream::TlsTcp(io) => Pin::new(io).poll_flush(cx),
    }
  }

  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    match self.get_mut() {
      AnyStream::Tcp(io) => Pin::new(io).poll_shutdown(cx),
      #[cfg(unix)]
      AnyStream::Unix(io) => Pin::new(io).poll_shutdown(cx),
      #[cfg(feature = "tls")]
      AnyStream::TlsTcp(io) => Pin::new(io).poll_shutdown(cx),
    }
  }
}

/// Wraps a plain accepted connection into the served stream form.
fn accepted_to_served(io: AcceptedIo) -> AnyStream {
  match io {
    AcceptedIo::Tcp(stream) => AnyStream::Tcp(TokioIo::new(stream)),
    #[cfg(unix)]
    AcceptedIo::Unix(stream) => AnyStream::Unix(TokioIo::new(stream)),
  }
}

fn parse_trusted_proxies<I, S>(proxies: I) -> Vec<IpNet>
where
  I: IntoIterator<Item = S>,
  S: AsRef<str>,
{
  proxies
    .into_iter()
    .map(|s| {
      let raw = s.as_ref();
      match raw.parse::<IpNet>() {
        Ok(net) => net,
        Err(_) => {
          // Bare IP: treat as a /32 (v4) or /128 (v6) host route.
          let ip: std::net::IpAddr = raw
            .parse()
            .unwrap_or_else(|_| panic!("invalid trusted proxy: {raw:?}"));
          IpNet::new(ip, if ip.is_ipv4() { 32 } else { 128 }).expect("host prefix is valid")
        }
      }
    })
    .collect()
}

/// Accepts connections in a loop until the listener is closed, an error
/// occurs, or the shutdown token is cancelled.
async fn accept_loop<L: Listener>(
  listener: L,
  router: Arc<Router>,
  tracker: &TaskTracker,
  shutdown: CancellationToken,
  trusted: Arc<Vec<IpNet>>,
  server: &Server,
) -> Result<()> {
  #[cfg(feature = "tls")]
  let tls_acceptor = server.tls_acceptor();

  loop {
    tokio::select! {
      accepted = listener.accept() => {
        let (io, remote_addr) = accepted?;
        let router = Arc::clone(&router);
        let trusted = Arc::clone(&trusted);
        let shutdown = shutdown.clone();
        let header_read_timeout = server.header_read_timeout;
        #[cfg(feature = "tls")]
        let tls_acceptor = tls_acceptor.clone();
        tracker.spawn(async move {
          // auto builder: HTTP/1.1 and ALPN-negotiated HTTP/2 (the h2 client
          // preface is auto-detected), plus WebSocket upgrade support.
          let mut builder = hyper_util::server::conn::auto::Builder::new(
            hyper_util::rt::TokioExecutor::new(),
          );
          {
            let h1 = &mut builder.http1();
            h1.timer(TokioTimer::new());
            if let Some(d) = header_read_timeout {
              h1.header_read_timeout(Some(d));
            }
          }
          {
            let h2 = &mut builder.http2();
            h2.timer(TokioTimer::new());
          }
          // TLS handshake (feature `tls`): failures just drop the connection.
          #[cfg(feature = "tls")]
          let io = match (tls_acceptor, io) {
            (Some(acceptor), AcceptedIo::Tcp(tcp)) => {
              match acceptor.accept(tcp).await {
                Ok(tls_stream) => {
                  debug!(
                    "TLS established, alpn: {:?}",
                    tls_stream.get_ref().1.alpn_protocol()
                  );
                  AnyStream::TlsTcp(TokioIo::new(Box::new(tls_stream)))
                }
                Err(err) => {
                  debug!("TLS handshake failed: {}", err);
                  return;
                }
              }
            }
            (None, other) => accepted_to_served(other),
            #[cfg(unix)]
            (_, other @ AcceptedIo::Unix(_)) => accepted_to_served(other),
          };
          #[cfg(not(feature = "tls"))]
          let io = accepted_to_served(io);
          let conn = builder.serve_connection_with_upgrades(
            io,
            Svc {
              router,
              remote_addr,
              trusted_proxies: Arc::clone(&trusted),
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
    assert_eq!(server.tcp_addr().unwrap().to_string(), "127.0.0.1:8080");
  }

  #[test]
  fn test_server_bind_different_ports() {
    let server1 = Server::bind("0.0.0.0:3000");
    let server2 = Server::bind("0.0.0.0:8080");
    assert_eq!(server1.tcp_addr().unwrap().port(), 3000);
    assert_eq!(server2.tcp_addr().unwrap().port(), 8080);
  }

  #[test]
  fn test_server_bind_unix() {
    #[cfg(unix)]
    {
      let server = Server::bind_unix("/tmp/desirable-test.sock");
      assert!(server.tcp_addr().is_none());
    }
  }
}
