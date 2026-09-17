## desirable v1.5.0 — 连接排空、Server API、静态文件缓存

### 1. 真正的优雅关闭（连接排空）
- accept_loop 中每个连接任务持有 `watch::Receiver<bool>`，任务内 `tokio::select!`：
  连接完成 vs 关闭信号。信号触发时 `conn.as_mut().graceful_shutdown()` 后继续 poll 排空
- 用 `tokio_util::task::TaskTracker` 跟踪所有连接任务；shutdown 后 `close()` + `wait().await`（带默认 10s 排空超时，用 tokio::time::timeout 包裹）
- `run_graceful` = `run_with_shutdown(router, ctrl_c)`

### 2. Server API 补全
- `Server::try_bind(&str) -> Result<Server>`（不 panic）；`bind()` 保持现有 panic 语义
- `run_with_shutdown(router, impl Future<Output=()> + Send + 'static)` 新方法
- accept 后 `stream.set_nodelay(true)`（失败仅记录不影响服务）

### 3. 静态文件缓存
- fs.rs 提取共享异步助手 `serve_file_with_cache(path, req)`：
  读取 metadata（mtime+size）→ 生成弱 ETag `W/"{mtime_secs:x}-{size:x}"` 与 Last-Modified（httpdate 格式化）
  → If-None-Match 命中或 If-Modified-Since 不早于 mtime → 返回 304（带 ETag/Last-Modified，无 body）
  → 否则 200 全量响应
- ServeFile 与 ServeDir 的 call() 统一走该助手（消除重复）
- `httpdate = "^1"` 加入直接依赖（已在 Cargo.lock，零新增）

### 4. 测试
- tests/nesting.rs：spawn_server 返回 (addr, JoinHandle)，等待就绪改为轮询 connect 而非固定 sleep
- 新 e2e 测试：静态文件 304（先 200 拿 ETag，再带 If-None-Match 请求断言 304）
- 新 e2e 测试：run_with_shutdown 触发后 server 任务正常返回 Ok

### 交付
- 文件：src/server.rs、src/fs.rs、src/lib.rs（如需 re-export）、Cargo.toml(1.5.0)、tests/nesting.rs、CHANGELOG.md
- 全量验证 test/clippy/fmt/examples；锁文件零新增 crate；无破坏性 API