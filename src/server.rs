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
pub struct Svc {
  pub router: Arc<Router>,
  pub remote_addr: Arc<SocketAddr>,
}

impl Service<HyperRequest> for Svc {
  type Response = HyperResponse;
  type Error = crate::error::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response>> + Send>>;

  fn call(&self, req: HyperRequest) -> Self::Future {
    let router = self.router.clone();
    let remote_addr = self.remote_addr.clone();
    let res = dispatch(req, remote_addr, router);
    Box::pin(async { res.await })
  }
}

pub async fn dispatch(
  req: HyperRequest,
  remote_addr: Arc<SocketAddr>,
  router: Arc<Router>,
) -> Result<HyperResponse> {
  let response = router.dispatch(req.into(), remote_addr).await?;
  Ok(response.inner)
}

pub struct Server {
  addr: SocketAddr,
}

impl Server {
  pub fn bind(addr: &str) -> Self {
    Server {
      addr: addr.parse().unwrap(),
    }
  }

  pub async fn run(&self, router: Router) -> Result<()> {
    let addr: SocketAddr = self.addr.into();
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
