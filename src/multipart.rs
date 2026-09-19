//! Streaming `multipart/form-data` parsing (feature `multipart`).
//!
//! Enabled by the `multipart` cargo feature (the [`multer`](multer) crate).
//!
//! [`Request::multipart`](crate::Request::multipart) returns a [`Multipart`]
//! iterator over the request's fields; file fields expose
//! [`MultipartField::chunk`] for incremental consumption, so uploads stream
//! to disk (or anywhere else) without ever being buffered in memory. The
//! body limit — server default or [`BodyLimit`](crate::BodyLimit) — applies
//! while parsing, aborting oversized uploads with 413.

use std::error::Error as _;

/// Streaming parser over a `multipart/form-data` request body.
///
/// Created by [`Request::multipart`](crate::Request::multipart); consume it
/// by repeatedly calling [`Multipart::next_field`] until it returns `None`.
pub struct Multipart<'a> {
  pub(super) inner: multer::Multipart<'a>,
}

impl<'a> Multipart<'a> {
  /// Yields the next field, or `None` when the body is exhausted.
  ///
  /// # Errors
  ///
  /// Returns [`crate::Error::Multipart`] for malformed multipart data, and
  /// [`crate::Error::BodyTooLarge`] (413) when the body limit is exceeded
  /// mid-parse.
  pub async fn next_field(&mut self) -> crate::Result<Option<MultipartField<'a>>> {
    match self.inner.next_field().await {
      Ok(Some(field)) => Ok(Some(field)),
      Ok(None) => Ok(None),
      Err(err) => {
        // multer wraps upstream stream errors; recover the framework's own
        // body-limit error so oversized uploads still surface as 413.
        let is_too_large = err
          .source()
          .and_then(|src| src.downcast_ref::<crate::Error>())
          .is_some_and(|src| matches!(src, crate::Error::BodyTooLarge));
        if is_too_large {
          Err(crate::Error::BodyTooLarge)
        } else {
          Err(crate::Error::Multipart(err))
        }
      }
    }
  }
}

/// One field of a multipart body: a form value or an uploaded file.
///
/// Re-exported from [`multer`]; the `chunk`/`bytes` methods consume the
/// field's data. Check `name()` / `file_name()` to tell form fields from
/// file uploads.
pub type MultipartField<'r> = multer::Field<'r>;
