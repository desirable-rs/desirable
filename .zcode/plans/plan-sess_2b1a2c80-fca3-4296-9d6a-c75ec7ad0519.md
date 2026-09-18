## desirable v2.2.0 — Range 请求、可信代理、强 ETag（路线图 v2.2）

### 1. Range 请求（fs.rs，零依赖）
- 新增 Range 解析器（纯函数，单测覆盖）：`bytes=start-end` / `bytes=start-` / `bytes=-suffix`；多区间或非 bytes 单位 → 忽略 Range 返回 200 全量（RFC 允许）
- serve_file_with_cache 中：非 304 且 Range 头存在时——
  - If-Range 存在且不匹配当前 ETag → 忽略 Range（200 全量）
  - 可满足 → `file.seek(start)` + `SizedBody(len=end-start+1)` → **206** + `Content-Range: bytes start-end/total`
  - 不可满足（start >= size）→ **416** + `Content-Range: bytes */total`
- 与预压缩交互：Range 作用于实际服务的表示（兄弟压缩文件）——语义一致
- 多部分区间（multipart/byteranges）明确不支持 → 200 全量（文档注明）

### 2. 强 ETag 选项（sha2，零新依赖）
- `ServeDir/ServeFile::strong_etag(bool)`，默认 false（保持弱 ETag）
- 开启后：sha256 内容哈希 → `"<hash前16hex>-<size>"`（无 W/ 前缀）
- 惰性 + 缓存：全局 `Mutex<HashMap<PathBuf, (mtime_secs, len, String)>>`，键 path、验证 mtime+len，文件未变则复用哈希（大文件只算一次）
- If-None-Match/If-Range 304 逻辑自动复用；与预压缩联动：哈希针对实际服务的表示

### 3. 可信代理 + Request::client_ip()
- 新增直接依赖 `ipnet = "2"`（树内已有，符合政策）
- `Server::trusted_proxies(["10.0.0.0/8", "127.0.0.1"])` builder——构造期 IpNet 解析（非法即 panic）
- 解析算法（右起第一个非信任 IP）：server.rs 在 dispatch 前计算，注入 `ClientIp(pub IpAddr)` extension
- `Request::client_ip() -> Option<IpAddr>`：读 extension，回退 remote_addr.ip()
- RateLimit 改用 client_ip() 作限流键（文档同步更新）
- Svc 与 dispatch 签名各加 trusted_proxies 参数（pub API 变更，CHANGELOG Breaking 注明）

### 4. 测试
- 单测：Range 解析器全分支、cidr contains、If-Range 判定
- e2e：无 Range→200；`bytes=0-9`→206 精确 10 字节 + Content-Range；`bytes=-5` 后缀；越界→416；If-Range 不匹配→200 全量；强 ETag 无 W/ 前缀且 304 往返；可信代理 XFF 解析（信任/不信任两态）+ RateLimit 按 XFF IP 限流

### 5. 收尾
- 版本 2.2.0；CHANGELOG（Added + Breaking 注记）；ROADMAP 勾选 v2.2
- 全量双配置 test/clippy/fmt/examples；完成后汇报，发布由用户决定