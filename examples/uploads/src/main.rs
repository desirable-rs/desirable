//! File uploads: multipart/form-data streamed to disk, plus a raw-body
//! streaming endpoint.
//!
//! Run: `cargo run -p example-uploads`, then
//!
//! ```sh
//! curl -F note=hello -F file=@somefile.bin http://127.0.0.1:3000/upload
//! curl -X POST --data-binary @bigfile http://127.0.0.1:3000/raw
//! ```
//!
//! Neither endpoint buffers the upload in memory: multipart fields and raw
//! bodies are consumed chunk by chunk (the default 2 MiB body limit still
//! applies — raise it with `Server::body_limit` for big uploads).

use desirable::{Request, Response, Result, Router};
use tokio::io::AsyncWriteExt as _;

#[tokio::main]
async fn main() -> Result<()> {
  let dir = std::env::temp_dir().join("desirable-uploads");
  tokio::fs::create_dir_all(&dir).await?;
  println!("storing uploads in {}", dir.display());

  let mut app = Router::new();

  // multipart/form-data: form fields + files, streamed field by field.
  let upload_dir = dir.clone();
  app.post("/upload", move |mut req: Request| {
    let dir = upload_dir.clone();
    async move {
      let mut mp = req.multipart()?;
      let mut saved = Vec::new();
      while let Some(mut field) = mp.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        let filename = field.file_name().map(str::to_string);
        match (name.as_str(), filename) {
          // A plain form value: buffer it (small by nature).
          ("note", _) => {
            let note = field.text().await?;
            saved.push(format!("note={note}"));
          }
          // A file: stream chunks straight to disk.
          (name, Some(filename)) => {
            let path = dir.join(format!("{name}-{filename}"));
            let mut file = tokio::fs::File::create(&path).await?;
            let mut written = 0u64;
            while let Some(chunk) = field.chunk().await? {
              file.write_all(&chunk).await?;
              written += chunk.len() as u64;
            }
            file.flush().await?;
            saved.push(format!("{filename} -> {} ({written} bytes)", path.display()));
          }
          _ => {}
        }
      }
      Ok::<_, desirable::Error>(Response::json(serde_json::json!({ "saved": saved })))
    }
  });

  // Raw-body streaming: any content type, straight to disk.
  let raw_dir = dir.clone();
  app.post("/raw", move |mut req: Request| {
    let dir = raw_dir.clone();
    async move {
      let path = dir.join("raw-body.bin");
      let written = req.save_body_to(&path).await?;
      Ok::<_, desirable::Error>(
        Response::builder().text(format!("{written} bytes -> {}", path.display())),
      )
    }
  });

  desirable::new("127.0.0.1:3000").run(app).await
}
