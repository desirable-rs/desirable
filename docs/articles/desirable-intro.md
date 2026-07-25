# desirable：一个 260 行的 Rust Web 框架，够用就好

> 如果你已经厌倦了 axum 的泛型迷宫和 actix-web 的 Actor 模型，也许你需要一个更"简单"的选择。

---

## 一句话介绍

**desirable** 是一个基于 hyper + tokio 构建的轻量级 Rust Web 框架，只做 Web 框架该做的事，不做多余的抽象。

```
Cargo.toml 依赖数：23 个（含间接依赖）
核心代码行数：  ~1200 行
框架体量：      ~260KB（release）
```

---

## 为什么又要造一个轮子？

Rust Web 框架领域已经非常成熟——axum、actix-web、rocket、warp、tide 等等。但每个框架都有自己的"哲学重量"：

| 框架 | 你需要接受的 |
|------|-------------|
| **axum** | 深入理解 tower 生态、`impl IntoResponse` 的泛型地狱、State 提取器 |
| **actix-web** | Actor 模型的心智负担、庞大的依赖树 |
| **rocket** | 宏魔法、nightly 依赖（早期）、编译慢 |
| **warp** | filter 组合子的独特范式，学习曲线陡峭 |

这些框架都很优秀，但当你只是想写一个简单的 API 服务，或者快速搭建一个微服务原型时，它们带来了太多"附带成本"。

**desirable 的设计哲学是：用你已经知道的概念工作。**

如果你写过 Express（Node.js）或 Gin（Go），你会觉得 desirable 很熟悉：

```rust
use desirable::{Router, Logger, Cors, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = Router::new();

    // 中间件：一行一个
    app.with(Logger);           // 访问日志
    app.with(Cors::new());      // 跨域支持

    // 路由：直觉式定义
    app.get("/", || async { "Hello, World!" });
    app.get("/users/:id", |req| async move {
        let id: i32 = req.param("id")?;
        format!("User {}", id)
    });
    app.post("/users", |mut req| async move {
        let body: serde_json::Value = req.body().await?;
        Response::json(&body)
    });

    // 启动，Ctrl+C 优雅退出
    desirable::new("127.0.0.1:3000").run(app).await
}
```

没有宏。没有泛型约束声明。没有 `#[derive]` 标记在 handler 上。就是纯函数。

---

## 核心特性一览

### 1. 路由：直观、零意外

支持所有 HTTP 方法和路径参数：

```rust
app.get("/users/:id", handler);
app.post("/users", handler);
app.put("/users/:id", handler);
app.delete("/users/:id", handler);
app.patch("/users/:id", handler);
app.options("/users", handler);
```

路径参数自动提取，类型安全：

```rust
async fn get_user(req: Request) -> Result {
    let id: i32 = req.param("id")?;     // /users/42 → 42
    let name: String = req.param("name")?; // 自动解析
    // ...
}
```

Query 参数解析同样简单：

```rust
#[derive(Deserialize)]
struct Pagination { page: u32, limit: u32 }

async fn list(req: Request) -> Result {
    if let Some(p) = req.query::<Pagination>()? {
        // p.page, p.limit
    }
    // ...
}
```

### 2. 中间件：就是普通的 trait

不需要理解 tower 的 Service/Layer 抽象。实现一个 trait 即可：

```rust
use desirable::{Middleware, Next, Request, Result};

struct Auth;

#[async_trait::async_trait]
impl Middleware for Auth {
    async fn handle(&self, req: Request, next: Next<'_>) -> Result {
        // 前置逻辑：检查 token
        if req.headers().get("Authorization").is_none() {
            return Response::with_status(401, "Unauthorized".into());
        }
        // 调用下游
        next.run(req).await
    }
}

app.with(Auth);
```

v1.1 内置了两个中间件：

- **Logger** — 自动记录 `GET /users/42 → 200 3ms`
- **Cors** — builder 模式，一行配置 `Cors::new().allow_origin("https://example.com")`

### 3. 响应：你想要的方式都支持

```rust
// 字符串
async fn hello() -> &'static str { "Hello" }

// JSON
async fn json() -> Result { Response::json(&my_data) }

// 状态码
async fn not_found() -> Result { Response::with_status(404, "Not Found".into()) }

// 重定向
async fn redirect() -> Result { Response::redirect(302, "/new") }

// Tuple
async fn tuple() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "gone")
}

// 无内容
async fn empty() {}
```

所有返回类型都实现了 `IntoResponse`，框架自动处理。

### 4. Session：签名 Cookie，开箱即用

```rust
let key = b"your-32-byte-secret-key-here!!!!";
let config = SessionConfig::new(key);
let manager = SessionManager::new(config);

// 创建 Session
let mut session = manager.create_session();
session.insert("user_id", 42)?;
session.insert("role", "admin")?;

// 生成 Cookie
let cookie_header = manager.make_cookie_header(&session);

// 从请求读取 Session
let cookie_value = manager.get_cookie_value(req.headers());
if let Some(session) = manager.read_session(&cookie_value)? {
    let user_id: Option<i32> = session.get("user_id")?;
}
```

基于 HMAC-SHA256 签名，防篡改，Base64URL 编码安全传输。

### 5. 静态文件

```rust
// 单个文件
app.get("/favicon.ico", ServeFile::new("static/favicon.ico".into()));

// 整个目录
app.get("/assets/*file", ServeDir::new("static/assets".into()));
```

### 6. 优雅关闭

v1.1 新增，`Ctrl+C` 不再是暴力 kill：

```rust
// run() 自带优雅关闭
server.run(app).await?;

// 或显式调用
server.run_graceful(app).await?;
```

---

## 和 axum 的对比：一个具体例子

假设我们要写一个简单的用户 CRUD API，带日志和 CORS：

**axum 版本：**

```rust
use axum::{
    Router, routing::{get, post},
    middleware,
    extract::{Path, State},
    Json,
};
use tower_http::cors::CorsLayer;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState::new());

    let app = Router::new()
        .route("/users", get(list_users).post(create_user))
        .route("/users/:id", get(get_user))
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn(logging))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

**desirable 版本：**

```rust
use desirable::{Router, Logger, Cors, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = Router::new();
    app.with(Logger);
    app.with(Cors::new());
    app.get("/users", list_users);
    app.post("/users", create_user);
    app.get("/users/:id", get_user);

    desirable::new("0.0.0.0:3000").run(app).await
}
```

不需要 `State`、不需要 `Path` extractor、不需要理解 `layer` 和 `route` 的区别。**函数签名就是接口**。

---

## 什么时候用 desirable？

| 场景 | 推荐 |
|------|------|
| 微服务 / API 网关 | ✅ 非常适合 |
| 快速原型 / MVP | ✅ 最佳选择 |
| 静态文件服务 | ✅ 可以胜任 |
| 学习 Rust Web 开发 | ✅ 概念少，易上手 |
| 需要 WebSocket | ❌ 暂不支持 |
| 需要 gRPC | ❌ 不在 scope 内 |
| 需要精细的性能调优 | ⚠️ 可以，但 axum 生态更丰富 |

---

## 核心指标

```
依赖数：    23 crates（含间接依赖 96 个）
核心代码：  ~1200 行
Release 体积：~260KB
编译时间：  ~8s（M1 Pro，from scratch）
v1.1 新增：  ~260 行（Logger + CORS + 优雅关闭）
```

---

## 安装使用

```toml
[dependencies]
desirable = "1.1"
tokio = { version = "1", features = ["full"] }
```

```rust
use desirable::{Router, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = Router::new();
    app.get("/", || async { "Hello, desirable!" });
    desirable::new("127.0.0.1:3000").run(app).await
}
```

详细文档和更多示例，请访问 [GitHub](https://github.com/desirable-rs/desirable)。

---

## 最后

desirable 不试图成为下一个 axum 或 actix-web。它的目标是成为那个**当你只需要 80% 功能时，不给你剩下 20% 的复杂度**的框架。

如果你正在寻找一个轻量、直觉、零宏、零魔法字符串的 Rust Web 框架，试试 desirable。

**够用，就好。**
