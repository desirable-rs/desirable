use crate::{Middleware, Next, Request, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

/// Aggregate request metrics for one [`Metrics`] instance.
///
/// Cheap to read (one `Arc`, atomics only); render Prometheus text with
/// [`Metrics::render_prometheus`] or feed [`Metrics::snapshot`] into your
/// own exposition format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsSnapshot {
  /// Requests seen (counted on arrival).
  pub requests: u64,
  /// Responses with a `2xx` status.
  pub ok: u64,
  /// Responses with a `3xx` status.
  pub redirects: u64,
  /// Responses with a `4xx` status.
  pub client_errors: u64,
  /// Responses with a `5xx` status (and middleware errors, which render as 500).
  pub server_errors: u64,
  /// Requests currently in flight.
  pub in_flight: i64,
  /// Cumulative handler time in microseconds.
  pub total_duration_us: u64,
}

#[derive(Default)]
struct MetricsState {
  requests: AtomicU64,
  ok: AtomicU64,
  redirects: AtomicU64,
  client_errors: AtomicU64,
  server_errors: AtomicU64,
  in_flight: AtomicI64,
  duration_us: AtomicU64,
}

/// Zero-dependency request-counting middleware.
///
/// Global counters (atomics — no locks): total requests, responses per
/// status class (`2xx`/`3xx`/`4xx`/`5xx`), in-flight gauge, and cumulative
/// handler duration. Read them from any clone of the handle:
///
/// ```rust,ignore
/// let metrics = Metrics::new();
/// app.with(metrics.clone());
///
/// // Expose the counters:
/// app.get("/metrics", move |_| {
///     let metrics = metrics.clone();
///     async move { desirable::Response::builder().text(metrics.render_prometheus()) }
/// });
/// ```
///
/// This is intentionally *not* a full metrics ecosystem — no histograms,
/// labels, or registry. For per-route cardinalities or histograms, plug in
/// the `metrics`/`prometheus` crates from a middleware of your own.
#[derive(Clone, Default)]
pub struct Metrics {
  state: Arc<MetricsState>,
}

impl std::fmt::Debug for Metrics {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Metrics")
      .field("snapshot", &self.snapshot())
      .finish()
  }
}

impl Metrics {
  /// Creates a fresh counter set.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Point-in-time copy of all counters.
  #[must_use]
  pub fn snapshot(&self) -> MetricsSnapshot {
    let s = &self.state;
    MetricsSnapshot {
      requests: s.requests.load(Ordering::Relaxed),
      ok: s.ok.load(Ordering::Relaxed),
      redirects: s.redirects.load(Ordering::Relaxed),
      client_errors: s.client_errors.load(Ordering::Relaxed),
      server_errors: s.server_errors.load(Ordering::Relaxed),
      in_flight: s.in_flight.load(Ordering::Relaxed),
      total_duration_us: s.duration_us.load(Ordering::Relaxed),
    }
  }

  /// Renders the counters in Prometheus text exposition format
  /// (`desirable_*` metric family).
  #[must_use]
  pub fn render_prometheus(&self) -> String {
    let s = self.snapshot();
    format!(
      "# HELP desirable_requests_total Total requests received.\n\
       # TYPE desirable_requests_total counter\n\
       desirable_requests_total {}\n\
       # HELP desirable_responses_total Responses by status class.\n\
       # TYPE desirable_responses_total counter\n\
       desirable_responses_total{{class=\"2xx\"}} {}\n\
       desirable_responses_total{{class=\"3xx\"}} {}\n\
       desirable_responses_total{{class=\"4xx\"}} {}\n\
       desirable_responses_total{{class=\"5xx\"}} {}\n\
       # HELP desirable_in_flight Requests currently in flight.\n\
       # TYPE desirable_in_flight gauge\n\
       desirable_in_flight {}\n\
       # HELP desirable_request_duration_microseconds_total Cumulative handler time.\n\
       # TYPE desirable_request_duration_microseconds_total counter\n\
       desirable_request_duration_microseconds_total {}\n",
      s.requests,
      s.ok,
      s.redirects,
      s.client_errors,
      s.server_errors,
      s.in_flight,
      s.total_duration_us
    )
  }

  fn classify(&self, status: hyper::StatusCode) {
    let s = &self.state;
    match status.as_u16() {
      200..=299 => s.ok.fetch_add(1, Ordering::Relaxed),
      300..=399 => s.redirects.fetch_add(1, Ordering::Relaxed),
      400..=499 => s.client_errors.fetch_add(1, Ordering::Relaxed),
      _ => s.server_errors.fetch_add(1, Ordering::Relaxed),
    };
  }
}

#[async_trait::async_trait]
impl Middleware for Metrics {
  async fn handle(&self, req: Request, next: Next<'_>) -> Result {
    let state = &self.state;
    state.requests.fetch_add(1, Ordering::Relaxed);
    state.in_flight.fetch_add(1, Ordering::Relaxed);
    let start = Instant::now();

    let result = next.run(req).await;
    let duration_us = start.elapsed().as_micros() as u64;

    state.in_flight.fetch_sub(1, Ordering::Relaxed);
    state.duration_us.fetch_add(duration_us, Ordering::Relaxed);
    match &result {
      Ok(res) => self.classify(res.status()),
      // Middleware errors render as 500 downstream; count them as such.
      Err(_) => {
        state.server_errors.fetch_add(1, Ordering::Relaxed);
      }
    }
    result
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fresh_metrics_are_zeroed() {
    let m = Metrics::new();
    let s = m.snapshot();
    assert_eq!(s.requests, 0);
    assert_eq!(s.in_flight, 0);
    assert_eq!(s.ok, 0);
  }

  #[test]
  fn test_prometheus_format_shape() {
    let m = Metrics::new();
    let text = m.render_prometheus();
    assert!(text.contains("# TYPE desirable_requests_total counter"));
    assert!(text.contains("desirable_requests_total 0"));
    assert!(text.contains("desirable_in_flight 0"));
    assert!(text.contains("desirable_responses_total{class=\"2xx\"} 0"));
  }
}
