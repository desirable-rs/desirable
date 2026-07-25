# desirable v1.2.0：来自真实使用反馈的 6 个 DX 打磨

> 一个好框架的升级，不该是新功能堆砌，而是消除那些让你日常皱眉的小摩擦。

---

## 背景

v1.1.0 发布后，我们收到了来自实际使用的反馈。没有致命 bug，没有架构缺陷——全是"这里要是能再顺手一点就好了"。

这 6 个问题全部来自日常编码，每一个都曾经让使用者多打几个字符、多写一行 `unwrap()`、多思考一秒生命周期。

v1.2.0 解决了它们全部。

---

## 1. `Response::json()` 的 `'static` 魔咒 🔥 最大痛点

**之前：**

```rust
#[derive(Serialize)]
struct User { name: String, age: i32 }

async fn handler() -> Result {
    let user = User { name: "Alice".into(), age: 30 };

    // ❌ 必须传 owned value，不能传引用
    Response::json(user)?

    // ❌ 这样不行——json() 要求 T: 'static
    // Response::json(&user)?
}
```

**为什么？** 旧签名要求 `T: Serialize + Sized + Send + Sync + 'static`。`Send + Sync + 'static` 对序列化完全多余——`serde_json::to_vec` 只借用数据，结果转成自有 `Bytes`，不持有任何引用。

**现在：**

```rust
async fn handler() -> Result {
    let user = User { name: "Alice".into(), age: 30 };

    // ✅ 传引用，不 clone，不 move
    // ✅ 不用 'static
    // ✅ 不用 ? （不再返回 Result）
    Ok(Response::json(&user))
}
```

签名的变化：

```diff
- pub fn json<T>(payload: T) -> Result<Self>
-   where T: Serialize + Sized + Send + Sync + 'static

+ pub fn json<T: Serialize>(payload: T) -> Response
```

---

## 2. `Response::builder()` 终于来了

**之前：** 要同时设状态码、响应头、JSON body：

```rust
// 只能用工厂方法，还要 deal with Result
let mut resp = Response::json(&data)?;          // 等一下，json 不返回 Result 了
resp.set_header("X-Request-Id".parse().unwrap(), id.parse().unwrap());
// 但是改不了 status... json() 默认 200
```

**现在：**

```rust
let resp = Response::builder()
    .status(StatusCode::CREATED)
    .header("X-Request-Id".parse().unwrap(), id.parse().unwrap())
    .json(&data);
```

链式 API，直觉友好。完整方法：

| 方法 | 说明 |
|------|------|
| `.status(StatusCode)` | 设置状态码 |
| `.status_u16(201)` | `u16` → `StatusCode` |
| `.header(name, value)` | 添加响应头 |
| `.json(&data)` | JSON body（infallible） |
| `.text("hello")` | 文本 body + Content-Type |
| `.body(bytes)` | 原始 body |

---

## 3. `query()` 不再逼你处理 `Option`

**之前：**

```rust
#[derive(Deserialize)]
struct Pagination { page: u32, limit: u32 }

async fn list(req: Request) -> Result {
    // 要处理 Option...
    let pagination = req.query::<Pagination>()?;
    let (page, limit) = match pagination {
        Some(p) => (p.page, p.limit),
        None => (1, 20),  // 手动设默认值
    };
    // ...
}
```

**现在：** 新增 `query_or_default()`，配合 `#[derive(Default)]` 一行搞定：

```rust
#[derive(Deserialize, Default)]
struct Pagination { page: u32, limit: u32 }

async fn list(req: Request) -> Result {
    let p = req.query_or_default::<Pagination>()?;
    // p.page == 0, p.limit == 0 — 就是 Rust 的 Default
}
```

原来的 `query()` 保留不变，只是多了一个选择。

---

## 4. `body_json()` — 名字就是文档

**之前：**

```rust
async fn create(mut req: Request) -> Result {
    let user: User = req.body().await?;
    //                   ^^^^ 是 JSON body？还是 form？还是 raw bytes？
}
```

`body()` 太泛了。**现在：**

```rust
async fn create(mut req: Request) -> Result {
    let user: User = req.body_json().await?;
    //                   ^^^^^^^^^ 一看就知道是 JSON
}
```

`body_json()` 就是 `body()` 的别名，签名完全一致。改一个名字，代码可读性提升明显。

---

## 5. `param_str()` — 告别 `::<String>`

Rust 的 turbofish 在日常开发中最烦人的场景之一：提取 String 参数。

**之前：**

```rust
// Route: /files/:filename
async fn get_file(req: Request) -> Result {
    let filename = req.param::<String>("filename")?;
    //                   ^^^^^^^^^^ 每次都要写
}
```

**现在：**

```rust
async fn get_file(req: Request) -> Result {
    let filename = req.param_str("filename")?;
    //                   ^^^^^^^^^ 干净
}
```

`param_str()` 内部就是 `param::<String>()`。大部分路径参数（ID、slug、文件名）都是 String，这个糖让代码干净很多。

---

## 6. `serde_json::Value` 可以直接返回了 🎁 Bonus

文档里写了但代码没实现的一个功能：

**之前：** 这样写编译不过：
```rust
async fn handler() -> impl IntoResponse {
    serde_json::json!({"msg": "hello"})  // ❌ Value 没有实现 IntoResponse
}
```

**现在：**

```rust
async fn handler() -> impl IntoResponse {
    serde_json::json!({"msg": "hello", "count": 42})
}
```

自动转 JSON 响应，`Content-Type: application/json`。

---

## 破坏性变更（唯一的）

`Response::json()` 的返回类型从 `Result<Response>` 改为 `Response`。

**迁移只需删掉 `?` 或 `.unwrap()`：**

```diff
- let resp = Response::json(&data)?;
+ let resp = Response::json(&data);
```

序列化失败会直接 panic——对于正常类型，这永远不会发生。

---

## 升级

```toml
[dependencies]
desirable = "1.2"
```

Cargo.toml 改个版本号即可。所有 v1.1 的 API 都兼容（除了 json 返回类型，见上）。

---

## 总结

| # | 痛点 | 修复 | 少打的字符 |
|---|------|------|-----------|
| 1 | json 要求 `'static` | 放宽至 `Serialize` | 省去 clone/move |
| 2 | 缺 builder | `Response::builder()` | 链式更直观 |
| 3 | query 返回 Option | `query_or_default()` | 省 4 行 match |
| 4 | body 命名模糊 | `body_json()` | 名字即文档 |
| 5 | param 要 turbofish | `param_str()` | 省 10 个字符 |
| 6 | json 返回 Result | 改签名为 Response | 省 `?` / `.unwrap()` |

**6 个问题，~120 行代码，零新依赖。** 全部来自真实使用反馈。

如果你也在用 desirable，欢迎 [提交 issue](https://github.com/desirable-rs/desirable/issues) 告诉我们你的痛点——下一个版本可能就是为你修的。
