## desirable 2.0.0 — 流式响应体

### 1. 新增 src/body.rs：具体 Body 枚举
- `enum Body { Full(Bytes), Streaming(BoxBody<Bytes, BoxError>) }`，两变体 Unpin → 安全 Pin 投影
- 实现 http_body::Body（Data=Bytes, Error=BoxError；size_hint/is_end_stream 透传）
- 构造器：empty() / full(impl Into<Bytes>) / stream(S: Stream)（futures-core，树内新直接依赖）/ channel(cap)（tokio mpsc 背压，SSE 用）
- From<Bytes/String/&'static str/Full<Bytes>/BoxBody>；impl IntoResponse for Body
- body.rs 单元测试（poll_frame、size_hint、channel）

### 2. 类型与内部迁移
- types.rs：HyperResponse = hyper::Response<Body>（公开别名变更=主要 breaking）
- response.rs 11 处 Full::new → Body::full；into_response.rs build_response 接受 impl Into<Body>，新增 IntoResponse for Body，保留 Full<Bytes> impl
- router.rs HEAD 剥离 → Body::empty()；error.rs 两个测试改为 async + BodyExt::collect
- server.rs / examples 零改动

### 3. 静态文件默认流式
- 新增 SizedBody 包装：转发 poll_frame、size_hint=fstat 精确长度 → hyper 发 Content-Length 而非 chunked
- serve_file_with_cache：read_to_end → Body::stream(ReaderStream) + SizedBody；tokio-util 加 "io" feature
- 既有 e2e（裸 TCP 解析 Content-Length）直接验证无回归

### 4. 2.0 清理
- 移除 utils 空模块及 lib.rs 弃用声明
- 新 e2e：流式多 chunk 响应（channel + chunked 编码验证）
- CHANGELOG 2.0.0 段（Breaking / Added / Changed）+ ROADMAP 更新
- 版本 2.0.0；全量 test + clippy + fmt + examples；完成后汇报，发布由用户决定