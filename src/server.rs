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
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;
use tracing::{debug, error, info, warn};

/// Default maximum time in-flight connections are given to finish after a
/// shutdown signal before the server gives up waiting.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default maximum request-body size, enforced when a handler buffers the
/// body (`Request::body`/`body_json`/`form`). Without a default, one request
/// with a huge body could buffer the process out of memory.
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Maximum time a TLS handshake may take before the connection is dropped
/// (feature `tls`).
#[cfg(feature = "tls")]
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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
pub(crate) struct Svc {
  /// The router for handling requests
  pub(crate) router: Arc<Router>,
  /// The remote address of the client (`None` for Unix domain sockets)
  pub(crate) remote_addr: Option<Arc<SocketAddr>>,
  /// Trusted proxy networks used to resolve the real client IP
  pub(crate) trusted_proxies: Arc<Vec<IpNet>>,
  /// Connection/task tracker shared with the accept loop, so spawned work
  /// (e.g. WebSocket sessions) is counted by graceful shutdown
  pub(crate) tracker: Arc<TaskTracker>,
  /// Server-level default request-body limit (`None` = unlimited)
  pub(crate) body_limit: Option<usize>,
}

impl Service<HyperRequest> for Svc {
  type Response = HyperResponse;
  type Error = crate::error::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response>> + Send>>;

  fn call(&self, req: HyperRequest) -> Self::Future {
    let router = self.router.clone();
    let remote_addr = self.remote_addr.clone();
    let trusted = self.trusted_proxies.clone();
    let tracker = Arc::clone(&self.tracker);
    let body_limit = self.body_limit;
    let res = async move { dispatch(req, remote_addr, trusted, tracker, body_limit, router).await };
    Box::pin(res)
  }
}

/// Creates the per-request tracing span that wraps request handling, so any
/// `tracing` log emitted by handlers and middleware carries the request's
/// method, path, and (when the client or `RequestId` provides one) request id.
fn request_span(method: &hyper::Method, path: &str, request_id: Option<&str>) -> tracing::Span {
  match request_id {
    Some(id) => tracing::info_span!(
      "request",
      http.method = %method,
      http.path = %path,
      http.request_id = %id,
    ),
    None => tracing::info_span!(
      "request",
      http.method = %method,
      http.path = %path,
      http.request_id = tracing::field::Empty,
    ),
  }
}

/// Dispatches a hyper request through the router.
///
/// Internal: converts hyper types to framework types and routes the request
/// through the application router.
///
/// # Arguments
///
/// * `req` - The incoming hyper request
/// * `remote_addr` - The client's socket address (`None` for Unix sockets)
/// * `trusted_proxies` - Proxy networks trusted to set `X-Forwarded-For`
/// * `tracker` - Connection task tracker
/// * `body_limit` - Server-level default request-body limit
/// * `router` - The application router
///
/// # Returns
///
/// The hyper response or an error
async fn dispatch(
  req: HyperRequest,
  remote_addr: Option<Arc<SocketAddr>>,
  trusted_proxies: Arc<Vec<IpNet>>,
  tracker: Arc<TaskTracker>,
  body_limit: Option<usize>,
  router: Arc<Router>,
) -> Result<HyperResponse> {
  let mut req = req;
  req.extensions_mut().insert(tracker);
  // Server-level default body limit. An explicit `BodyLimit` middleware,
  // installed later in the chain, replaces this value via its own insert.
  if let Some(max) = body_limit {
    req
      .extensions_mut()
      .insert(crate::middleware::body_limit::BodyLimitValue(max));
  }
  if let Some(addr) = &remote_addr {
    let client_ip = resolve_client_ip(addr.ip(), req.headers(), &trusted_proxies);
    req
      .extensions_mut()
      .insert(crate::request::ClientIp(client_ip));
  }
  let span = request_span(
    req.method(),
    req.uri().path(),
    req
      .headers()
      .get("x-request-id")
      .and_then(|v| v.to_str().ok()),
  );
  // A propagated `Err` (middleware failure, invalid status code, ...) is
  // rendered like any other error — returning it raw would close the
  // connection with no response at all.
  let fut = router.dispatch(req.into(), remote_addr);
  let response = match fut.instrument(span).await {
    Ok(resp) => resp,
    Err(err) => crate::error::render_error(err),
  };
  Ok(response.inner)
}

/// Resolves the real client IP: the first non-trusted address walking the
/// `X-Forwarded-For` chain from right to left. Falls back to the peer
/// address when absent, malformed, or entirely trusted.
fn resolve_client_ip(
  peer: std::net::IpAddr,
  headers: &hyper::HeaderMap,
  trusted: &[IpNet],
) -> std::net::IpAddr {
  use std::net::IpAddr;

  // The X-Forwarded-For chain is only honored when the immediate peer is a
  // trusted proxy. A direct client can set the header to anything, so
  // without this gate the client IP is fully attacker-controlled. The
  // common no-proxies deployment short-circuits here.
  if trusted.is_empty() || !trusted.iter().any(|net| net.contains(&peer)) {
    return peer;
  }

  let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
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
///   router.get("/", |_| async { "Hello!" });
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
  /// Default request-body limit for handlers that buffer the body
  body_limit: Option<usize>,
  /// Hard cap on concurrent connections (`None` = unbounded)
  max_connections: Option<usize>,
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
      body_limit: Some(DEFAULT_BODY_LIMIT),
      max_connections: None,
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
      body_limit: Some(DEFAULT_BODY_LIMIT),
      max_connections: None,
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
  /// Protects against slow-loris style clients. Default: 30 seconds
  /// (hyper's built-in default); this setter customizes it.
  #[must_use]
  pub fn http1_header_read_timeout(mut self, timeout: Duration) -> Self {
    self.header_read_timeout = Some(timeout);
    self
  }

  /// Overrides the default request-body limit (2 MiB, see
  /// [`DEFAULT_BODY_LIMIT`]) enforced when a handler buffers the body via
  /// [`Request::body`](crate::Request::body) / `body_json` / `form`.
  /// Exceeding it yields `413 Payload Too Large`.
  ///
  /// An explicit [`BodyLimit`](crate::BodyLimit) middleware takes precedence
  /// over this server-level default.
  #[must_use]
  pub fn body_limit(mut self, max_bytes: usize) -> Self {
    self.body_limit = Some(max_bytes);
    self
  }

  /// Disables the server-level request-body limit, restoring fully
  /// unbounded buffering (not recommended: one request can exhaust memory).
  #[must_use]
  pub fn no_body_limit(mut self) -> Self {
    self.body_limit = None;
    self
  }

  /// Caps the number of concurrently handled connections. When the cap is
  /// reached, new connections are closed immediately (fail fast) instead of
  /// being queued; a debug-level log records each rejection.
  ///
  /// Default: unbounded.
  #[must_use]
  pub fn max_connections(mut self, max: usize) -> Self {
    self.max_connections = Some(max);
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
  /// Accepts CIDR strings and bare IPs. The `X-Forwarded-For` chain is
  /// honored ONLY when the immediate peer is inside a trusted network; the
  /// resolved client IP (first non-trusted address from the right) is
  /// exposed via [`Request::client_ip`](crate::Request::client_ip) and used
  /// by the [`RateLimit`](crate::RateLimit) middleware.
  ///
  /// Off by default: without trusted proxies the header is never trusted,
  /// and spoofed values cannot influence `client_ip()` or rate limiting.
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
    let tracker = Arc::new(TaskTracker::new());
    let shutdown = CancellationToken::new();
    let trusted = Arc::new(self.trusted_proxies.clone());
    let connection_permits = self
      .max_connections
      .map(|max| Arc::new(Semaphore::new(max)));
    let config = ConnConfig {
      body_limit: self.body_limit,
      header_read_timeout: self.header_read_timeout,
      connection_permits,
      #[cfg(feature = "tls")]
      tls_acceptor: self.tls_acceptor(),
    };

    tokio::select! {
      result = accept_loop(
        listener,
        router,
        Arc::clone(&tracker),
        shutdown.clone(),
        Arc::clone(&trusted),
        config,
      ) => {
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

/// Per-connection serving configuration threaded from `Server` into the
/// accept loop.
struct ConnConfig {
  body_limit: Option<usize>,
  header_read_timeout: Option<Duration>,
  connection_permits: Option<Arc<Semaphore>>,
  #[cfg(feature = "tls")]
  tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
}

/// Accepts connections in a loop until the listener is closed, an error
/// occurs, or the shutdown token is cancelled.
async fn accept_loop<L: Listener>(
  listener: L,
  router: Arc<Router>,
  tracker: Arc<TaskTracker>,
  shutdown: CancellationToken,
  trusted: Arc<Vec<IpNet>>,
  config: ConnConfig,
) -> Result<()> {
  let ConnConfig {
    body_limit,
    header_read_timeout,
    connection_permits,
    #[cfg(feature = "tls")]
    tls_acceptor,
  } = config;

  loop {
    let accept_result = tokio::select! {
      accepted = listener.accept() => {
        match accepted {
          Ok(ok) => ok,
          // Transient accept failures (interrupted, aborted by the client)
          // are logged and retried instead of killing the loop.
          Err(err) if matches!(
            err.kind(),
            std::io::ErrorKind::ConnectionAborted
              | std::io::ErrorKind::Interrupted
              | std::io::ErrorKind::WouldBlock
          ) => {
            warn!("Transient accept error: {}", err);
            continue;
          }
          Err(err) => return Err(err.into()),
        }
      }
      _ = shutdown.cancelled() => {
        return Ok(());
      }
    };
    let (io, remote_addr) = accept_result;
    // Concurrency cap: fail fast when full — the permit lives inside the
    // connection task, so it frees up as soon as the connection finishes
    // (including failed TLS handshakes).
    let permit = match connection_permits.as_ref().map(Arc::clone) {
      Some(sem) => match sem.try_acquire_owned() {
        Ok(permit) => Some(permit),
        Err(_) => {
          debug!(
            "connection limit reached, rejecting peer {:?}",
            remote_addr.as_deref().map(|a| a.to_string())
          );
          continue; // drops `io`: the client sees a closed connection
        }
      },
      None => None,
    };
    let router = Arc::clone(&router);
    let trusted = Arc::clone(&trusted);
    let shutdown = shutdown.clone();
    let conn_tracker = Arc::clone(&tracker);
    #[cfg(feature = "tls")]
    let tls_acceptor = tls_acceptor.clone();
    let svc_tracker = Arc::clone(&conn_tracker);
    conn_tracker.spawn(async move {
      // Hold the permit for the whole connection lifetime.
      let _permit = permit;
      // preface is auto-detected), plus WebSocket upgrade support.
      let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
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
      // TLS handshake (feature `tls`): bounded by a handshake timeout and
      // interrupted by shutdown; failures just drop the connection.
      #[cfg(feature = "tls")]
      let io = match (tls_acceptor, io) {
        (Some(acceptor), AcceptedIo::Tcp(tcp)) => {
          let handshake = async {
            let tls_stream = acceptor.accept(tcp).await?;
            debug!(
              "TLS established, alpn: {:?}",
              tls_stream.get_ref().1.alpn_protocol()
            );
            Ok::<_, std::io::Error>(AnyStream::TlsTcp(TokioIo::new(Box::new(tls_stream))))
          };
          match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, handshake).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
              debug!("TLS handshake failed: {}", err);
              return;
            }
            Err(_) => {
              debug!("TLS handshake timed out");
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
          tracker: svc_tracker,
          body_limit,
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
  use super::{Server, parse_trusted_proxies};
  use std::net::IpAddr;

  fn trusted() -> Vec<IpNet> {
    parse_trusted_proxies(["10.0.0.0/8", "127.0.0.1"])
  }

  fn request_with_xff(value: &str) -> hyper::HeaderMap {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-forwarded-for", value.parse().unwrap());
    headers
  }

  #[test]
  fn test_request_span_records_method_path_and_request_id() {
    use std::sync::{Arc, Mutex};

    /// (span id, name, initial fields) + post-creation records.
    type CapturedSpans = Vec<(tracing::Id, String, Vec<(String, String)>)>;

    // Captures span creation (name + initial fields) and later `record`
    // calls (e.g. the request id, which is attached after creation).
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Captured>>);

    #[derive(Default)]
    struct Captured {
      spans: CapturedSpans,
      records: Vec<(tracing::Id, String, String)>,
    }

    impl<S> tracing_subscriber::Layer<S> for Capture
    where
      S: tracing::Subscriber,
    {
      fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
      ) {
        struct Visitor(Vec<(String, String)>);
        impl tracing::field::Visit for Visitor {
          fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self
              .0
              .push((field.name().to_string(), format!("{value:?}")));
          }
        }
        let mut visitor = Visitor(Vec::new());
        attrs.record(&mut visitor);
        let mut state = self.0.lock().unwrap();
        state
          .spans
          .push((id.clone(), attrs.metadata().name().to_string(), visitor.0));
      }

      fn on_record(
        &self,
        id: &tracing::Id,
        values: &tracing::span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
      ) {
        struct Visitor(Vec<(String, String)>);
        impl tracing::field::Visit for Visitor {
          fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self
              .0
              .push((field.name().to_string(), format!("{value:?}")));
          }
        }
        let mut visitor = Visitor(Vec::new());
        values.record(&mut visitor);
        let mut state = self.0.lock().unwrap();
        for (field, value) in visitor.0 {
          state.records.push((id.clone(), field, value));
        }
      }
    }

    let captured = Capture::default();
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    tracing::subscriber::with_default(subscriber, || {
      let _a = request_span(&hyper::Method::GET, "/users/42", Some("abc-1"));
      let _b = request_span(&hyper::Method::POST, "/login", None);
    });

    let state = captured.0.lock().unwrap();
    assert_eq!(state.spans.len(), 2);
    let (_id0, name0, fields0) = &state.spans[0];
    assert_eq!(name0, "request");
    assert!(fields0.contains(&("http.method".to_string(), "GET".to_string())));
    assert!(fields0.contains(&("http.path".to_string(), "/users/42".to_string())));
    // Incoming request id is part of the span at creation time.
    assert!(fields0.contains(&("http.request_id".to_string(), "abc-1".to_string())));
    // No incoming request id on the second span: the field stays empty.
    assert!(!state.spans[1].2.iter().any(|(f, _)| f == "http.request_id"));
  }

  #[test]
  fn test_client_ip_direct_peer_without_xff() {
    let peer: IpAddr = "203.0.113.5".parse().unwrap();
    let headers = hyper::HeaderMap::new();
    assert_eq!(resolve_client_ip(peer, &headers, &trusted()), peer);
  }

  #[test]
  fn test_client_ip_untrusted_peer_xff_is_ignored() {
    // Peer is NOT in trusted networks: spoofed XFF must be ignored.
    let peer: IpAddr = "203.0.113.5".parse().unwrap();
    let req = request_with_xff("9.9.9.9");
    assert_eq!(resolve_client_ip(peer, &req, &trusted()), peer);
  }

  #[test]
  fn test_client_ip_trusted_peer_resolves_xff() {
    // Peer 127.0.0.1 is trusted: rightmost untrusted entry wins.
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    let req = request_with_xff("203.0.113.7, 10.0.0.1");
    assert_eq!(
      resolve_client_ip(peer, &req, &trusted()),
      "203.0.113.7".parse::<IpAddr>().unwrap()
    );
  }

  #[test]
  fn test_client_ip_chain_all_trusted_falls_to_leftmost() {
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    let req = request_with_xff("10.0.0.1, 10.0.0.2");
    assert_eq!(
      resolve_client_ip(peer, &req, &trusted()),
      "10.0.0.1".parse::<IpAddr>().unwrap()
    );
  }

  #[test]
  fn test_client_ip_untrusted_entry_wins_over_malformed_left() {
    // Rightmost entry is untrusted -> it is the client; the malformed
    // entry further left is never reached.
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    let req = request_with_xff("not-an-ip, 203.0.113.7");
    assert_eq!(
      resolve_client_ip(peer, &req, &trusted()),
      "203.0.113.7".parse::<IpAddr>().unwrap()
    );
  }

  #[test]
  fn test_client_ip_malformed_rightmost_falls_back_to_peer() {
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    let req = request_with_xff("garbage");
    assert_eq!(resolve_client_ip(peer, &req, &trusted()), peer);
  }

  #[test]
  fn test_client_ip_empty_trusted_list_never_trusts() {
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    let req = request_with_xff("9.9.9.9");
    assert_eq!(resolve_client_ip(peer, &req, &[]), peer);
  }

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
