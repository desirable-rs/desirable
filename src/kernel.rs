use crate::IntoResponse;
use crate::{Request, Result};
use std::future::Future;
use std::sync::Arc;
#[async_trait::async_trait]
pub trait Endpoint: Send + Sync + 'static {
  async fn call(&self, req: Request) -> Result;
}

pub type DynEndpoint = dyn Endpoint;

#[async_trait::async_trait]
impl<F, Fut, Res> Endpoint for F
where
  F: Send + Sync + 'static + Fn(Request) -> Fut,
  Fut: Future<Output = Res> + Send + 'static,
  Res: IntoResponse + 'static,
{
  async fn call(&self, req: Request) -> Result {
    let fut = (self)(req);
    let res = fut.await;
    res.into_response()
  }
}

pub struct Next<'a> {
  pub endpoint: &'a DynEndpoint,
  pub middlewares: &'a [Arc<dyn Middleware>],
}

impl Next<'_> {
  pub async fn run(mut self, req: Request) -> Result {
    if let Some((cur, next)) = self.middlewares.split_first() {
      self.middlewares = next;
      match cur.handle(req, self).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
      }
    } else {
      self.endpoint.call(req).await
    }
  }
}

#[async_trait::async_trait]
pub trait Middleware: Send + Sync + 'static {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result;
  fn name(&self) -> &str {
    std::any::type_name::<Self>()
  }
}

#[async_trait::async_trait]
impl<F, Fut, Res> Middleware for F
where
  F: Send + Sync + 'static + Fn(Request, Next) -> Fut,
  Fut: Future<Output = Res> + Send + 'static,
  Res: IntoResponse + 'static,
{
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let fut = (self)(req, next);
    let res = fut.await;
    res.into_response()
  }
}
