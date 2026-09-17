## desirable v1.8.0 — SessionLayer 自动会话中间件

### 1. 新增 src/middleware/session.rs
- `SessionLayer { manager: SessionManager }`，`SessionLayer::new(SessionManager)`
- handle 流程：读 Cookie → read_session（Ok(Some) 复用 / Ok(None) 或 Err（篡改，记 debug 日志）→ create_session）→ Arc<Mutex<Session>> 本地克隆 + SessionHandle 注入 extensions → next.run → Ok 响应且 is_modified() 时 set_cookie(manager.make_cookie_header(&session))
- `SessionHandle(Arc<Mutex<Session>>)`：`#[derive(Clone)]`，`impl Deref for SessionHandle { type Target = Mutex<Session> }`；未安装层时 Request::session() 返回游离空会话（debug 日志提示）
- `Request::session(&self) -> SessionHandle` 新方法

### 2. 导出
- middleware/mod.rs：pub mod session + pub use SessionLayer
- lib.rs：re-export SessionLayer

### 3. 测试
- 单元：SessionHandle 修改可见性（Arc 内共享）
- e2e（tests/nesting.rs，裸 HTTP）：
  a) POST 登录路由 insert → 响应头含 Set-Cookie
  b) 带 Set-Cookie 值 GET → 读回正确值
  c) 篡改 Cookie → 会话为空且正常 200（不 panic）
  d) 无修改访问 → 响应无 Set-Cookie 头

### 4. 文档与版本
- README：Handlers 一节补充「用 ? 请用具名 async fn」指南；Sessions 一节更新为 SessionLayer 用法
- Cargo.toml 1.8.0、CHANGELOG
- 全量验证 test/clippy/fmt/examples；零新依赖