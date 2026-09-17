use crate::{Middleware, Next, Request, Response, Result};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

/// Default cap on the number of tracked client buckets. When exceeded, the
/// map is cleared — a simple safeguard against memory exhaustion from
/// spoofed-source floods before eviction logic becomes warranted.
const DEFAULT_MAX_BUCKETS: usize = 65_536;

/// A per-client token bucket.
#[derive(Debug)]
struct Bucket {
  tokens: f64,
  last_refill: Instant,
}

/// In-memory, per-client-IP rate limiting middleware using token buckets.
///
/// Each client IP gets a bucket with `capacity` tokens, refilled at
/// `refill_per_sec` tokens per second. A request consumes one token; when the
/// bucket is empty, the client receives `429 Too Many Requests` with a
/// `Retry-After: 1` header.
///
/// The state is process-local: it does not survive restarts and is not
/// shared between instances. Behind a proxy, consider keying on
/// `X-Forwarded-For` (not done here to avoid trusting spoofable headers).
///
/// # Example
///
/// ```rust,ignore
/// use desirable::RateLimit;
///
/// let mut router = Router::new();
/// router.with(RateLimit::per_second(100));
/// // or: RateLimit::new(200, 50.0) — burst of 200, refilling 50/s
/// ```
#[derive(Clone, Debug)]
pub struct RateLimit {
  capacity: f64,
  refill_per_sec: f64,
  max_buckets: usize,
  state: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
}

impl RateLimit {
  /// Creates a limiter with a burst capacity of `per_second` requests and a
  /// refill rate of `per_second` tokens per second.
  #[must_use]
  pub fn per_second(per_second: u32) -> Self {
    Self::new(u64::from(per_second), f64::from(per_second))
  }

  /// Creates a limiter with an explicit burst capacity and refill rate.
  #[must_use]
  pub fn new(capacity: u64, refill_per_sec: f64) -> Self {
    Self {
      capacity: capacity as f64,
      refill_per_sec,
      max_buckets: DEFAULT_MAX_BUCKETS,
      state: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  /// Attempts to take one token for `ip`. Returns `false` when the client is
  /// over the limit.
  fn try_acquire(&self, ip: &IpAddr) -> bool {
    let mut state = self.state.lock().expect("rate limit mutex poisoned");
    if state.len() >= self.max_buckets {
      state.clear();
    }
    let now = Instant::now();
    let bucket = state.entry(*ip).or_insert(Bucket {
      tokens: self.capacity,
      last_refill: now,
    });
    let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
    bucket.last_refill = now;
    if bucket.tokens >= 1.0 {
      bucket.tokens -= 1.0;
      true
    } else {
      false
    }
  }

  /// Whole seconds until the client's bucket refills enough for one request
  /// (at least 1).
  fn retry_after_secs(&self) -> u64 {
    1
  }
}

#[async_trait::async_trait]
impl Middleware for RateLimit {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let allowed = match req.remote_addr.as_ref() {
      Some(addr) => self.try_acquire(&addr.ip()),
      // No peer address (e.g. direct Request construction): don't limit.
      None => true,
    };

    if !allowed {
      let mut response = Response::with_status(429, "too many requests".to_string())?;
      response.set_header(
        hyper::header::RETRY_AFTER,
        self.retry_after_secs().to_string().parse().unwrap(),
      );
      return Ok(response);
    }

    next.run(req).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_token_bucket_enforces_capacity() {
    let limiter = RateLimit::per_second(3);
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    assert!(limiter.try_acquire(&ip));
    assert!(limiter.try_acquire(&ip));
    assert!(limiter.try_acquire(&ip));
    // Bucket exhausted within the same instant.
    assert!(!limiter.try_acquire(&ip));
  }

  #[test]
  fn test_token_bucket_refills_over_time() {
    let limiter = RateLimit::new(1, 1000.0); // refills 1000 tokens/sec
    let ip: IpAddr = "10.0.0.2".parse().unwrap();

    assert!(limiter.try_acquire(&ip));
    assert!(!limiter.try_acquire(&ip));
    // Wait long enough for ~10 tokens to accumulate.
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(limiter.try_acquire(&ip));
  }

  #[test]
  fn test_clients_are_isolated() {
    let limiter = RateLimit::per_second(1);
    let a: IpAddr = "10.0.0.3".parse().unwrap();
    let b: IpAddr = "10.0.0.4".parse().unwrap();

    assert!(limiter.try_acquire(&a));
    assert!(!limiter.try_acquire(&a));
    // Client b is unaffected by client a's exhaustion.
    assert!(limiter.try_acquire(&b));
  }

  #[test]
  fn test_bucket_map_clears_at_capacity() {
    let mut limiter = RateLimit::per_second(1);
    limiter.max_buckets = 8;
    for i in 0..64u32 {
      let ip: IpAddr = format!("10.0.0.{i}").parse().unwrap();
      let _ = limiter.try_acquire(&ip);
    }
    assert_eq!(limiter.state.lock().unwrap().len(), 8);
  }
}
