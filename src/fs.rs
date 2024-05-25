use crate::Endpoint;
use crate::Request;
use crate::Result;
use bytes::Bytes;
use http_body_util::Full;
use std::path::PathBuf;
pub struct ServeFile {
  path: PathBuf,
}
impl ServeFile {
  pub fn new(path: PathBuf) -> Self {
    ServeFile { path }
  }
}
#[async_trait::async_trait]
impl Endpoint for ServeFile {
  async fn call(&self, _req: Request) -> Result {
    let body = tokio::fs::read(&self.path).await?;
    let response = hyper::Response::new(Full::new(Bytes::from(body)));
    Ok(response.into())
  }
}

pub struct ServeDir {
  dir: PathBuf,
}

impl ServeDir {
  pub fn new(dir: PathBuf) -> Self {
    ServeDir { dir }
  }
}
#[async_trait::async_trait]
impl Endpoint for ServeDir {
  async fn call(&self, req: Request) -> Result {
    let file = req.param::<String>("file")?;
    let dir = self.dir.clone();
    let file = PathBuf::from(format!("{}/{}", dir.to_string_lossy(), file));
    let body = tokio::fs::read(file).await?;
    let response = hyper::Response::new(Full::new(Bytes::from(body)));
    Ok(response.into())
  }
}
