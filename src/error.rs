use crate::Response;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum Error {
  #[error("hyper error {0:?}")]
  HyperError(#[from] hyper::Error),
  #[error("http error {0:?}")]
  HTTPError(#[from] hyper::http::Error),
  #[error("json error {0:?}")]
  JsonError(#[from] serde_json::Error),
  #[error("IO error {0:?}")]
  IOError(#[from] std::io::Error),
  #[error("any error {0:?}")]
  AnyError(#[from] anyhow::Error),
  #[error("query error {0:?}")]
  UrlencodedError(#[from] serde_urlencoded::de::Error),
  #[error("addr parse error {0:?}")]
  AddrParseError(#[from] std::net::AddrParseError),
  #[error("InvalidStatusCode {0:?}")]
  InvalidStatusCode(#[from] hyper::http::status::InvalidStatusCode),
  #[error("missing url param {name:?}")]
  MissingParam { name: String },
  #[error("invalid param {name:?} as {expected:?}, {err:?}")]
  InvalidParam {
    name: String,
    expected: &'static str,
    err: String,
  },
  #[error("error msg {msg:?}")]
  Message { msg: String },
}

pub fn missing_param(name: &str) -> Error {
  Error::MissingParam {
    name: name.to_string(),
  }
}

pub fn error_msg(msg: &str) -> Error {
  Error::Message {
    msg: msg.to_string(),
  }
}

pub fn invalid_param(
  name: impl ToString,
  expected: &'static str,
  err: impl std::error::Error,
) -> Error {
  Error::InvalidParam {
    name: name.to_string(),
    expected,
    err: err.to_string(),
  }
}

impl From<Error> for Response {
  fn from(err: Error) -> Self {
    Response::with_status(500, err.to_string()).unwrap()
  }
}
