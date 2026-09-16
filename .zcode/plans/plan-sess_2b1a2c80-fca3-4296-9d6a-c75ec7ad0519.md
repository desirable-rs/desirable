## desirable v1.3.0 — Bug 修复 + 功能完善

### P0 修复（Bug/安全）

1. **ServeDir 路径穿越防护** (`src/fs.rs`)
   - 用 `Path::components()` 校验解析后的路径仍位于 `dir` 之内
   - 越界请求返回 403 Forbidden

2. **默认 404 状态码修复** (`src/router.rs`)
   - `default_handler` 改为返回 `(StatusCode::NOT_FOUND, "handle not found")`

3. **405 Method Not Allowed** (`src/router.rs`)
   - 路径在任意方法的路由表中命中、但当前方法没有对应路由时：
     返回 405 + `Allow: GET, POST` 头（列出该路径支持的所有方法）

### P1 功能

4. **静态文件 Content-Type** (`src/fs.rs`)
   - 按 `mime_guess` 风格用已有 `mime` crate 做扩展名映射（内建小型映射表即可，避免引入 mime_guess）
   - 常见类型：html/css/js/json/png/jpg/svg/ico/txt/xml/pdf/wasm

5. **`req.form::<T>()`** (`src/request.rs`)
   - 校验 Content-Type 为 `application/x-www-form-urlencoded`
   - 收集 body 后用 `serde_urlencoded::from_bytes` 解析

### P2 人体工学

6. **`req.header(name) -> Option<&HeaderValue>`** (`src/request.rs`)
7. **`req.cookie(name) -> Option<Cookie>`** (`src/request.rs`，用已有 cookie crate)
8. **`Response::html()`** 构造器 (`src/response.rs`)，`text/html; charset=utf-8`
9. **RequestId 中间件** (`src/middleware/request_id.rs`)
   - 默认从 `X-Request-Id` 读取或生成随机 ID（复用 rand，格式 hex）
   - 写入响应头 `X-Request-Id`，并通过 `request.extensions()` 供下游读取

### 交付物

- 文件变更：`fs.rs`、`router.rs`、`request.rs`、`response.rs`、`middleware/mod.rs`、新增 `middleware/request_id.rs`、`lib.rs`（re-export RequestId）、`Cargo.toml`（版本 1.3.0）
- 每个修复/新增均带单元测试；路径穿越与 405 用测试锁定行为
- 全量验证：cargo test + clippy -D warnings + fmt --check
- 更新 CHANGELOG.md（新增 1.3.0 段落，P0 项标注 Fixed，P1/P2 标注 Added）
- 零新依赖，预计 ~350 行

### 版本决策
含安全修复与行为修正（404/405 状态码），发布为 v1.3.0。无破坏性 API 变更。