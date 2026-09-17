## desirable v1.6.0 — 错误语义、静态文件 404、BodyLimit、RateLimit

### 1. 错误语义修复
- `src/error.rs`：新增 `Error::status()` 方法——MissingParam/InvalidParam/Urlencoded/Json → 400，其余 → 500；新增 `Error::BodyTooLarge` 变体 → 413
- `src/into_response.rs` 的 `impl IntoResponse for Error`：按 status() 返回；4xx 用错误消息作 body，5xx 用通用 "internal server error" 并 `tracing::error!` 记录真实错误（不泄漏内部细节）
- `src/error.rs` 的 `From<Error> for Response` 同步修复

### 2. 静态文件 404
- `src/fs.rs` 的 `serve_file_with_cache`：`tokio::fs::metadata`/`read` 的 NotFound 错误返回 404 响应；其他 IO 错误维持 500

### 3. BodyLimit 中间件（新增 src/middleware/body_limit.rs）
- `BodyLimit::new(max_bytes)`：Content-Length > 限制 → 立即 413
- 同时将限制插入 request extensions；`Request::body()/body_json()/form()` 读取限制并用 `http_body_util::Limited` 包裹 collect，超限 → `Error::BodyTooLarge`（413）——覆盖 chunked 场景
- middleware/mod.rs 与 lib.rs 补导出

### 4. RateLimit 中间件（新增 src/middleware/rate_limit.rs）
- `RateLimit::per_second(n)` / `RateLimit::new(capacity, refill_per_sec)`：按 `req.remote_addr` 的 IP 分桶的内存令牌桶（std Mutex<HashMap<IpAddr, Bucket>>，Instant 惰性补充）
- 超限 → 429 + `Retry-After: 1`
- 桶数量上限（默认 65536）防内存膨胀，超限时清空全部桶（简单防洪）
- 补导出

### 5. 小修
- `src/server.rs`：连接 future 完成结果不再用 `_` 丢弃——错误记 debug/warn 日志

### 交付
- 文件：error.rs、into_response.rs、fs.rs、request.rs、middleware/{body_limit,rate_limit,mod}.rs、lib.rs、server.rs、Cargo.toml(1.6.0)、CHANGELOG.md
- 单元测试：status() 映射、is_not_modified 无关、令牌桶行为、413/429 e2e（tests/nesting.rs 增补）
- 全量验证 test/clippy/fmt/examples；零新依赖；无破坏性 API