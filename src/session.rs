//! Cookie-based session management for the desirable web framework.

use crate::Result;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use hyper::header::HeaderMap;
use hyper::http;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;

type HmacSha256 = Hmac<Sha256>;
const DEFAULT_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;
const SESSION_ID_LENGTH: usize = 32;
const DEFAULT_COOKIE_NAME: &str = "desirable_session";

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
  #[error("invalid session cookie")]
  InvalidCookie,
  #[error("session signature mismatch")]
  SignatureMismatch,
  #[error("session expired")]
  Expired,
  #[error("session not found")]
  NotFound,
  #[error("key not found in session: {0}")]
  KeyNotFound(String),
  #[error("session serialization error: {0}")]
  Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
  pub cookie_name: String,
  pub path: String,
  pub domain: Option<String>,
  pub secure: bool,
  pub http_only: bool,
  pub same_site: cookie::SameSite,
  pub max_age_secs: Option<i64>,
  pub signing_key: Vec<u8>,
}

impl SessionConfig {
  pub fn new(signing_key: &[u8]) -> Self {
    assert!(
      signing_key.len() >= 32,
      "signing key must be at least 32 bytes"
    );
    Self {
      cookie_name: DEFAULT_COOKIE_NAME.to_string(),
      path: "/".to_string(),
      domain: None,
      secure: true,
      http_only: true,
      same_site: cookie::SameSite::Lax,
      max_age_secs: Some(DEFAULT_MAX_AGE_SECS),
      signing_key: signing_key.to_vec(),
    }
  }

  #[must_use]
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  #[must_use]
  pub fn path(mut self, path: &str) -> Self {
    self.path = path.to_string();
    self
  }

  #[must_use]
  pub fn domain(mut self, domain: &str) -> Self {
    self.domain = Some(domain.to_string());
    self
  }

  #[must_use]
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }

  #[must_use]
  pub fn http_only(mut self, http_only: bool) -> Self {
    self.http_only = http_only;
    self
  }

  #[must_use]
  pub fn same_site(mut self, same_site: cookie::SameSite) -> Self {
    self.same_site = same_site;
    self
  }

  #[must_use]
  pub fn max_age_secs(mut self, secs: i64) -> Self {
    self.max_age_secs = Some(secs);
    self
  }
}

impl Default for SessionConfig {
  fn default() -> Self {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    Self::new(&key)
  }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionData {
  pub id: String,
  pub created: DateTime<Utc>,
  pub accessed: DateTime<Utc>,
  #[serde(flatten)]
  pub data: HashMap<String, String>,
}

impl SessionData {
  pub fn new() -> Self {
    let now = Utc::now();
    let mut bytes = [0u8; SESSION_ID_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    Self {
      id,
      created: now,
      accessed: now,
      data: HashMap::new(),
    }
  }
}

impl Default for SessionData {
  fn default() -> Self {
    Self::new()
  }
}

#[derive(Clone, Debug)]
pub struct Session {
  inner: SessionData,
  modified: bool,
}

impl Session {
  pub fn new(data: SessionData) -> Self {
    Self {
      inner: data,
      modified: false,
    }
  }

  pub fn id(&self) -> &str {
    &self.inner.id
  }
  pub fn created(&self) -> DateTime<Utc> {
    self.inner.created
  }
  pub fn accessed(&self) -> DateTime<Utc> {
    self.inner.accessed
  }
  pub fn len(&self) -> usize {
    self.inner.data.len()
  }
  pub fn is_empty(&self) -> bool {
    self.inner.data.is_empty()
  }
  pub fn data(&self) -> &HashMap<String, String> {
    &self.inner.data
  }
  pub fn contains_key(&self, key: &str) -> bool {
    self.inner.data.contains_key(key)
  }
  pub fn is_modified(&self) -> bool {
    self.modified
  }

  pub fn get<T>(&self, key: &str) -> Result<Option<T>>
  where
    T: for<'de> serde::de::Deserialize<'de>,
  {
    if let Some(value) = self.inner.data.get(key) {
      Ok(Some(serde_json::from_str(value)?))
    } else {
      Ok(None)
    }
  }

  pub fn get_str(&self, key: &str) -> Option<&str> {
    self.inner.data.get(key).map(|s| s.as_str())
  }

  pub fn insert<T>(&mut self, key: &str, value: T) -> Result<()>
  where
    T: Serialize,
  {
    let json = serde_json::to_string(&value)?;
    self.inner.data.insert(key.to_string(), json);
    self.modified = true;
    Ok(())
  }

  pub fn remove<T>(&mut self, key: &str) -> Result<Option<T>>
  where
    T: for<'de> serde::de::Deserialize<'de>,
  {
    if let Some(value) = self.inner.data.remove(key) {
      self.modified = true;
      Ok(Some(serde_json::from_str(&value)?))
    } else {
      Ok(None)
    }
  }

  pub fn remove_str(&mut self, key: &str) -> Option<String> {
    let removed = self.inner.data.remove(key);
    if removed.is_some() {
      self.modified = true;
    }
    removed
  }

  pub fn clear(&mut self) {
    if !self.inner.data.is_empty() {
      self.inner.data.clear();
      self.modified = true;
    }
  }

  pub fn regenerate_id(&mut self) {
    let mut bytes = [0u8; SESSION_ID_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);
    self.inner.id = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    self.modified = true;
  }

  pub fn touch(&mut self) {
    self.inner.accessed = Utc::now();
  }
  pub fn into_data(self) -> SessionData {
    self.inner
  }
  pub fn data_mut(&mut self) -> &mut HashMap<String, String> {
    self.modified = true;
    &mut self.inner.data
  }
}

impl Default for Session {
  fn default() -> Self {
    Self::new(SessionData::new())
  }
}

#[derive(Clone, Debug)]
pub struct SessionManager {
  config: Arc<SessionConfig>,
}

impl SessionManager {
  pub fn new(config: SessionConfig) -> Self {
    Self {
      config: Arc::new(config),
    }
  }

  pub fn with_random_key() -> Self {
    Self::new(SessionConfig::default())
  }

  pub fn config(&self) -> &SessionConfig {
    &self.config
  }
  pub fn create_session(&self) -> Session {
    Session::new(SessionData::new())
  }

  pub fn read_session(&self, cookie_value: &str) -> Result<Option<Session>> {
    if cookie_value.is_empty() {
      return Ok(None);
    }
    let decoded = base64::engine::general_purpose::URL_SAFE
      .decode(cookie_value)
      .map_err(|_| SessionError::InvalidCookie)?;
    let pos = decoded.iter().position(|&c| c == b'|');
    if let Some(idx) = pos {
      let (data_bytes, signature_bytes) = decoded.split_at(idx);
      if signature_bytes.is_empty() || signature_bytes[0] != b'|' {
        return Err(SessionError::InvalidCookie.into());
      }
      let sig = &signature_bytes[1..];
      let mut mac = HmacSha256::new_from_slice(&self.config.signing_key)
        .map_err(|_| SessionError::InvalidCookie)?;
      mac.update(data_bytes);
      mac
        .verify_slice(sig)
        .map_err(|_| SessionError::SignatureMismatch)?;
      let session_data: SessionData =
        serde_json::from_slice(data_bytes).map_err(|_| SessionError::InvalidCookie)?;
      Ok(Some(Session::new(session_data)))
    } else {
      Err(SessionError::InvalidCookie.into())
    }
  }

  pub fn write_session(&self, session: &Session) -> String {
    let data_bytes = serde_json::to_vec(&session.inner).unwrap_or_default();
    let mut mac = HmacSha256::new_from_slice(&self.config.signing_key).unwrap();
    mac.update(&data_bytes);
    let signature = mac.finalize().into_bytes();
    let mut combined = data_bytes;
    combined.push(b'|');
    combined.extend_from_slice(&signature);
    base64::engine::general_purpose::URL_SAFE.encode(&combined)
  }

  pub fn make_cookie_header(&self, session: &Session) -> http::HeaderValue {
    let cookie_value = self.write_session(session);
    let mut builder = cookie::CookieBuilder::new(self.config.cookie_name.clone(), cookie_value)
      .path(self.config.path.clone())
      .http_only(self.config.http_only)
      .same_site(self.config.same_site);
    if let Some(max_age) = self.config.max_age_secs {
      builder = builder.max_age(time::Duration::seconds(max_age));
    }
    if self.config.secure {
      builder = builder.secure(true);
    }
    if let Some(ref domain) = self.config.domain {
      builder = builder.domain(domain.clone());
    }
    builder.build().to_string().parse().unwrap()
  }

  pub fn make_deletion_cookie(&self) -> http::HeaderValue {
    let mut builder = cookie::CookieBuilder::new(self.config.cookie_name.clone(), "")
      .path(self.config.path.clone())
      .http_only(self.config.http_only)
      .same_site(self.config.same_site)
      .max_age(time::Duration::seconds(0));
    if self.config.secure {
      builder = builder.secure(true);
    }
    if let Some(ref domain) = self.config.domain {
      builder = builder.domain(domain.clone());
    }
    builder.build().to_string().parse().unwrap()
  }

  pub fn get_cookie_value(&self, headers: &HeaderMap) -> Option<String> {
    headers
      .get(http::header::COOKIE)
      .and_then(|v| v.to_str().ok())
      .and_then(|cookie_str| {
        cookie_str
          .split(';')
          .map(|s| s.trim())
          .find(|s| s.starts_with(&format!("{}=", self.config.cookie_name)))
          .and_then(|s| s.split('=').nth(1).map(|s| s.to_string()))
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_session_new() {
    let session = Session::new(SessionData::new());
    assert!(!session.id().is_empty());
    assert!(session.is_empty());
    assert!(!session.is_modified());
  }

  #[test]
  fn test_session_insert_get() {
    let mut session = Session::new(SessionData::new());
    session.insert("user_id", 42).unwrap();
    session.insert("name", "Alice").unwrap();
    assert_eq!(session.len(), 2);
    assert!(session.is_modified());

    let user_id: Option<i32> = session.get("user_id").unwrap();
    assert_eq!(user_id, Some(42));

    let name: Option<String> = session.get("name").unwrap();
    assert_eq!(name, Some("Alice".to_string()));
  }

  #[test]
  fn test_session_remove() {
    let mut session = Session::new(SessionData::new());
    session.insert("key", "value").unwrap();
    let removed: Option<String> = session.remove("key").unwrap();
    assert_eq!(removed, Some("value".to_string()));
    assert!(session.is_empty());
  }

  #[test]
  fn test_session_manager_roundtrip() {
    let key = b"this-is-a-32-byte-secret-key-!!!";
    let manager = SessionManager::new(SessionConfig::new(key));

    let mut session = manager.create_session();
    session.insert("user_id", 123).unwrap();
    session.insert("name", "Bob").unwrap();

    let cookie_value = manager.write_session(&session);
    assert!(!cookie_value.is_empty());

    let loaded = manager.read_session(&cookie_value).unwrap().unwrap();
    assert_eq!(loaded.id(), session.id());
    let user_id: Option<i32> = loaded.get("user_id").unwrap();
    assert_eq!(user_id, Some(123));
  }

  #[test]
  fn test_session_cookie_header() {
    let key = b"this-is-a-32-byte-secret-key-!!!";
    let config = SessionConfig::new(key).secure(false).http_only(true);
    let manager = SessionManager::new(config);

    let session = manager.create_session();
    let header = manager.make_cookie_header(&session);
    assert!(!header.to_str().unwrap().is_empty());
    assert!(header.to_str().unwrap().contains("HttpOnly"));
  }
}
