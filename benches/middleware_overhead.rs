//! Measures the per-layer cost of `#[async_trait]`'s boxed futures in the
//! exact middleware-chain shape used by `Next::run` (dyn `Middleware` +
//! dyn `Endpoint`, recursion through borrowed slices).
//!
//! This is the evidence base for the v3.0 decision on whether the public
//! `Middleware`/`Endpoint` traits are worth redesigning (generic, axum-style
//! layering) to eliminate the per-layer `Pin<Box<dyn Future>>` allocation.
//!
//! Run with: cargo bench --bench middleware_overhead

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// The plain baseline: no trait object, no boxing.
async fn plain_handler(n: u64) -> u64 {
  n + 1
}

#[async_trait::async_trait]
trait Endpoint: Send + Sync + 'static {
  async fn call(&self, n: u64) -> u64;
}

struct PlainEndpoint;

#[async_trait::async_trait]
impl Endpoint for PlainEndpoint {
  async fn call(&self, n: u64) -> u64 {
    n + 1
  }
}

#[async_trait::async_trait]
trait Middleware: Send + Sync + 'static {
  async fn handle(&self, n: u64, next: Next<'_>) -> u64;
}

/// Mirrors `desirable::Next`: borrowed middleware slice + endpoint,
/// recursed in `run`.
struct Next<'a> {
  endpoint: &'a dyn Endpoint,
  middlewares: &'a [Box<dyn Middleware>],
}

impl Next<'_> {
  async fn run(mut self, n: u64) -> u64 {
    if let Some((cur, rest)) = self.middlewares.split_first() {
      self.middlewares = rest;
      cur.handle(n, self).await
    } else {
      self.endpoint.call(n).await
    }
  }
}

struct Noop;

/// The chain must have an observable side effect per layer, otherwise LLVM
/// elides the boxed futures entirely and the numbers are fiction.
static LAYER_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[async_trait::async_trait]
impl Middleware for Noop {
  async fn handle(&self, n: u64, next: Next<'_>) -> u64 {
    use std::sync::atomic::Ordering::Relaxed;
    LAYER_CALLS.fetch_add(1, Relaxed);
    next.run(n).await
  }
}

/// Owns the endpoint for the duration of the chain, exactly like a request
/// owns its route's endpoint through `Next::run`. `#[inline(never)]` keeps
/// the compiled chain opaque to the caller so the per-layer boxes are
/// actually materialized inside.
#[inline(never)]
async fn run_chain(middlewares: &[Box<dyn Middleware>], n: u64) -> u64 {
  let endpoint = PlainEndpoint;
  let out = Next {
    endpoint: &endpoint,
    middlewares,
  }
  .run(n)
  .await;
  std::hint::black_box(out)
}

fn middleware_boxing_benchmark(c: &mut Criterion) {
  let no_mw: Vec<Box<dyn Middleware>> = Vec::new();
  let one_mw: Vec<Box<dyn Middleware>> = vec![Box::new(Noop)];
  let three_mw: Vec<Box<dyn Middleware>> = vec![Box::new(Noop), Box::new(Noop), Box::new(Noop)];
  let eight_mw: Vec<Box<dyn Middleware>> = (0..8)
    .map(|_| Box::new(Noop) as Box<dyn Middleware>)
    .collect();

  let rt = tokio::runtime::Runtime::new().unwrap();
  let mut group = c.benchmark_group("middleware_chain");

  // NOTE: every future is awaited (`.to_async`), and the chain carries a
  // side effect per layer, so the boxed futures are really materialized.
  group.bench_function("plain_async_baseline", |b| {
    b.to_async(&rt)
      .iter(|| async { std::hint::black_box(plain_handler(black_box(41)).await) })
  });

  group.bench_function("dyn_endpoint_call_boxed", |b| {
    b.to_async(&rt).iter(|| async {
      let endpoint: Box<dyn Endpoint> = Box::new(PlainEndpoint);
      std::hint::black_box(endpoint.call(black_box(41)).await)
    })
  });

  group.bench_function("chain_0_middleware", |b| {
    b.to_async(&rt)
      .iter(|| async { run_chain(&no_mw, black_box(41)).await })
  });

  group.bench_function("chain_1_middleware", |b| {
    b.to_async(&rt)
      .iter(|| async { run_chain(&one_mw, black_box(41)).await })
  });

  group.bench_function("chain_3_middleware", |b| {
    b.to_async(&rt)
      .iter(|| async { run_chain(&three_mw, black_box(41)).await })
  });

  group.bench_function("chain_8_middleware", |b| {
    b.to_async(&rt)
      .iter(|| async { run_chain(&eight_mw, black_box(41)).await })
  });

  group.finish();
}

criterion_group!(benches, middleware_boxing_benchmark);
criterion_main!(benches);
