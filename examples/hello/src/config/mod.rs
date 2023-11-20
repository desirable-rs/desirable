use once_cell::sync::Lazy;
use std::env;
pub static ENV_NAME: Lazy<String> =
  Lazy::new(|| env::var("ENV_NAME").expect("ENV_NAME must be set"));
