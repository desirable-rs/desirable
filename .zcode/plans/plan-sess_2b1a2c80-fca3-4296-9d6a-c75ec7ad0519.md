## desirable v2.1.0 — 内容协商与缓存（路线图 v2.1）

### 1. Compression 中间件（feature `compression`，新增首个 optional 依赖）
- Cargo.toml：`compression = ["dep:async-compression"]`；`async-compression = { version = "^0.4", features = ["gzip", "tokio"], optional = true }`
- 新增 `src/middleware/compression.rs`：`Compression::new()`（gzip，Level::Default）
  - 协商：请求 `Accept-Encoding` 含 `gzip`（简化解析，不含 q 值权重）才生效
  - 跳过条件：响应已有 Content-Encoding、无 Content-Length 且非流式、状态码非 2xx、Content-Type 不可压缩（image/* video/* application/octet-stream 等）、body 过小（< 256B）
  - 仅处理 `Body::Full`（内存中直接 gzip 压缩并重算 Content-Length）；`Body::Streaming` 透传不压（流式动态响应用例本就不该压；静态文件走预压缩资产路径）——文档明确记录
- cfg(feature) 门控代码与导出；dev 中按 feature 门控 e2e 测试

### 2. CI 补全覆盖（重要修复）
- .github/workflows/rust.yml 增加 `cargo build --all-features` + `cargo test --all-features` 步骤，否则 feature 代码永远不进 CI

### 3. 预压缩静态资产（零新依赖）
- `ServeDir::precompressed(bool)` / `ServeFile::precompressed(bool)`（默认 false）
- 开启后：`open_for_serve` 成功且请求 Accept-Encoding 含 br/gzip 时，探测同目录 `.br`/`.gz` 兄弟文件（br 优先）；命中则以压缩文件流式响应 + `Content-Encoding: br|gzip` + `Vary: Accept-Encoding`，Content-Type 仍取原扩展名，ETag 沿用弱 ETag（同一资源），SizedBody 精确 Content-Length 不变

### 4. Cache-Control 配置（零新依赖）
- `ServeDir::cache_control(&str)` / `ServeFile::cache_control(&str)` builder（如 `"public, max-age=31536000, immutable"`），参数穿透 `serve_file_with_cache`，在 200/304 响应上附加

### 5. Session::destroy()
- `Session` 包装体新增 `destroyed: bool`（不进序列化）+ `destroy()`（清空数据 + 置位）+ `is_destroyed()`
- SessionLayer persist 块：destroyed → `make_deletion_cookie()`；否则 modified → 正常 cookie

### 6. 测试与收尾
- 单测：压缩协商/跳过条件、Cache-Control 头、destroy 标志
- e2e（tests/nesting.rs）：gzip 压缩往返（feature 门控）、预压缩资产协商（.gz 优先/.br 优先/无 Accept-Encoding 回退）、Cache-Control 头、登录→destroy→删除 cookie→回到 anonymous
- 版本 2.1.0；CHANGELOG + ROADMAP 勾选；全量 test + clippy -D warnings（--all-features）+ fmt + examples + CI workflow 更新
- 完成后汇报，发布由用户决定