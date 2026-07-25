## DX Enhancement Plan for desirable v1.1.1

### Issue 1: Relax Response::json() bounds + return Response directly
- Remove `Sized + Send + Sync + 'static` bounds, keep only `Serialize`
- Change return type from `Result<Self>` to `Response`
- Panic internally on serialization failure (`.expect()`)
- This also addresses Issue 6

### Issue 2: Response::builder() pattern
- Add `ResponseBuilder` struct with chainable methods: `.status()`, `.header()`, `.json()`, `.body()`, `.text()`
- Add `Response::builder()` static method
- `builder().json()` is infallible (consistent with updated `Response::json()`)
- `builder().body()` returns `Result<Response>` (hyper builder can fail)

### Issue 3: req.query_or_default<T>()
- Add `query_or_default::<T: DeserializeOwned + Default>()` method
- Returns `T::default()` when no query string exists, avoiding `Option` boilerplate

### Issue 4: req.body_json<T>() alias
- Add `body_json::<T>()` as a clear alias for `body::<T>()`
- Same signature, just delegates

### Issue 5: req.param_str() convenience
- Add `param_str(name) -> Result<String>` method
- Avoids needing `::<String>` turbofish for the most common param extraction case

### Issue 6: Response::json() returns Response (not Result)
- Covered by Issue 1 — unified fix

### Bonus: IntoResponse for serde_json::Value
- Add `impl IntoResponse for serde_json::Value`
- Enables `serde_json::json!(...)` as handler return type (doc already claims this works)

### Files Changed
- `src/response.rs`: +80 lines (ResponseBuilder, json() signature change)
- `src/request.rs`: +35 lines (new methods)
- `src/into_response.rs`: +10 lines (serde_json::Value impl)
- `Cargo.toml`: version 1.1.0 → 1.1.1

### Total: ~125 lines new code. No new dependencies. All existing tests must pass.