//! Static file serving: directory listing fallback, caching, ranges.
//!
//! Run: `cargo run -p example-static-files`, then
//!
//! ```sh
//! curl -i http://127.0.0.1:3000/static/          # index.html (dir fallback)
//! curl -i -H "Range: bytes=0-4" http://127.0.0.1:3000/static/hello.txt   # 206
//! curl -i -H "If-None-Match: <etag>" http://127.0.0.1:3000/static/hello.txt
//! ```
//!
//! Security is built in: `..` traversal gets 403, symlinks pointing outside
//! the base directory get 404. Files stream with an exact Content-Length.

use desirable::{Router, ServeDir};

#[tokio::main]
async fn main() -> desirable::Result<()> {
  // A self-contained demo directory; point ServeDir at your real assets.
  let dir = std::env::temp_dir().join("desirable-static-demo");
  std::fs::create_dir_all(&dir)?;
  std::fs::write(dir.join("index.html"), "<h1>static-files demo</h1>")?;
  std::fs::write(dir.join("hello.txt"), "hello world")?;
  println!("serving {}", dir.display());

  let mut app = Router::new();
  app.get(
    "/static/*file",
    ServeDir::new(dir)
      .cache_control("public, max-age=3600")
      // Serve sibling `file.br`/`file.gz` when present and accepted.
      .precompressed(true),
  );

  desirable::new("127.0.0.1:3000").run(app).await
}
