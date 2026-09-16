use crate::{Endpoint, Request, Response, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::header;
use std::path::{Component, Path, PathBuf};

/// Returns the MIME type for a file based on its extension.
///
/// Covers common web file types; falls back to `application/octet-stream`.
fn mime_for_path(path: &Path) -> &'static str {
  let ext = path
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or_default();
  match ext.to_ascii_lowercase().as_str() {
    "html" | "htm" => "text/html; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "json" => "application/json",
    "txt" => "text/plain; charset=utf-8",
    "xml" => "application/xml",
    "pdf" => "application/pdf",
    "wasm" => "application/wasm",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "svg" => "image/svg+xml",
    "webp" => "image/webp",
    "ico" => "image/x-icon",
    "woff" => "font/woff",
    "woff2" => "font/woff2",
    "ttf" => "font/ttf",
    "otf" => "font/otf",
    "mp3" => "audio/mpeg",
    "mp4" => "video/mp4",
    "webm" => "video/webm",
    _ => "application/octet-stream",
  }
}

/// Normalizes a user-supplied relative path and ensures it stays within `base`.
///
/// Rejects paths containing `..` components that would escape the base
/// directory (path traversal attacks). Returns `None` when the path escapes.
fn resolve_within(base: &Path, relative: &str) -> Option<PathBuf> {
  let relative_path = Path::new(relative);
  let mut resolved = base.to_path_buf();
  for component in relative_path.components() {
    match component {
      Component::Normal(part) => resolved.push(part),
      Component::CurDir => {}
      // ParentDir, RootDir, Prefix would escape (or replace) the base dir.
      _ => return None,
    }
  }
  Some(resolved)
}

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
  /// The file contents as a response with a Content-Type header inferred
  /// from the file extension, or an error
  async fn call(&self, _req: Request) -> Result {
    let body = tokio::fs::read(&self.path).await?;
    let mime = mime_for_path(&self.path);
    let response = hyper::Response::builder()
      .header(header::CONTENT_TYPE, mime)
      .body(Full::new(Bytes::from(body)))?;
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
/// # Security
///
/// Path traversal is blocked: requests whose `:file` parameter contains
/// `..` components (e.g. `/assets/../../etc/passwd`) receive `403 Forbidden`.
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
/// the file `static/assets/js/app.js` with content type `text/javascript`.
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
  /// The requested file as a response with an inferred Content-Type header
  ///
  /// # Errors
  ///
  /// Returns a 403 response when the file parameter attempts to escape the
  /// directory, and an error if the file parameter is missing or the file
  /// cannot be read.
  async fn call(&self, req: Request) -> Result {
    let file = req.param_str("file")?;
    let resolved = match resolve_within(&self.dir, &file) {
      Some(path) => path,
      None => return Response::with_status(403, "Forbidden".to_string()),
    };
    let body = tokio::fs::read(resolved.clone()).await?;
    let mime = mime_for_path(&resolved);
    let response = hyper::Response::builder()
      .header(header::CONTENT_TYPE, mime)
      .body(Full::new(Bytes::from(body)))?;
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

  #[test]
  fn test_mime_for_path() {
    assert_eq!(
      mime_for_path(Path::new("a.html")),
      "text/html; charset=utf-8"
    );
    assert_eq!(mime_for_path(Path::new("a.CSS")), "text/css; charset=utf-8");
    assert_eq!(
      mime_for_path(Path::new("a.js")),
      "text/javascript; charset=utf-8"
    );
    assert_eq!(mime_for_path(Path::new("a.json")), "application/json");
    assert_eq!(mime_for_path(Path::new("a.png")), "image/png");
    assert_eq!(mime_for_path(Path::new("a.SVG")), "image/svg+xml");
    assert_eq!(mime_for_path(Path::new("a.woff2")), "font/woff2");
    assert_eq!(
      mime_for_path(Path::new("a.unknownext")),
      "application/octet-stream"
    );
    assert_eq!(
      mime_for_path(Path::new("noext")),
      "application/octet-stream"
    );
  }

  #[test]
  fn test_resolve_within_accepts_normal() {
    let base = PathBuf::from("/var/www/static");
    let resolved = resolve_within(&base, "js/app.js").unwrap();
    assert_eq!(resolved, PathBuf::from("/var/www/static/js/app.js"));
  }

  #[test]
  fn test_resolve_within_allows_cur_dir() {
    let base = PathBuf::from("/var/www/static");
    let resolved = resolve_within(&base, "./js/app.js").unwrap();
    assert_eq!(resolved, PathBuf::from("/var/www/static/js/app.js"));
  }

  #[test]
  fn test_resolve_within_rejects_traversal() {
    let base = PathBuf::from("/var/www/static");
    assert!(resolve_within(&base, "../secrets.txt").is_none());
    assert!(resolve_within(&base, "js/../../../etc/passwd").is_none());
    // On Windows, backslashes separate path components; on Unix they are
    // just characters within a (harmless) filename.
    if cfg!(windows) {
      assert!(resolve_within(&base, "..\\windows\\system32").is_none());
    }
  }

  #[test]
  fn test_resolve_within_rejects_absolute() {
    let base = PathBuf::from("/var/www/static");
    // On unix, "/etc/passwd" has a RootDir component and must be rejected.
    assert!(resolve_within(&base, "/etc/passwd").is_none());
  }
}
