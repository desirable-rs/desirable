mod config;
mod controller;
mod error;
mod middleware;
mod model;
mod service;
mod types;
use config::ENV_NAME;
use desirable::{ServeDir, ServeFile};
use std::env;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
#[tokio::main]
async fn main() -> desirable::Result<()> {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
  let arguments: Vec<String> = env::args().collect();
  let env_name = arguments.get(1).expect("env name must be provided");
  let env_file = format!("env/{}.env", env_name);
  dotenv::from_filename(env_file).ok();

  info!("ENV_NAME: {}", ENV_NAME.to_string());
  let mut app = desirable::Router::new();
  app.with(middleware::Logger);
  app.get("/", ServeFile::new("dist/index.html".into()));
  app.get("/assets/:file", ServeDir::new("dist/assets".into()));
  app.get("/hello", controller::hello);
  app.get("/hello1", controller::hello1);
  app.get("/error", controller::error);
  app.get("/user", controller::get_users);
  app.get("/query", controller::get_query);
  app.get("/user/:id", controller::get_user_by_id);
  app.post("/user", controller::create_users);

  let addr = "127.0.0.1:1337";
  let serve = desirable::new(addr);
  serve.run(app).await?;
  info!("hello");
  Ok(())
}
