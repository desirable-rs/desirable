//! End-to-end tests for the observability round: the `Metrics` middleware
//! counters and the enriched request span fields.

use desirable::{Metrics, Response, Router, test::TestServer};

#[tokio::test]
async fn metrics_counters_track_status_classes_and_in_flight() {
  let metrics = Metrics::new();
  let mut app = Router::new();
  app.with(metrics.clone());

  app.get("/ok", |_| async { "ok" });
  let missing_metrics = metrics.clone();
  app.get("/metrics", move |_| {
    let metrics = missing_metrics.clone();
    async move { Response::builder().text(metrics.render_prometheus()) }
  });

  let server = TestServer::spawn(app).await;

  // 3 × 2xx, 1 × 404 (router fallback), then render.
  for _ in 0..3 {
    let res = server.get("/ok").await;
    assert_eq!(res.status(), hyper::StatusCode::OK);
  }
  let res = server.get("/nope").await;
  assert_eq!(res.status(), hyper::StatusCode::NOT_FOUND);

  let text = server.get("/metrics").await.text();
  // At render time the /metrics request itself is already counted (on
  // arrival) but not yet classified: 3 + 1 + 1 = 5, in flight = 1.
  assert!(text.contains("desirable_requests_total 5"), "got:\n{text}");
  assert!(text.contains("desirable_in_flight 1"), "got:\n{text}");
  assert!(
    text.contains("desirable_responses_total{class=\"2xx\"} 3"),
    "got:\n{text}"
  );
  assert!(
    text.contains("desirable_responses_total{class=\"4xx\"} 1"),
    "got:\n{text}"
  );
  assert!(
    text.contains("desirable_responses_total{class=\"5xx\"} 0"),
    "got:\n{text}"
  );
  // Duration accumulated for the handled requests.
  assert!(
    !text.contains("desirable_request_duration_microseconds_total 0"),
    "duration should have accumulated, got:\n{text}"
  );

  // After traffic stops: in flight drops back to zero.
  let snapshot = metrics.snapshot();
  assert_eq!(snapshot.in_flight, 0);
  assert_eq!(snapshot.ok, 4, "3 × /ok + the classified /metrics render");
  assert_eq!(snapshot.client_errors, 1);

  server.shutdown().await.unwrap();
}
