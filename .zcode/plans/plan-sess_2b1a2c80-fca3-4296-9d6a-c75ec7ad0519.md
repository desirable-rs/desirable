## desirable v1.7.0 — 类型化 State、SIGTERM、尾斜杠容错、自定义错误处理

### 1. 类型化应用状态
- Router 新增 `pub state: Option<Arc<dyn Any + Send + Sync>>` 字段
- `Router::with_state<T>(T) -> Self` builder（注册路由前调用）
- dispatch 时把 `Arc<dyn Any + Send + Sync>` 注入 request extensions
- `Request::state<T>() -> Option<Arc<T>>`：从 extensions 取出并 downcast
- merge() 丢弃目标 state（父级 state 注入所有已合并路由），文档注明
- lib.rs re-export 无需新增类型

### 2. SIGTERM 优雅关闭
- shutdown_signal()：unix 下用 tokio::signal::unix::SignalKind::terminate() 与 ctrl_c 做 select；非 unix 保持 ctrl_c

### 3. 尾斜杠容错
- dispatch 精确匹配失败时：若 path 以 '/' 结尾且非根路径，用去掉尾斜杠的路径重试 recognize
- matching_methods 同样加尾斜杠重试，避免 /users/ 误报 405

### 4. 自定义错误处理器
- kernel.rs：Next::run 中间件分支改为 `cur.handle(req, self).await` 直接返回（Err 传播而非就地转响应）；与文档描述对齐
- Router 新增 `error_handler: Option<Arc<dyn Fn(Error) -> Response + Send + Sync>>` 私有字段 + `pub fn error_handler()` 注册方法
- dispatch：Next.run 结果为 Err 时 → error_handler(err)（或默认 err.into_response()）；HEAD 剥离 body 逻辑不受影响（仍作用于 Ok）

### 交付
- 文件：router.rs、kernel.rs、request.rs、server.rs、tests/nesting.rs、Cargo.toml(1.7.0)、CHANGELOG.md
- 测试：state 注入与 downcast（e2e）、error_handler JSON 渲染（e2e）、尾斜杠命中（e2e）、SIGTERM 无法在 CI 安全测试则覆盖单元级
- 全量验证 test/clippy/fmt/examples；零新依赖；无破坏性 API