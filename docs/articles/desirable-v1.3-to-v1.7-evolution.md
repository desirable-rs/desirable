# desirable v1.7：一个轻量 Rust Web 框架的五步进化

> 从"能跑的 demo"到"敢上生产"——desirable 用五个版本走完了这段路，核心依赖一个没多加。

---

## 背景

[v1.2.0 的 DX 打磨](desirable-v1.2-upgrade.md)解决了"写起来不顺手"的问题之后，我们把注意力转向了更本质的问题：**当你把一个框架部署到真实服务器上，会发生什么？**

答案是：路径穿越漏洞、错误的 HTTP 状态码、无限制的请求体、被 SIGTERM 直接掐断的连接。v1.3.0 到 v1.7.0，五个版本，逐一解决。

---

## v1.3：安全与正确性

这一版修了三个"上线前必须知道"的问题：

**1. `ServeDir` 路径穿越**。`/assets/*file` 路由的 `:file` 参数直接字符串拼接路径，`../../etc/passwd` 可以读走服务器任意文件。修复采用组件级校验——解析后的路径必须仍在目录内，否则 403。

**2. 默认 404 返回 HTTP 200**。未匹配路由的响应体写着 "handle not found"，状态码却是 200。所有监控和网关都会被骗过。

**3. 方法不匹配返回 404 而非 405**。`GET /users` 存在时 `POST /users` 现在正确返回 `405 Method Not Allowed`，并带上 `Allow: GET` 头——这是 REST 客户端自动发现接口的依据。

同时补齐了文档里承诺但代码里从未实现的 Content-Type 检测（.css/.js/.png 等 22 类），以及 `form()` 表单解析、`header()`/`cookie()` 访问器。

## v1.4：路由系统成熟

三个能力一次到位：

- **路由嵌套**：`Router::new().prefix("/api").with(Auth)` 然后 `app.merge(api)`。子路由器的中间件随路由走，不再被 merge 丢弃
- **HEAD 回退 GET**：爬虫和健康检查的 `HEAD /health` 不再撞上 405
- **目录索引**：`ServeDir` 命中目录时自动返回 `index.html`

过程中还发现并修复了一个**潜伏已久的 bug**：`merge()` 用 `HashMap::extend` 会整体替换同名（GET/POST…）的路由表——合并子路由器后，父路由器同方法的所有路由直接消失。集成测试抓到了它。

这一版还引入了**端到端集成测试**：起真实服务器、裸 TCP 发 HTTP 请求，锁定中间件作用域、HEAD 空 body、405 Allow 等行为。

## v1.5：生产级关闭与缓存

**连接排空**。此前的"优雅关闭"只停了 accept 循环，在途请求直接被掐断。现在：SIGINT/SIGTERM 触发后，每个连接通过 hyper 的 `graceful_shutdown()` 完成当前请求，`TaskTracker` 等待全部排空（默认 10 秒超时可配）才退出。

**静态文件条件请求**。响应自动携带弱 ETag（`W/"{mtime:x}-{size:x}"`，tower-http 同款策略）和 `Last-Modified`；浏览器二次请求命中 `If-None-Match` 即返回 304 空 body。`ServeFile` 和 `ServeDir` 共享同一实现，顺带消除了重复代码。

## v1.6：HTTP 语义与防护

两个修复、两个新中间件：

**错误语义**。此前所有错误一律 500，且响应体泄漏内部细节（如 `IO error Os { code: 2, ... }`）。现在：参数/查询/JSON 错误 → 400，body 超限 → 413；5xx 返回通用的 "internal server error"，真实错误走 `tracing` 日志。

**静态文件 404**。文件不存在不再返回带 IO 错误详情的 500。

**`BodyLimit` 中间件**。`app.with(BodyLimit::new(1024 * 1024))`——声明超限立即拒绝（不读 body），chunked 请求体在读取时用 `http_body_util::Limited` 强制截断。

**`RateLimit` 中间件**。按 IP 的内存令牌桶，`app.with(RateLimit::per_second(100))`，超限 429 + `Retry-After`。桶数量有上限，防止伪造源 IP 撑爆内存。

## v1.7：DX 补全

最后一块拼图：

- **类型化状态**：`Router::new().with_state(db)` + handler 里 `req.state::<DbPool>()`。类型检查的 downcast，注入 extensions，无泛型参数污染
- **SIGTERM 优雅关闭**：Kubernetes 滚动更新发的是 SIGTERM——现在能正确排空
- **尾斜杠容错**：`GET /users/` 自动回退匹配 `/users`
- **`set_error_handler()`**：进程级错误渲染器，一行配出统一 JSON 错误信封

## 数字

| 指标 | v1.2.0 | v1.7.1 |
|------|--------|--------|
| 单元 + 集成测试 | 48 | 84（含 10 个端到端） |
| 修复的安全/语义问题 | — | 11 |
| 新增外部依赖 | 0 | 0（httpdate、tokio-util 原已是传递依赖） |
| crates.io 版本 | 1.2.0 | 1.7.1 |

## 升级

```toml
[dependencies]
desirable = "1.7"
```

从 1.2 一路升到 1.7 只需改版本号——唯一需要留意的行为变化：

1. **`Router::merge()` 不再丢路由**（v1.4 修复，`routes` 字段类型调整）
2. **`with()` 改为 axum 式顺序语义**（v1.4：中间件只作用于其后注册的路由）
3. **5xx 响应体不再包含错误详情**（v1.6：如需自定义格式，用 `set_error_handler`）

完整变更见 [CHANGELOG](https://github.com/desirable-rs/desirable/blob/main/CHANGELOG.md)。

---

## 写在最后

这五个版本没有引入任何"大特性"——没有 WebSocket，没有 TLS，没有宏。每一行新代码都在回答同一个问题：**这个框架敢不敢直接放到互联网上？**

现在敢了。

GitHub：[desirable-rs/desirable](https://github.com/desirable-rs/desirable) · 文档：[docs.rs/desirable](https://docs.rs/desirable)
