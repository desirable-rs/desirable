use desirable::Result;
pub struct Logger;
#[async_trait::async_trait]
impl desirable::Middleware for Logger {
  async fn handle(&self, req: desirable::Request, next: desirable::Next<'_>) -> Result {
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let res = next.run(req).await?;
    let status = res.status().to_string();
    let log = format!("{} {} {}", method, uri, status);
    println!("{}", log);
    Ok(res)
  }
}
