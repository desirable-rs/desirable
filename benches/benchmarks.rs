//! Benchmark tests for the desirable web framework.
//!
//! Run benchmarks with: cargo bench
//!
//! These benchmarks measure core operations that don't require
//! Request creation (which has hyper body type constraints).

use criterion::{Criterion, criterion_group, criterion_main};
use desirable::{Response, Router};
use route_recognizer::{Params, Router as RouteRecognizer};
use std::collections::HashMap;

// ===== Router Creation Benchmarks =====

fn router_creation_benchmark(c: &mut Criterion) {
  c.bench_function("router_new", |b| b.iter(Router::new));

  c.bench_function("router_with_single_route", |b| {
    b.iter(|| {
      let mut router = Router::new();
      router.get("/", |_| async { "home" });
      router
    })
  });

  c.bench_function("router_with_ten_routes", |b| {
    b.iter(|| {
      let mut router = Router::new();
      for i in 0..10 {
        router.get(&format!("/{}", i), |_| async { "test" });
      }
      router
    })
  });

  c.bench_function("router_with_hundred_routes", |b| {
    b.iter(|| {
      let mut router = Router::new();
      for i in 0..100 {
        router.get(&format!("/{}", i), |_| async { "test" });
      }
      router
    })
  });
}

// ===== Response Creation Benchmarks =====

fn response_creation_benchmark(c: &mut Criterion) {
  c.bench_function("response_body_string", |b| {
    b.iter(|| Response::body("Hello, World!"))
  });

  c.bench_function("response_with_status_200", |b| {
    b.iter(|| Response::with_status(200, "OK".to_string()))
  });

  c.bench_function("response_with_status_404", |b| {
    b.iter(|| Response::with_status(404, "Not Found".to_string()))
  });

  c.bench_function("response_redirect_302", |b| {
    b.iter(|| Response::redirect(302, "/new-location"))
  });

  c.bench_function("response_json_small", |b| {
    #[derive(serde::Serialize)]
    struct SmallJson {
      status: String,
    }
    b.iter(|| {
      Response::json(SmallJson {
        status: "ok".to_string(),
      })
    })
  });

  c.bench_function("response_json_large", |b| {
    #[derive(serde::Serialize)]
    struct LargeJson {
      id: i32,
      name: String,
      email: String,
      items: Vec<Item>,
    }
    #[derive(serde::Serialize)]
    struct Item {
      name: String,
      price: f64,
    }
    b.iter(|| {
      Response::json(LargeJson {
        id: 1,
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
        items: vec![
          Item {
            name: "Item 1".to_string(),
            price: 9.99,
          },
          Item {
            name: "Item 2".to_string(),
            price: 19.99,
          },
        ],
      })
    })
  });
}

// ===== IntoResponse Conversion Benchmarks =====

fn into_response_benchmark(c: &mut Criterion) {
  use desirable::IntoResponse;

  c.bench_function("into_response_static_str", |b| {
    b.iter(|| "Hello, World!".into_response())
  });

  c.bench_function("into_response_string", |b| {
    let s = String::from("Hello, World!");
    b.iter(|| s.clone().into_response())
  });

  c.bench_function("into_response_unit", |b| b.iter(|| ().into_response()));

  c.bench_function("into_response_tuple_status_str", |b| {
    b.iter(|| (200, "OK").into_response())
  });

  c.bench_function("into_response_tuple_u16_str", |b| {
    b.iter(|| (404, "Not Found").into_response())
  });

  c.bench_function("into_response_result_ok", |b| {
    b.iter(|| Ok::<&str, &str>("ok").into_response())
  });

  c.bench_function("into_response_result_err", |b| {
    b.iter(|| Err::<(), &str>("error").into_response())
  });
}

// ===== Route Recognition Benchmarks =====

fn route_recognition_benchmark(c: &mut Criterion) {
  c.bench_function("route_recognizer_new", |b| {
    b.iter(RouteRecognizer::<()>::new)
  });

  c.bench_function("route_recognizer_add_single", |b| {
    b.iter(|| {
      let mut router = RouteRecognizer::<()>::new();
      router.add("/", ());
      router
    })
  });

  c.bench_function("route_recognizer_add_ten", |b| {
    b.iter(|| {
      let mut router = RouteRecognizer::<()>::new();
      for i in 0..10 {
        router.add(&format!("/{}", i), ());
      }
      router
    })
  });

  c.bench_function("route_recognizer_recognize_exact", |b| {
    let mut router = RouteRecognizer::<()>::new();
    for i in 0..100 {
      router.add(&format!("/user/{}", i), ());
    }
    b.iter(|| {
      let _match = router.recognize("/user/42");
    })
  });

  c.bench_function("route_recognizer_recognize_param", |b| {
    let mut router = RouteRecognizer::<()>::new();
    router.add("/users/:id", ());
    router.add("/posts/:id/comments/:comment_id", ());
    b.iter(|| {
      let _match1 = router.recognize("/users/123");
      let _match2 = router.recognize("/posts/456/comments/789");
    })
  });

  c.bench_function("route_recognizer_recognize_not_found", |b| {
    let mut router = RouteRecognizer::<()>::new();
    router.add("/", ());
    router.add("/users/:id", ());
    b.iter(|| {
      let _result = router.recognize("/not-found");
    })
  });
}

// ===== Params Access Benchmarks =====

fn params_access_benchmark(c: &mut Criterion) {
  c.bench_function("params_new", |b| b.iter(Params::new));

  c.bench_function("params_find_empty", |b| {
    let params = Params::new();
    b.iter(|| params.find("id"))
  });

  c.bench_function("hashmap_insert", |b| {
    b.iter(|| {
      let mut map = HashMap::new();
      map.insert("id".to_string(), "42".to_string());
      map.insert("name".to_string(), "test".to_string());
      map.insert("action".to_string(), "delete".to_string());
      map
    })
  });

  c.bench_function("hashmap_find_hit", |b| {
    let mut map = HashMap::new();
    map.insert("id".to_string(), "42".to_string());
    map.insert("name".to_string(), "test".to_string());
    map.insert("action".to_string(), "delete".to_string());
    b.iter(|| {
      map.get("id");
      map.get("name");
      map.get("action");
    })
  });

  c.bench_function("hashmap_find_miss", |b| {
    let mut map = HashMap::new();
    map.insert("id".to_string(), "42".to_string());
    b.iter(|| {
      map.get("missing");
      map.get("another");
    })
  });
}

// ===== Header Value Creation Benchmarks =====

fn header_value_benchmark(c: &mut Criterion) {
  use hyper::header;

  c.bench_function("header_content_type_text", |b| {
    b.iter(|| header::HeaderValue::from_static("text/plain; charset=utf-8"))
  });

  c.bench_function("header_content_type_json", |b| {
    b.iter(|| header::HeaderValue::from_static("application/json"))
  });

  c.bench_function("header_clone", |b| {
    let value = header::HeaderValue::from_static("text/plain; charset=utf-8");
    b.iter(|| value.clone())
  });
}

criterion_group!(
  benches,
  router_creation_benchmark,
  response_creation_benchmark,
  into_response_benchmark,
  route_recognition_benchmark,
  params_access_benchmark,
  header_value_benchmark
);
criterion_main!(benches);
