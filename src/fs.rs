use crate::Endpoint;
use crate::Request;
use crate::Result;
use bytes::Bytes;
use http_body_util::Full;
use std::path::Path;
use std::path::PathBuf;
use tracing::info;
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
    let ext = Path::new(&file).extension();
    let file = PathBuf::from(format!("{}/{}", dir.to_string_lossy(), file));
    let body = tokio::fs::read(file).await?;
    info!("ext {:?}", ext);
    let mut response = hyper::Response::new(Full::new(Bytes::from(body)));
    if let Some(ext) = ext {
      let ext = ext.to_string_lossy();
      if ext.contains(&"js") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::TEXT_JAVASCRIPT.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"css") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::TEXT_CSS.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"png") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_PNG.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"png") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_PNG.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"svg") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_SVG.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"jpeg") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_JPEG.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"gif") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_GIF.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"bmp") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::IMAGE_BMP.as_ref().parse().unwrap(),
        );
      } else if ext.contains(&"mp4") {
        response.headers_mut().insert(
          hyper::header::CONTENT_TYPE,
          mime::MP4.as_ref().parse().unwrap(),
        );
      }
    }
    Ok(response.into())
  }
}
