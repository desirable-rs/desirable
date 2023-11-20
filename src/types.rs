use crate::Error;
use crate::Response;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;

pub type AnyResult<T> = anyhow::Result<T, anyhow::Error>;
pub type Result<T = Response> = std::result::Result<T, Error>;

pub type HyperResponse = hyper::Response<Full<Bytes>>;
pub type HyperRequest = hyper::Request<Incoming>;