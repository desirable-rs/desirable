## desirable v1.9.0 — 性能优化 + 架构组织

### 性能（热路径）

P1. dispatch() 零分配：移除 method.clone() 与 path.to_string()（每请求一次堆分配）；match_path/matching_methods 已确认只需 &str 且不借用 req，用作用域借用替代
P2. 修复 get_cookie_value 的 base64 padding 截断 bug（split('=').nth(1) → split_once）；消除每分号段一次的 format! 分配（零分配前缀比较）
P3. fs.rs serve_file_with_cache：metadata+read 两次 syscall → File::open 一次 + f.metadata() + read_to_end
P4. 新增 pub(crate) Response::with_status_code(StatusCode, ..)，9 处内部字面量调用点迁移；HeaderValue .clone() 改 &引用

### 架构

A1. utils.rs（1字节空模块）：1.9.0 加 #[deprecated]，2.0 移除（严格 semver）
A2. Content-Type 常量收敛到 response.rs pub(crate)，删除 into_response.rs 重复副本与 allow(dead_code)
A3. into_response.rs：提取 build_response 助手，收敛 6 个重复 builder 链（4 个 tuple impl）
A4. session.rs（1620 行）拆为 session/ 目录：mod.rs(Session+文档)/config.rs/data.rs/error.rs/manager.rs，纯移动

### 基线与验证

- 改造前后 criterion 基线对比（--sample-size 20）
- 全量 test + clippy -D warnings + fmt + examples
- 版本 1.9.0；CHANGELOG 记录 cookie 截断 fix（Fixed）与 utils deprecation（Deprecated）
- 不发布，完成后汇报（发布由用户决定）