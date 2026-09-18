## desirable v2.3.0 — WebSocket（feature `websocket`，路线图 v2.3）

### 1. 依赖（全部 feature 门控）
- `websocket = ["dep:tokio-tungstenite", "dep:futures-util"]`
- `tokio-tungstenite = { version = "^0.29", optional = true }`（本地 registry 0.29.0 可用，默认无 TLS——WSS 走反代）
- `futures-util = { version = "^0.3", optional = true }`（SinkExt/StreamExt，树内）

### 2. 新增 src/websocket.rs（cfg(feature) 门控）
- `WebSocketUpgrade::from_request(&Request) -> Result<Self, Response>`：校验 Upgrade: websocket / Sec-WebSocket-Key / Sec-WebSocket-Version: 13，失败返回 400
- `WebSocketUpgrade::on_upgrade(self, req: Request, callback: Arc<F>) -> Result<Response>`：
  - `tungstenite::handshake::derive_accept_key` 计算 Sec-WebSocket-Accept
  - 构造 101 Switching Protocols（Upgrade + Connection + Accept 头，Body::empty）
  - spawn 任务：`hyper::upgrade::on(&mut req.inner).await` → `WebSocketStream::from_raw_socket(io, Role::Server, None).await` → `callback(WebSocketConn)`
- `WebSocketConn`：包装 `WebSocketStream<Upgraded>`
  - `recv() -> Option<Result<Message, Error>>`、`send(Message)`、`send_text(impl Into<String>)`、`close()`
- 重导出 `tungstenite::Message` 供用户使用

### 3. Router::websocket（cfg 门控）
```rust
app.websocket("/ws", |conn: WebSocketConn| async move {
    while let Some(Ok(Message::Text(txt))) = conn.recv().await {
        conn.send_text(txt.to_string()).await.unwrap();  // echo
    }
});
```
- 注册为 GET 路由；回调存 Arc（用户闭包无需 Clone）
- 非 WebSocket 请求打到该路由 → 400

### 4. 测试（cfg 门控）
- e2e：echo 服务 + `tokio_tungstenite::connect_async` 客户端（双向收发 + 正常关闭）；非 upgrade GET → 400

### 5. 收尾
- 版本 2.3.0；CHANGELOG/ROADMAP/README；双配置（default/all-features）全量 test + clippy + fmt + examples；完成后汇报，发布由用户决定