use crate::{Error, Response, Result};
use bytes::{Bytes, BytesMut};
use http_body_util::Full;
use hyper::header;
use std::borrow::Cow;
pub trait IntoResponse {
  fn into_response(self) -> Result;
}

impl IntoResponse for Full<Bytes> {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder().body(self)?.into();
    Ok(response)
  }
}

impl IntoResponse for &'static str {
  fn into_response(self) -> Result {
    Cow::Borrowed(self).into_response()
  }
}

impl IntoResponse for String {
  fn into_response(self) -> Result {
    Cow::<'static, str>::Owned(self).into_response()
  }
}

impl IntoResponse for Cow<'static, str> {
  fn into_response(self) -> Result {
    let mut res = Full::from(self).into_response()?;
    res.inner.headers_mut().insert(
      header::CONTENT_TYPE,
      header::HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
    );
    Ok(res)
  }
}

impl<T, E> IntoResponse for std::result::Result<T, E>
where
  T: IntoResponse,
  E: IntoResponse,
{
  fn into_response(self) -> Result {
    match self {
      Ok(response) => response.into_response(),
      Err(err) => err.into_response(),
    }
  }
}

impl IntoResponse for Error {
  fn into_response(self) -> Result {
    let val = self.to_string();
    Response::with_status(500, val)
  }
}

impl IntoResponse for () {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .body(Full::new(Bytes::default()))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for Response {
  fn into_response(self) -> Result {
    Ok(self)
  }
}

impl IntoResponse for (hyper::StatusCode, String) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(self.0)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (hyper::StatusCode, &'static str) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(self.0)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (u16, String) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(hyper::StatusCode::from_u16(self.0)?)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for (u16, &'static str) {
  fn into_response(self) -> Result {
    let response = hyper::http::Response::builder()
      .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.to_string())
      .status(hyper::StatusCode::from_u16(self.0)?)
      .body(Full::new(Bytes::from(self.1)))?
      .into();
    Ok(response)
  }
}

impl IntoResponse for Bytes {
  fn into_response(self) -> Result {
    let mut res = Full::from(self).into_response()?;
    res.inner.headers_mut().insert(
      header::CONTENT_TYPE,
      header::HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
    );
    Ok(res)
  }
}

impl IntoResponse for BytesMut {
  fn into_response(self) -> Result {
    self.freeze().into_response()
  }
}
