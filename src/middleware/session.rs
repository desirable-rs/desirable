use crate::session::{Session, SessionManager};
use crate::{Middleware, Next, Request, Result};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use tracing::debug;

/// A cloneable handle to the request's session, shared between the
/// [`SessionLayer`] middleware and handlers.
///
/// `SessionHandle` dereferences to [`Mutex<Session>`], so all [`Session`]
/// methods are available after `lock()`:
///
/// ```rust,ignore
/// req.session().lock().unwrap().insert("user_id", 42)?;
/// let uid: Option<i32> = req.session().lock().unwrap().get("user_id")?;
/// ```
#[derive(Clone)]
pub struct SessionHandle(Arc<Mutex<Session>>);

impl SessionHandle {
  /// Wraps a session in a new handle.
  pub(crate) fn new(session: Session) -> Self {
    Self(Arc::new(Mutex::new(session)))
  }

  /// Returns `true` when the session has pending changes that the
  /// [`SessionLayer`] will persist via `Set-Cookie`.
  pub fn is_modified(&self) -> bool {
    self.0.lock().expect("session mutex poisoned").is_modified()
  }
}

impl Deref for SessionHandle {
  type Target = Mutex<Session>;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

/// Middleware that loads the request's session from its signed cookie and
/// persists it back automatically.
///
/// The loaded session is exposed to handlers through
/// [`Request::session`](crate::Request::session). When the response returns,
/// a `Set-Cookie` header is appended **only if the session was modified** —
/// anonymous requests that never touch the session produce no cookie.
///
/// A missing or empty cookie starts a fresh session. A tampered cookie
/// (signature mismatch) is logged and replaced with a fresh session rather
/// than failing the request.
///
/// # Example
///
/// ```rust,ignore
/// use desirable::{SessionConfig, SessionLayer, SessionManager};
///
/// let manager = SessionManager::new(SessionConfig::new(b"your-32-byte-secret-key!!!!"));
/// let mut app = Router::new();
/// app.with(SessionLayer::new(manager));
/// app.post("/login", |req: Request| async move {
///   req.session().lock().insert("user_id", 42)?;
///   Ok(desirable::Response::text("logged in"))
/// });
/// ```
#[derive(Clone, Debug)]
pub struct SessionLayer {
  manager: SessionManager,
}

impl SessionLayer {
  /// Creates a session middleware backed by the given manager.
  #[must_use]
  pub fn new(manager: SessionManager) -> Self {
    Self { manager }
  }
}

#[async_trait::async_trait]
impl Middleware for SessionLayer {
  async fn handle(&self, mut req: Request, next: Next<'_>) -> Result {
    // Load (or start) the session for this request; the cookie lookup is
    // borrowing, so no per-request allocation happens here.
    let session = {
      let cookie = self.manager.get_cookie_value_str(req.inner.headers());
      match cookie.map(|c| self.manager.read_session(c)) {
        Some(Ok(Some(session))) => session,
        Some(Ok(None)) => self.manager.create_session(),
        Some(Err(err)) => {
          debug!(%err, "session cookie rejected, starting a fresh session");
          self.manager.create_session()
        }
        None => self.manager.create_session(),
      }
    };

    // Share the session with handlers; keep our own clone to persist after.
    let handle = SessionHandle::new(session);
    let persist = Arc::clone(&handle.0);
    req.extensions_mut().insert(handle);

    let mut response = next.run(req).await;

    // Persist the session only when handlers actually changed it; a
    // destroyed session gets a deletion cookie instead.
    if let Ok(res) = &mut response {
      let session = persist.lock().expect("session mutex poisoned");
      if session.is_destroyed() {
        res.append_header(
          hyper::header::SET_COOKIE,
          self.manager.make_deletion_cookie(),
        );
      } else if session.is_modified() {
        res.append_header(
          hyper::header::SET_COOKIE,
          self.manager.make_cookie_header(&session),
        );
      }
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_session_handle_shares_state() {
    let handle = SessionHandle::new(Session::new(crate::session::SessionData::new()));
    let clone = handle.clone();
    handle.lock().unwrap().insert("k", 1).unwrap();
    let v: Option<i32> = clone.lock().unwrap().get("k").unwrap();
    assert_eq!(v, Some(1));
    assert!(clone.is_modified());
  }
}
