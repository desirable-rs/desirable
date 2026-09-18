//! The response body type.
//!
//! [`Body`] is a concrete, cheaply-constructible body that is either fully
//! buffered ([`Body::full`]) or streamed frame by frame
//! ([`Body::stream`]/[`Body::channel`]). Handlers and middleware always deal
//! with this single type — no body generics leak into the public API.

use bytes::Bytes;
use futures_core::Stream;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full, StreamBody};
use std::pin::Pin;
use std::task::{Context, Poll};

/// The boxed error type carried by streaming bodies.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Minimal poll-based stream adapter (futures-core only ships the trait).
struct PollStream<F>(F);

impl<F, T> Stream for PollStream<F>
where
  F: FnMut(&mut Context<'_>) -> Poll<Option<T>> + Unpin,
{
  type Item = T;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
    (self.get_mut().0)(cx)
  }
}

/// The response body: fully buffered or streamed.
///
/// # Example
///
/// ```rust,ignore
/// // Buffered:
/// let body = Body::full("Hello, World!");
///
/// // Streamed from a channel (e.g. server-sent events):
/// let (mut sender, body) = Body::channel(16);
/// tokio::spawn(async move {
///   for i in 0..10 {
///     sender.send(format!("data: {i}\n\n").into()).await.unwrap();
///   }
/// });
/// ```
pub enum Body {
  /// A fully buffered body.
  Full(Bytes),
  /// A body streamed frame by frame.
  Streaming(BoxBody<Bytes, BoxError>),
}

/// The sending half of a [`Body::channel`] pair.
///
/// Dropping the sender ends the body gracefully.
pub struct BodySender {
  tx: tokio::sync::mpsc::Sender<Result<Bytes, BoxError>>,
}

impl BodySender {
  /// Enqueues one chunk into the body.
  ///
  /// Returns `Err` when the receiving body has been dropped.
  pub async fn send(
    &self,
    chunk: impl Into<Bytes>,
  ) -> Result<(), tokio::sync::mpsc::error::SendError<Result<Bytes, BoxError>>> {
    self.tx.send(Ok(chunk.into())).await
  }
}

impl Default for Body {
  fn default() -> Self {
    Body::empty()
  }
}

impl std::fmt::Debug for Body {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Body::Full(data) => f
        .debug_struct("Body::Full")
        .field("len", &data.len())
        .finish(),
      Body::Streaming(_) => f.debug_struct("Body::Streaming").finish_non_exhaustive(),
    }
  }
}

impl Body {
  /// Creates an empty body.
  #[must_use]
  pub fn empty() -> Self {
    Body::Full(Bytes::new())
  }

  /// Creates a fully buffered body.
  pub fn full(data: impl Into<Bytes>) -> Self {
    Body::Full(data.into())
  }

  /// Creates a streaming body from any stream of chunks.
  ///
  /// Chunks are `Result`-typed so the stream can fail mid-body; the error is
  /// surfaced by hyper as a truncated response.
  ///
  /// # Example
  ///
  /// ```rust,ignore
  /// let body = Body::stream(tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024));
  /// ```
  pub fn stream<S, T, E>(stream: S) -> Self
  where
    S: Stream<Item = Result<T, E>> + Send + Sync + 'static,
    T: Into<Bytes>,
    E: Into<BoxError>,
  {
    // Box once so the adapter owns a pinned stream — no unsafe projection.
    let mut stream = Box::pin(stream);
    let frames = PollStream(
      move |cx: &mut Context<'_>| match stream.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(http_body::Frame::data(chunk.into())))),
        Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err.into()))),
        Poll::Ready(None) => Poll::Ready(None),
        Poll::Pending => Poll::Pending,
      },
    );
    Body::Streaming(StreamBody::new(frames).boxed())
  }

  /// Creates a channel-backed streaming body, plus its sender.
  ///
  /// The body ends when the sender is dropped.
  pub fn channel(capacity: usize) -> (BodySender, Body) {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, BoxError>>(capacity);
    let body = Body::Streaming(BoxBody::new(ChannelBody { rx }));
    (BodySender { tx }, body)
  }
}

/// Streams chunks from a channel receiver until it closes.
struct ChannelBody {
  rx: tokio::sync::mpsc::Receiver<Result<Bytes, BoxError>>,
}

impl http_body::Body for ChannelBody {
  type Data = Bytes;
  type Error = BoxError;

  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
    let this = self.get_mut();
    match std::pin::pin!(this.rx.recv()).poll(cx) {
      Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(http_body::Frame::data(chunk)))),
      Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
      Poll::Ready(None) => Poll::Ready(None),
      Poll::Pending => Poll::Pending,
    }
  }
}

impl http_body::Body for Body {
  type Data = Bytes;
  type Error = BoxError;

  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
    // Both variants are Unpin, so `get_mut` is all the projection needed.
    let this = self.get_mut();
    match this {
      Body::Full(data) => {
        match Pin::new(&mut Full::new(std::mem::take(data))).poll_frame(cx) {
          // `Full<Bytes>` can never error; silence the Infallible arm.
          Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
          Poll::Ready(Some(Err(err))) => match err {},
          Poll::Ready(None) => Poll::Ready(None),
          Poll::Pending => Poll::Pending,
        }
      }
      Body::Streaming(inner) => Pin::new(inner).poll_frame(cx),
    }
  }

  fn size_hint(&self) -> http_body::SizeHint {
    match self {
      Body::Full(data) => http_body::SizeHint::with_exact(data.len() as u64),
      Body::Streaming(inner) => inner.size_hint(),
    }
  }

  fn is_end_stream(&self) -> bool {
    match self {
      Body::Full(data) => data.is_empty(),
      Body::Streaming(inner) => inner.is_end_stream(),
    }
  }
}

impl From<Bytes> for Body {
  fn from(data: Bytes) -> Self {
    Body::full(data)
  }
}

impl From<&'static str> for Body {
  fn from(data: &'static str) -> Self {
    Body::full(data)
  }
}

impl From<String> for Body {
  fn from(data: String) -> Self {
    Body::full(data)
  }
}

impl From<std::borrow::Cow<'static, str>> for Body {
  fn from(data: std::borrow::Cow<'static, str>) -> Self {
    match data {
      std::borrow::Cow::Borrowed(s) => Body::full(s),
      std::borrow::Cow::Owned(s) => Body::full(s),
    }
  }
}

impl From<Vec<u8>> for Body {
  fn from(data: Vec<u8>) -> Self {
    Body::full(data)
  }
}

impl From<Full<Bytes>> for Body {
  fn from(full: Full<Bytes>) -> Self {
    Body::Full(full.into_inner().unwrap_or_default())
  }
}

impl From<BoxBody<Bytes, BoxError>> for Body {
  fn from(streaming: BoxBody<Bytes, BoxError>) -> Self {
    Body::Streaming(streaming)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use http_body::Body as _;
  use http_body_util::BodyExt;

  /// A stream from a Vec, for tests (futures-core has no `iter`).
  struct IterStream(std::vec::IntoIter<Result<Bytes, BoxError>>);

  impl Stream for IterStream {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
      Poll::Ready(self.get_mut().0.next())
    }
  }

  #[tokio::test]
  async fn test_full_body_roundtrip() {
    let body = Body::full("hello");
    assert_eq!(body.size_hint().exact(), Some(5));
    let collected = BodyExt::collect(body).await.unwrap().to_bytes();
    assert_eq!(&collected[..], b"hello");
  }

  #[tokio::test]
  async fn test_empty_body_ends_immediately() {
    let body = Body::empty();
    assert!(body.is_end_stream());
    let collected = BodyExt::collect(body).await.unwrap().to_bytes();
    assert!(collected.is_empty());
  }

  #[tokio::test]
  async fn test_stream_body_delivers_chunks() {
    let chunks: Vec<Result<Bytes, BoxError>> = vec![
      Ok(Bytes::from("aa")),
      Ok(Bytes::from("bb")),
      Ok(Bytes::from("cc")),
    ];
    let body = Body::stream(IterStream(chunks.into_iter()));
    let collected = BodyExt::collect(body).await.unwrap().to_bytes();
    assert_eq!(&collected[..], b"aabbcc");
  }

  #[tokio::test]
  async fn test_channel_body_receives_until_sender_dropped() {
    let (sender, body) = Body::channel(4);
    tokio::spawn(async move {
      sender.send("one").await.unwrap();
      sender.send("two").await.unwrap();
      // sender dropped here -> body ends
    });
    let collected = BodyExt::collect(body).await.unwrap().to_bytes();
    assert_eq!(&collected[..], b"onetwo");
  }
}
