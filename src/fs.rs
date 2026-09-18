use crate::body::Body;
use crate::{Endpoint, Request, Response, Result};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::header;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

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
  options: ServeOptions,
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
    ServeFile {
      path,
      options: ServeOptions::default(),
    }
  }

  /// Emits a `Cache-Control` header on responses.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// ServeFile::new(path).cache_control("public, max-age=3600")
  /// ```
  #[must_use]
  pub fn cache_control(mut self, value: &str) -> Self {
    self.options.cache_control = Some(value.to_string());
    self
  }

  /// Serves precompressed `.gz`/`.br` siblings when the client accepts
  /// them (brotli preferred). Responses gain `Content-Encoding` and
  /// `Vary: Accept-Encoding`.
  #[must_use]
  pub fn precompressed(mut self, yes: bool) -> Self {
    self.options.precompressed = yes;
    self
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
  /// The file contents as a response with `Content-Type`, `ETag`, and
  /// `Last-Modified` headers, or `304 Not Modified` when conditional request
  /// headers allow it, or an error
  async fn call(&self, req: Request) -> Result {
    serve_file_with_cache(&req, &self.path, &self.options).await
  }
}

/// Per-endpoint serving options (builder-configured).
#[derive(Clone, Debug, Default)]
struct ServeOptions {
  /// Emits `Cache-Control` on 200/304 when set.
  cache_control: Option<String>,
  /// Serves `.br`/`.gz` siblings with `Content-Encoding` when the client
  /// accepts them.
  precompressed: bool,
}

/// Returns true when the request's `Accept-Encoding` mentions `token`.
fn accepts_encoding(req: &Request, token: &str) -> bool {
  req
    .header("accept-encoding")
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| {
      v.split(',')
        .any(|part| part.trim().to_ascii_lowercase().starts_with(token))
    })
}

/// Builds a response for the file at `path`, honoring HTTP conditional
/// requests.
///
/// Always sets `Content-Type` (from the extension), `ETag`
/// (`W/"{mtime_secs:x}-{size:x}"`), and `Last-Modified`. When the request's
/// `If-None-Match` matches the ETag (or `If-Modified-Since` is not earlier
/// than the file's modification time and no `If-None-Match` is present),
/// responds `304 Not Modified` with an empty body.
async fn serve_file_with_cache(req: &Request, path: &Path, options: &ServeOptions) -> Result {
  // A missing file is a client-visible 404, not a server error.
  let (mut file, meta) = match open_for_serve(path).await {
    Ok(opened) => opened,
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
      return Ok(Response::with_status_code(
        hyper::StatusCode::NOT_FOUND,
        "not found".to_string(),
      ));
    }
    Err(err) => return Err(err.into()),
  };
  let modified = meta.modified()?;
  let mtime_secs = modified
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  let etag = format!("W/\"{:x}-{:x}\"", mtime_secs, meta.len());
  let last_modified = httpdate::fmt_http_date(modified);

  // Precompressed siblings: prefer brotli, then gzip. The Content-Type and
  // validators stay those of the plain file (same logical resource).
  let mut encoding: Option<&'static str> = None;
  if options.precompressed {
    for (ext, token) in [(".br", "br"), (".gz", "gzip")] {
      if !accepts_encoding(req, token) {
        continue;
      }
      let mut sibling = path.as_os_str().to_os_string();
      sibling.push(ext);
      if let Ok(compressed) = tokio::fs::File::open(sibling).await {
        file = compressed;
        encoding = Some(token);
        break;
      }
    }
  }

  let not_modified = is_not_modified(req, &etag, modified);
  if not_modified {
    let mut builder = hyper::Response::builder()
      .status(hyper::StatusCode::NOT_MODIFIED)
      .header(header::ETAG, etag.as_str())
      .header(header::LAST_MODIFIED, last_modified);
    if let Some(cache_control) = &options.cache_control {
      builder = builder.header(header::CACHE_CONTROL, cache_control.clone());
    }
    if let Some(enc) = encoding {
      builder = builder.header(header::CONTENT_ENCODING, enc);
    }
    if options.precompressed {
      builder = builder.header(header::VARY, "Accept-Encoding");
    }
    let response = builder.body(Body::empty())?;
    return Ok(response.into());
  }

  let len = file.metadata().await?.len();
  let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
  let body = Body::Streaming(BoxBody::new(SizedBody {
    inner: Body::stream(stream),
    len,
  }));
  let mime = mime_for_path(path);
  let mut builder = hyper::Response::builder()
    .header(header::CONTENT_TYPE, mime)
    .header(header::ETAG, etag.as_str())
    .header(header::LAST_MODIFIED, last_modified);
  if let Some(cache_control) = &options.cache_control {
    builder = builder.header(header::CACHE_CONTROL, cache_control.clone());
  }
  if let Some(enc) = encoding {
    builder = builder.header(header::CONTENT_ENCODING, enc);
  }
  if options.precompressed {
    builder = builder.header(header::VARY, "Accept-Encoding");
  }
  let response = builder.body(body)?;
  Ok(response.into())
}

/// Wraps a streaming body whose exact length is known up front (from
/// `fstat`), so hyper can send `Content-Length` instead of chunked encoding.
struct SizedBody<B> {
  inner: B,
  len: u64,
}

impl<B> http_body::Body for SizedBody<B>
where
  B: http_body::Body<Data = Bytes, Error = crate::body::BoxError> + Unpin,
{
  type Data = Bytes;
  type Error = crate::body::BoxError;

  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.get_mut().inner).poll_frame(cx)
  }

  fn size_hint(&self) -> http_body::SizeHint {
    http_body::SizeHint::with_exact(self.len)
  }
}

/// Evaluates the request's conditional headers against the file state.
fn is_not_modified(req: &Request, etag: &str, modified: SystemTime) -> bool {
  // If-None-Match takes precedence over If-Modified-Since.
  if let Some(inm) = req.header("if-none-match").and_then(|v| v.to_str().ok()) {
    return inm.split(',').any(|candidate| {
      let candidate = candidate.trim();
      candidate == "*" || candidate == etag || candidate == etag.trim_start_matches("W/")
    });
  }
  if let Some(ims) = req
    .header("if-modified-since")
    .and_then(|v| v.to_str().ok())
    && let Ok(since) = httpdate::parse_http_date(ims)
  {
    return modified <= since;
  }
  false
}

/// Opens `path` for serving, falling back to `path/index.html` when the
/// path resolves to a directory (directory-index fallback).
///
/// A regular file costs a single `open` + `fstat`; the extra open happens
/// only for directory hits.
async fn open_for_serve(path: &Path) -> std::io::Result<(tokio::fs::File, std::fs::Metadata)> {
  let file = tokio::fs::File::open(path).await?;
  let meta = file.metadata().await?;
  if meta.is_dir() {
    let file = tokio::fs::File::open(path.join("index.html")).await?;
    let meta = file.metadata().await?;
    Ok((file, meta))
  } else {
    Ok((file, meta))
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
/// # Directory Index
///
/// When the resolved path is a directory, `index.html` inside it is served
/// instead. An empty `:file` parameter therefore serves the directory's
/// `index.html`.
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
  options: ServeOptions,
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
    ServeDir {
      dir,
      options: ServeOptions::default(),
    }
  }

  /// Emits a `Cache-Control` header on responses.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// ServeDir::new(dir).cache_control("public, max-age=31536000, immutable")
  /// ```
  #[must_use]
  pub fn cache_control(mut self, value: &str) -> Self {
    self.options.cache_control = Some(value.to_string());
    self
  }

  /// Serves precompressed `.gz`/`.br` siblings when the client accepts
  /// them (brotli preferred). Responses gain `Content-Encoding` and
  /// `Vary: Accept-Encoding`.
  #[must_use]
  pub fn precompressed(mut self, yes: bool) -> Self {
    self.options.precompressed = yes;
    self
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
  /// The requested file as a response with `Content-Type`, `ETag`, and
  /// `Last-Modified` headers, or `304 Not Modified` when conditional request
  /// headers allow it
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
      None => {
        return Ok(Response::with_status_code(
          hyper::StatusCode::FORBIDDEN,
          "Forbidden".to_string(),
        ));
      }
    };
    serve_file_with_cache(&req, &resolved, &self.options).await
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

  #[tokio::test]
  async fn test_open_for_serve_falls_back_to_index_html() {
    let base = std::env::temp_dir().join(format!("desirable-dir-index-{}", std::process::id()));
    let subdir = base.join("site");
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(subdir.join("index.html"), "<h1>hi</h1>").unwrap();

    // A directory opens its index.html.
    let (file, meta) = open_for_serve(&subdir).await.unwrap();
    assert!(!meta.is_dir());
    drop(file);

    // A file opens directly.
    let file_path = base.join("app.js");
    std::fs::write(&file_path, b"console.log(1)").unwrap();
    let (_, meta) = open_for_serve(&file_path).await.unwrap();
    assert_eq!(meta.len(), 14);

    std::fs::remove_dir_all(&base).ok();
  }
}
