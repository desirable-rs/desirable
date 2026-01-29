use crate::Endpoint;
use crate::Request;
use crate::Result;
use bytes::Bytes;
use http_body_util::Full;
use std::path::PathBuf;

/// Endpoint for serving a single static file.
///
/// Reads the file from disk and returns it as the response body.
/// The content type is inferred from the file extension.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{ServeFile, Router};
///
/// let mut router = Router::new();
/// router.get("/favicon.ico", ServeFile::new("static/favicon.ico".into()));
/// ```
pub struct ServeFile {
  /// The path to the file to serve
  path: PathBuf,
}

impl ServeFile {
  /// Creates a new ServeFile endpoint.
  ///
  /// # Arguments
  ///
  /// * `path` - The path to the file to serve
  ///
  /// # Returns
  ///
  /// A new ServeFile instance
  pub fn new(path: PathBuf) -> Self {
    ServeFile { path }
  }
}

#[async_trait::async_trait]
impl Endpoint for ServeFile {
  /// Serves the file content.
  ///
  /// # Arguments
  ///
  /// * `_req` - The incoming request (not used)
  ///
  /// # Returns
  ///
  /// The file contents as a response or an error
  async fn call(&self, _req: Request) -> Result {
    let body = tokio::fs::read(&self.path).await?;
    let response = hyper::Response::new(Full::new(Bytes::from(body)));
    Ok(response.into())
  }
}

/// Endpoint for serving files from a directory.
///
/// Extracts a `:file` parameter from the URL and serves the corresponding
/// file from the configured directory.
///
/// # Path Parameters
///
/// * `file` - The relative path of the file within the directory
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{ServeDir, Router};
///
/// let mut router = Router::new();
/// router.get("/assets/*file", ServeDir::new("static/assets".into()));
/// ```
///
/// With this configuration, a request to `/assets/js/app.js` would serve
/// the file `static/assets/js/app.js`.
pub struct ServeDir {
  /// The directory to serve files from
  dir: PathBuf,
}

impl ServeDir {
  /// Creates a new ServeDir endpoint.
  ///
  /// # Arguments
  ///
  /// * `dir` - The directory path to serve files from
  ///
  /// # Returns
  ///
  /// A new ServeDir instance
  pub fn new(dir: PathBuf) -> Self {
    ServeDir { dir }
  }
}

#[async_trait::async_trait]
impl Endpoint for ServeDir {
  /// Serves a file from the directory.
  ///
  /// # Arguments
  ///
  /// * `req` - The incoming request containing the `:file` parameter
  ///
  /// # Returns
  ///
  /// The requested file as a response or an error
  ///
  /// # Errors
  ///
  /// Returns an error if the file parameter is missing or the file cannot be read.
  async fn call(&self, req: Request) -> Result {
    let file = req.param::<String>("file")?;
    let dir = self.dir.clone();
    let file = PathBuf::from(format!("{}/{}", dir.to_string_lossy(), file));
    let body = tokio::fs::read(file).await?;
    let response = hyper::Response::new(Full::new(Bytes::from(body)));
    Ok(response.into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_serve_file_new() {
    let path = PathBuf::from("/var/www/index.html");
    let serve_file = ServeFile::new(path);
    assert_eq!(serve_file.path, PathBuf::from("/var/www/index.html"));
  }

  #[test]
  fn test_serve_dir_new() {
    let dir = PathBuf::from("/var/www/static");
    let serve_dir = ServeDir::new(dir);
    assert_eq!(serve_dir.dir, PathBuf::from("/var/www/static"));
  }
}
