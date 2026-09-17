## desirable v1.4.0 — 路由嵌套、HEAD 回退、目录索引、Cookie API、超时

### 1. Router::prefix() + merge() 中间件修复
- 新增 `Router::prefix(&str) -> Self` builder 方法（设置已有 prefix 字段）
- 重写 `merge()`：将目标路由器的中间件链包装进其每个 endpoint（新建内部 `WrappedEndpoint`，`call` 内通过 `Next` 执行链），不再丢弃中间件
- 用法：`app.merge(Router::new().prefix("/api").with(Auth))` 实现嵌套

### 2. HEAD 请求自动回退 GET
- dispatch 中：`HEAD` 未命中 HEAD 路由时改查 GET 路由表
- 命中 GET 则执行并将响应 body 置空（Full<Bytes> 换为空）
- 405 判定相应调整：HEAD 存在 GET 路由即视为允许

### 3. ServeDir 目录索引
- `:file` 解析后若是目录（或为空串），追加尝试 `index.html`
- 用 `tokio::fs::metadata` 判断目录

### 4. Response Cookie API 与头语义修复
- `Response::headers_mut() -> &HeaderMap` 公共访问器
- `append_header(name, value)`（append 语义，不覆盖）
- `set_cookie(Cookie)`：append 到 SET-COOKIE，支持多个
- `remove_cookie(name)`：附加 max-age=0 的删除 cookie
- 保留现有 `set_header`（insert 语义）不变，不破坏兼容

### 5. Timeout 中间件（新增 src/middleware/timeout.rs）
- `Timeout::new(Duration)`，`tokio::time::timeout` 包装 `next.run()`
- 超时返回 `408 Request Timeout`
- mod.rs 与 lib.rs 补 re-export

### 交付
- 文件：router.rs、fs.rs、response.rs、middleware/{mod,timeout}.rs、lib.rs、Cargo.toml(1.4.0)、CHANGELOG.md
- 每项带单元测试（HEAD 回退、merge 中间件、index.html、cookie、timeout 触发）
- 全量验证：test + clippy -D warnings + fmt + examples
- 零新依赖，~300 行，无破坏性 API 变更