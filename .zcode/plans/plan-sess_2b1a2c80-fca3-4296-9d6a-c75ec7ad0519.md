## desirable v2.5.0 — TLS + HTTP/2（feature `tls`，路线图 v2.5）

### 1. 依赖（feature `tls` 门控，首个全新依赖子树）
- `tls = ["dep:tokio-rustls", "dep:rustls-pemfile"]`
- `tokio-rustls = { version = "^0.26", optional = true, default-features = false, features = ["ring", "tls12"] }`
- `rustls-pemfile = { version = "^2", optional = true }`
- rustls 0.23 + pki-types 经由上述引入；默认构建完全不受影响

### 2. 新增 src/tls.rs（cfg 门控）
- `TlsAcceptorBuilder` 助手：`server_config_from_pem(certs_pem: &[u8], key_pem: &[u8]) -> io::Result<ServerConfig>`
  - rustls-pemfile 解析证书链/私钥；`builder_with_provider(ring)` + `with_safe_default_protocol_versions`
  - 预设 ALPN：`[h2, http/1.1]`
- `Server::tls_config(Arc<rustls::ServerConfig>)` builder：启用 TCP 监听的 TLS
- 导出 `tokio_rustls::TlsAcceptor`、`rustls::ServerConfig` 别名便于用户自建

### 3. server.rs 集成
- Server 增加 `tls: Option<Arc<rustls::ServerConfig>>`（cfg tls）
- accept 流程重构：TCP accept 返回原始 `TcpStream`（AnyStream 构建延后）
- spawn 内：TLS 开启时 `TlsAcceptor::accept(tcp)`（握手失败仅日志+断开）；TLS 流的 ALPN 探测 informational
- **h1/h2 统一服务**：auto builder 的嗅探机制自动识别 ALPN-h2 前缀与 h1，零分支；h1 timer/header_read_timeout + h2 timer 双配置
- AnyStream 增加 `Tls(TokioIo<server::TlsStream<TcpStream>>)` 变体（hyper Read/Write 委托）；UDS 保持明文

### 4. 测试（cfg 门控）
- 提交自签名测试证书/私钥 fixture（openssl 生成，CN=localhost + SAN IP:127.0.0.1）
- e2e 1：rustls 客户端（ALPN http/1.1，信任测试 CA）HTTP/1.1 over TLS 请求 → 200 + 协商断言
- e2e 2：ALPN h2 → `hyper::client::conn::http2::handshake` → 发请求 → 断言响应体（证明 h2 全通）

### 5. 收尾
- 版本 2.5.0；CHANGELOG + ROADMAP（勾 v2.5、注明 ws-over-h2 限制）+ README TLS 段
- 双配置全量验证；完成后汇报，发布由用户决定