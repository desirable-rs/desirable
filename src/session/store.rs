//! Pluggable server-side session storage.
//!
//! By default sessions live entirely inside the signed cookie (~4 KB, no
//! server-side state, no revocation). Installing a [`SessionStore`] switches
//! the manager to *server-side sessions*: the cookie then carries only the
//! signed session ID and the data lives in the store, which buys
//!
//! - **Revocation** — `Session::destroy()` deletes the stored entry, so a
//!   stolen cookie dies with the session instead of surviving to `max_age`;
//! - **Capacity** — session data is no longer bounded by cookie size limits;
//! - **Restart invalidation** — cookies reference store entries, so nothing
//!   survives a process restart with the in-memory store.
//!
//! The trait is object-safe (`Arc<dyn SessionStore>`), so deployments can
//! swap in a Redis/database implementation without touching the framework.

use super::data::SessionData;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Server-side session storage.
///
/// Implement this trait to persist sessions anywhere (memory, Redis, a
/// database). The manager calls `save` only when a session was modified and
/// `remove` on `Session::destroy()`; `load` must return `None` for unknown,
/// expired, or revoked IDs — the request then simply starts a fresh session.
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync + 'static {
  /// Loads the data stored under `id`, if present.
  async fn load(&self, id: &str) -> Option<SessionData>;

  /// Persists `data` under `data.id`.
  async fn save(&self, data: &SessionData);

  /// Deletes the session with the given ID (revocation).
  async fn remove(&self, id: &str);
}

/// Default cap on tracked sessions; beyond it, idle sessions are evicted
/// first, then the least-recently-touched one.
const DEFAULT_MAX_SESSIONS: usize = 65_536;

/// A session untouched for longer than this is evicted when the store is
/// at capacity.
const IDLE_EVICTION: Duration = Duration::from_secs(60 * 60);

/// In-memory [`SessionStore`] — zero dependencies, built in.
///
/// Entries are keyed by session ID and tracked with a last-touched
/// timestamp (updated on load and save). When the store hits capacity,
/// sessions idle for more than an hour are evicted; if none qualify, the
/// single least-recently-touched session is dropped, keeping memory bounded.
///
/// Data is process-local: it does not survive restarts and is not shared
/// between instances.
///
/// # Example
///
/// ```rust
/// use desirable::{MemorySessionStore, SessionConfig, SessionManager};
///
/// let manager = SessionManager::new(SessionConfig::new(b"your-32-byte-secret-key-here!!!!"))
///     .with_store(MemorySessionStore::new());
/// ```
#[derive(Default)]
pub struct MemorySessionStore {
  entries: Mutex<HashMap<String, (SessionData, Instant)>>,
  max_capacity: usize,
}

impl MemorySessionStore {
  /// Creates a store holding up to 65,536 sessions.
  pub fn new() -> Self {
    Self::with_capacity(DEFAULT_MAX_SESSIONS)
  }

  /// Creates a store with an explicit session-count cap.
  pub fn with_capacity(max_capacity: usize) -> Self {
    Self {
      entries: Mutex::new(HashMap::new()),
      max_capacity,
    }
  }

  /// Drops sessions idle beyond the eviction window; when none qualify and
  /// the store is still full, drops the single least-recently-touched one.
  fn evict_if_full(&self, map: &mut HashMap<String, (SessionData, Instant)>) {
    if map.len() < self.max_capacity {
      return;
    }
    let now = Instant::now();
    let stale: Vec<String> = map
      .iter()
      .filter(|(_, (_, touched))| now.duration_since(*touched) > IDLE_EVICTION)
      .map(|(id, _)| id.clone())
      .collect();
    for id in stale {
      map.remove(&id);
    }
    if map.len() >= self.max_capacity
      && let Some(oldest) = map
        .iter()
        .min_by_key(|(_, (_, touched))| *touched)
        .map(|(id, _)| id.clone())
    {
      map.remove(&oldest);
    }
  }
}

#[async_trait::async_trait]
impl SessionStore for MemorySessionStore {
  async fn load(&self, id: &str) -> Option<SessionData> {
    let mut map = self.entries.lock().expect("session store mutex poisoned");
    let (data, touched) = map.get_mut(id)?;
    *touched = Instant::now();
    Some(data.clone())
  }

  async fn save(&self, data: &SessionData) {
    let mut map = self.entries.lock().expect("session store mutex poisoned");
    self.evict_if_full(&mut map);
    map.insert(data.id.clone(), (data.clone(), Instant::now()));
  }

  async fn remove(&self, id: &str) {
    self
      .entries
      .lock()
      .expect("session store mutex poisoned")
      .remove(id);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn test_save_load_remove_roundtrip() {
    let store = MemorySessionStore::new();
    let mut data = SessionData::new();
    data
      .data
      .insert("user".to_string(), "\"alice\"".to_string());

    store.save(&data).await;
    let loaded = store.load(&data.id).await.unwrap();
    assert_eq!(
      loaded.data.get("user").map(String::as_str),
      Some("\"alice\"")
    );

    store.remove(&data.id).await;
    assert!(store.load(&data.id).await.is_none());
  }

  #[tokio::test]
  async fn test_load_unknown_id_is_none() {
    let store = MemorySessionStore::new();
    assert!(store.load("does-not-exist").await.is_none());
  }

  #[tokio::test]
  async fn test_capacity_eviction_keeps_recent_sessions() {
    let store = MemorySessionStore::with_capacity(8);
    let mut ids = Vec::new();
    for i in 0..64u32 {
      let mut data = SessionData::new();
      data.data.insert("n".to_string(), i.to_string());
      store.save(&data).await;
      ids.push(data.id);
    }
    // Hard cap enforced; the most recently saved session survives.
    assert!(store.entries.lock().unwrap().len() <= 8);
    assert!(store.load(&ids[63]).await.is_some());
  }
}
