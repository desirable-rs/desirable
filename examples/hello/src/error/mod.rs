use desirable::IntoResponse;
use desirable::Response;
use desirable::Result;
use thiserror::Error;

#[derive(Debug, Error)]
#[expect(clippy::enum_variant_names, reason = "variant names read naturally at call sites")]
pub enum Error {
  #[error("hyper error")]
  DesirableError(#[from] desirable::Error),
  #[error("json error")]
  JsonError(#[from] serde_json::Error),
  #[error("json error")]
  AnyError(#[from] anyhow::Error),
}

impl IntoResponse for Error {
  fn into_response(self) -> Result {
    let val = self.to_string();
    Response::with_status(500, val)
  }
}
