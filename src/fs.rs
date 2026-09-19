use crate::body::Body;
use crate::{Endpoint, Request, Response, Result};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::header;
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

/// Returns the MIME type for a file based on its extension.
///
/// Covers common web file types; falls back to `application/octet-stream`.
/// Already-lowercase extensions (the common case) are matched without
/// allocating; only mixed-case extensions pay for a lowercase copy.
fn mime_for_path(path: &Path) -> &'static str {
  let ext = path
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or_default();
  let ext: Cow<'_, str> = if ext.bytes().any(|b| b.is_ascii_uppercase()) {
    Cow::Owned(ext.to_ascii_lowercase())
  } else {
    Cow::Borrowed(ext)
  };
  match ext.as_ref() {
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
    self.options.cache_control = Some(parse_cache_control(value));
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

  /// Uses a strong content-hash ETag instead of the default weak
  /// `mtime`+`size` validator. The hash is computed once per file version
  /// and cached.
  #[must_use]
  pub fn strong_etag(mut self, yes: bool) -> Self {
    self.options.strong_etag = yes;
    self
  }
}

/// Parses a configured `Cache-Control` value once, at construction time.
fn parse_cache_control(value: &str) -> header::HeaderValue {
  value
    .parse()
    .unwrap_or_else(|_| panic!("invalid Cache-Control value: {value:?}"))
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

/// Per-endpoint serving options (builder-configured). Parsed eagerly so the
/// request path never validates or re-parses configuration values.
#[derive(Clone, Debug, Default)]
struct ServeOptions {
  /// Emits `Cache-Control` on 200/304 when set.
  cache_control: Option<header::HeaderValue>,
  /// Serves `.br`/`.gz` siblings with `Content-Encoding` when the client
  /// accepts them.
  precompressed: bool,
  /// Strong content-hash ETag instead of the default weak validator.
  strong_etag: bool,
}

/// Case-insensitive ASCII `starts_with` for a trimmed string — no
/// allocation (header paths call this per request).
fn starts_with_ignore_case(part: &str, prefix: &str) -> bool {
  let part = part.trim();
  part.len() >= prefix.len()
    && part.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Returns true when the request's `Accept-Encoding` mentions `token`.
fn accepts_encoding(req: &Request, token: &str) -> bool {
  req
    .header("accept-encoding")
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| {
      v.split(',')
        .any(|part| starts_with_ignore_case(part, token))
    })
}

/// The outcome of parsing a `Range` header against a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteRange {
  /// No usable `Range` header: serve the full body with 200.
  None,
  /// Serve `len` bytes starting at `start` with 206.
  Satisfiable(u64, u64),
  /// The range is outside the resource: 416.
  Unsatisfiable,
}

/// Parses a single-range `bytes=` header (start-end, start-, or -suffix).
///
/// Multi-range and non-`bytes` units are not supported and yield
/// [`ByteRange::None`] (a full 200 response), which RFC 9110 permits.
fn parse_byte_range(header: &str, total: u64) -> ByteRange {
  let Some(spec) = header.strip_prefix("bytes=") else {
    return ByteRange::None;
  };
  if spec.contains(',') {
    return ByteRange::None;
  }
  let Some((start_str, end_str)) = spec.split_once('-') else {
    return ByteRange::None;
  };
  match (start_str.is_empty(), end_str.is_empty()) {
    // bytes=-N: final N bytes
    (true, false) => {
      let Ok(n) = end_str.parse::<u64>() else {
        return ByteRange::None;
      };
      if n == 0 || total == 0 {
        return ByteRange::Unsatisfiable;
      }
      let start = total.saturating_sub(n);
      ByteRange::Satisfiable(start, total - start)
    }
    // bytes=start-end / bytes=start-
    (false, _) => {
      let Ok(start) = start_str.parse::<u64>() else {
        return ByteRange::None;
      };
      let end = if end_str.is_empty() {
        total.saturating_sub(1)
      } else {
        match end_str.parse::<u64>() {
          Ok(v) => v,
          Err(_) => return ByteRange::None,
        }
      };
      if end < start || start >= total {
        return ByteRange::Unsatisfiable;
      }
      ByteRange::Satisfiable(start, end.min(total - 1) - start + 1)
    }
    (true, true) => ByteRange::None,
  }
}

/// `If-Range` gate: when the header is present it must match the current
/// ETag, otherwise the `Range` header is ignored.
fn if_range_allows(req: &Request, etag: &str) -> bool {
  match req.header("if-range").and_then(|v| v.to_str().ok()) {
    None => true,
    Some(if_range) => if_range == etag || if_range == etag.trim_start_matches("W/"),
  }
}

/// Builds a response for the file at `path`, honoring HTTP conditional
/// requests.
///
/// Always sets `Content-Type` (from the extension), `ETag`
/// (`W/"{mtime_secs:x}-{size:x}"`), and `Last-Modified`. When the request's
/// `If-None-Match` matches the ETag (or `If-Modified-Since` is not earlier
/// than the file's modification time and no `If-None-Match` is present),
/// responds `304 Not Modified` with an empty body.
/// Applies the validators/cache/encoding headers shared by every file
/// response (200/206/304/416).
fn common_headers(
  mut builder: hyper::http::response::Builder,
  etag: &str,
  last_modified: &str,
  options: &ServeOptions,
  encoding: Option<&'static str>,
) -> hyper::http::response::Builder {
  builder = builder
    .header(header::ETAG, etag)
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
  builder
}

/// Computes (and caches by path+mtime+size) a strong content-hash ETag.
///
/// The first request for each file version reads the whole file; later
/// requests reuse the cache until the file changes.
async fn strong_etag_for(
  path: &Path,
  file: &mut tokio::fs::File,
  meta: &std::fs::Metadata,
) -> std::io::Result<String> {
  use sha2::{Digest, Sha256};
  use std::collections::HashMap;
  use std::sync::{Mutex, OnceLock};

  type EtagCache = Mutex<HashMap<PathBuf, (u64, u64, String)>>;

  static CACHE: OnceLock<EtagCache> = OnceLock::new();
  let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

  let mtime_secs = meta
    .modified()?
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  let size = meta.len();

  if let Some((m, l, tag)) = cache.lock().unwrap().get(path)
    && *m == mtime_secs
    && *l == size
  {
    return Ok(tag.clone());
  }

  let mut buf = Vec::with_capacity(size as usize);
  file.seek(std::io::SeekFrom::Start(0)).await?;
  file.read_to_end(&mut buf).await?;
  file.seek(std::io::SeekFrom::Start(0)).await?;

  let digest = Sha256::digest(&buf);
  let mut hex = String::with_capacity(32);
  for byte in &digest[..16] {
    use std::fmt::Write as _;
    let _ = write!(hex, "{byte:02x}");
  }
  let tag = format!("\"{}-{}\"", hex, size);

  {
    let mut map = cache.lock().unwrap();
    if map.len() >= 1024 {
      map.clear();
    }
    map.insert(path.to_path_buf(), (mtime_secs, size, tag.clone()));
  }
  Ok(tag)
}

async fn serve_file_with_cache(req: &Request, path: &Path, options: &ServeOptions) -> Result {
  // A missing file is a client-visible 404, not a server error.
  let (mut file, meta, index_fallback) = match open_for_serve(path).await {
    Ok(opened) => opened,
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
      return Ok(Response::static_text(
        hyper::StatusCode::NOT_FOUND,
        "not found",
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
  // validators stay those of the plain file (same logical resource). A
  // directory hit (already swapped to its index.html) has no siblings.
  let mut encoding: Option<&'static str> = None;
  if options.precompressed && !index_fallback {
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

  let etag = if options.strong_etag {
    strong_etag_for(path, &mut file, &meta).await?
  } else {
    etag
  };

  // Conditional-request methods: If-None-Match/If-Modified-Since produce
  // 304 only for GET/HEAD; for other methods a matching If-None-Match is a
  // 412 (RFC 9110 §13.1.2), and If-Modified-Since is ignored (§13.1.3).
  let is_get_or_head = req.method() == hyper::Method::GET || req.method() == hyper::Method::HEAD;
  let not_modified = is_get_or_head && is_not_modified(req, &etag, modified);
  let precondition_failed = !is_get_or_head && is_not_modified(req, &etag, modified);
  if not_modified {
    let response = common_headers(
      hyper::Response::builder().status(hyper::StatusCode::NOT_MODIFIED),
      etag.as_str(),
      &last_modified,
      options,
      encoding,
    )
    .body(Body::empty())?;
    return Ok(response.into());
  }

  if precondition_failed {
    let response = common_headers(
      hyper::Response::builder().status(hyper::StatusCode::PRECONDITION_FAILED),
      etag.as_str(),
      &last_modified,
      options,
      encoding,
    )
    .body(Body::empty())?;
    return Ok(response.into());
  }

  // Length of the representation actually served: the fstat from
  // `open_for_serve` covers the plain file; a precompressed sibling (a
  // different inode) needs one fresh fstat.
  let served_len = match encoding {
    Some(_) => file.metadata().await?.len(),
    None => meta.len(),
  };

  // Single-range requests (206/416). Ranges apply to the representation
  // actually served (a precompressed sibling, when selected).
  // Range handling is defined for GET only (RFC 9110 §14.2): other methods
  // get the full representation.
  let serve_range = req.method() == hyper::Method::GET;
  let mut partial: Option<(u64, u64)> = None; // (start, len)
  let mut unsatisfiable = false;
  if serve_range
    && let Some(spec) = req.header("range").and_then(|v| v.to_str().ok())
    && if_range_allows(req, &etag)
  {
    match parse_byte_range(spec, served_len) {
      ByteRange::Satisfiable(start, len) => {
        if file.seek(std::io::SeekFrom::Start(start)).await.is_ok() {
          partial = Some((start, len));
        }
      }
      ByteRange::Unsatisfiable => unsatisfiable = true,
      ByteRange::None => {}
    }
  }

  if unsatisfiable {
    let response = common_headers(
      hyper::Response::builder().status(hyper::StatusCode::RANGE_NOT_SATISFIABLE),
      etag.as_str(),
      &last_modified,
      options,
      encoding,
    )
    .header(header::CONTENT_RANGE, format!("bytes */{}", served_len))
    .body(Body::empty())?;
    return Ok(response.into());
  }

  // Stream the file: memory use is one chunk, not the whole file. The exact
  // length from fstat is preserved as the size hint so hyper sends
  // Content-Length instead of chunked encoding.
  let (status, len, content_range) = match partial {
    Some((start, len)) => (
      hyper::StatusCode::PARTIAL_CONTENT,
      len,
      Some(format!(
        "bytes {}-{}/{}",
        start,
        start + len - 1,
        served_len
      )),
    ),
    None => (hyper::StatusCode::OK, served_len, None),
  };
  let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
  let body = Body::Streaming(BoxBody::new(SizedBody {
    inner: Body::stream(stream),
    len,
  }));
  let mime = if index_fallback {
    mime_for_path(&path.join("index.html"))
  } else {
    mime_for_path(path)
  };
  let mut builder = common_headers(
    hyper::Response::builder().status(status),
    etag.as_str(),
    &last_modified,
    options,
    encoding,
  )
  .header(header::CONTENT_TYPE, mime);
  if let Some(cr) = content_range {
    builder = builder.header(header::CONTENT_RANGE, cr);
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
async fn open_for_serve(
  path: &Path,
) -> std::io::Result<(tokio::fs::File, std::fs::Metadata, bool)> {
  // Reject symlinks at the final component: uploaded or planted links must
  // not serve files from outside the base directory. (lstat does not follow
  // the last component; the handle-based fstat below still gives the real
  // file's metadata afterwards.)
  let meta = tokio::fs::symlink_metadata(path).await?;
  if meta.file_type().is_symlink() {
    return Err(std::io::Error::new(
      std::io::ErrorKind::NotFound,
      "symlinks are not served",
    ));
  }
  let file = tokio::fs::File::open(path).await?;
  let meta = file.metadata().await?;
  if meta.is_dir() {
    let index = path.join("index.html");
    let meta = tokio::fs::symlink_metadata(&index).await?;
    if meta.file_type().is_symlink() {
      return Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "symlinks are not served",
      ));
    }
    let file = tokio::fs::File::open(&index).await?;
    let meta = file.metadata().await?;
    Ok((file, meta, true))
  } else {
    Ok((file, meta, false))
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
#[derive(Clone)]
pub struct ServeDir {
  /// The directory to serve files from
  dir: PathBuf,
  /// Symlink-resolved `dir`, computed once (at construction, or on the
  /// first request when the directory did not exist yet); requests are
  /// required to canonicalize into this base.
  canonical_base: std::sync::OnceLock<PathBuf>,
  options: ServeOptions,
}

impl ServeDir {
  /// Creates a new ServeDir endpoint.
  ///
  /// The base directory is resolved (symlinks followed) once here; if it
  /// does not exist yet, resolution is retried on the first request.
  ///
  /// # Arguments
  ///
  /// * `dir` - The directory path to serve files from
  ///
  /// # Returns
  ///
  /// A new ServeDir instance
  pub fn new(dir: PathBuf) -> Self {
    let canonical_base = std::sync::OnceLock::new();
    if let Ok(resolved) = dir.canonicalize() {
      let _ = canonical_base.set(resolved);
    }
    ServeDir {
      dir,
      canonical_base,
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
    self.options.cache_control = Some(parse_cache_control(value));
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

  /// Uses a strong content-hash ETag instead of the default weak
  /// `mtime`+`size` validator. The hash is computed once per file version
  /// and cached.
  #[must_use]
  pub fn strong_etag(mut self, yes: bool) -> Self {
    self.options.strong_etag = yes;
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
        return Ok(Response::static_text(
          hyper::StatusCode::FORBIDDEN,
          "Forbidden",
        ));
      }
    };

    // Symlink hardening: fully resolve the path (following any directory or
    // file symlinks) and require it to stay inside the canonical base
    // directory. Catches planted directory symlinks that the final-
    // component lstat check cannot see. The base is resolved once (cached),
    // so the steady-state cost is a single canonicalize per request.
    if let Ok(actual) = resolved.canonicalize() {
      let base = self
        .canonical_base
        .get_or_init(|| self.dir.canonicalize().unwrap_or_else(|_| self.dir.clone()));
      if !actual.starts_with(base) {
        return Ok(Response::static_text(
          hyper::StatusCode::NOT_FOUND,
          "not found",
        ));
      }
    }

    serve_file_with_cache(&req, &resolved, &self.options).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_starts_with_ignore_case() {
    assert!(starts_with_ignore_case("br", "br"));
    assert!(starts_with_ignore_case("BR", "br"));
    assert!(starts_with_ignore_case(" GZip ; q=0.1", "gzip"));
    assert!(!starts_with_ignore_case("b", "br"));
    assert!(!starts_with_ignore_case("deflate", "gzip"));
  }

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

  #[test]
  fn test_parse_byte_range() {
    use super::ByteRange;
    use super::parse_byte_range;

    let total = 100;
    // Full forms
    assert_eq!(
      parse_byte_range("bytes=0-9", total),
      ByteRange::Satisfiable(0, 10)
    );
    assert_eq!(
      parse_byte_range("bytes=10-", total),
      ByteRange::Satisfiable(10, 90)
    );
    assert_eq!(
      parse_byte_range("bytes=-5", total),
      ByteRange::Satisfiable(95, 5)
    );
    // End beyond the resource clamps to the last byte.
    assert_eq!(
      parse_byte_range("bytes=95-200", total),
      ByteRange::Satisfiable(95, 5)
    );
    // Unsatisfiable
    assert_eq!(
      parse_byte_range("bytes=100-", total),
      ByteRange::Unsatisfiable
    );
    assert_eq!(
      parse_byte_range("bytes=-0", total),
      ByteRange::Unsatisfiable
    );
    assert_eq!(
      parse_byte_range("bytes=5-4", total),
      ByteRange::Unsatisfiable
    );
    assert_eq!(parse_byte_range("bytes=-5", 0), ByteRange::Unsatisfiable);
    // Unsupported forms: full 200
    assert_eq!(parse_byte_range("bytes=0-4,10-19", total), ByteRange::None);
    assert_eq!(parse_byte_range("items=0-9", total), ByteRange::None);
    assert_eq!(parse_byte_range("garbage", total), ByteRange::None);
  }

  #[tokio::test]
  async fn test_open_for_serve_falls_back_to_index_html() {
    let base = std::env::temp_dir().join(format!("desirable-dir-index-{}", std::process::id()));
    let subdir = base.join("site");
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(subdir.join("index.html"), "<h1>hi</h1>").unwrap();

    // A directory opens its index.html.
    let (file, meta, _fallback) = open_for_serve(&subdir).await.unwrap();
    assert!(!meta.is_dir());
    drop(file);

    // A file opens directly.
    let file_path = base.join("app.js");
    std::fs::write(&file_path, b"console.log(1)").unwrap();
    let (_, meta, _fallback) = open_for_serve(&file_path).await.unwrap();
    assert_eq!(meta.len(), 14);

    std::fs::remove_dir_all(&base).ok();
  }
}
