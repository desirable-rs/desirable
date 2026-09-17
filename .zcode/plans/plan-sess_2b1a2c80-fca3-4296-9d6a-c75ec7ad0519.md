## desirable v1.10.0 — 中间件热路径优化

### P1. Cors 构造期预计算
- Cors 字段 allow_origin/allow_methods/allow_headers: String→HeaderValue（构造期 parse 一次，非法值提前到构造时 panic）；max_age: u32→HeaderValue（预格式化）；expose_headers: Option<HeaderValue>
- apply_headers 仅做 insert（&HeaderValue），每请求消除 4–6 次解析分配
- 单测：预计算字段正确、行为不变

### P2. Session 借用查找
- manager.rs 新增 pub(crate) get_cookie_value_str<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str>（零分配）
- SessionLayer 改用借用版；公开 get_cookie_value(String) 保留，内部委托借用版

### P3. 静态文件单 open
- 新内部 open_for_serve(path): open→fstat→is_dir 则改开 index.html；serve_file_with_cache 使用；with_dir_index 删除
- 常规文件 2 stat+1 open → 1 open+1 fstat；NotFound→404 语义保留
- 适配现有 fs 单测

### P4. Router 空快照免包装
- at(): middlewares_arc 为空时直接 Box::new(dest)，否则 ScopedEndpoint（语义等价：空链透传）

### P5. RequestId 微优化
- generate(): 查表写 hex 替代 16 次 format!
- HeaderName 经 OnceLock 静态持有，去掉每请求 from_static

### 验证与版本
- 全量 test（76+11+35）+ clippy -D warnings + fmt + examples
- 版本 1.10.0，CHANGELOG Performance 段；不发布，完成后汇报