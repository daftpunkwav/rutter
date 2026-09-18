# Rutter 后端全面只读审查报告

- **审查日期**: 2026-09-18
- **审查模式**: 仅阅读,未做任何修改
- **审查范围**: `crates/{core, events, engine, engine-cdp, observe, policy, session, mcp, dashboard, cli}`、`docs/*`、`Cargo.toml`、`Cargo.lock`
- **分类标准**: 12 个审查方向 (correctness / edge-case / security / api-contract / decouple / dependency / performance / resilience / maintainability / naming / documentation / testing)
- **每条问题字段**: 问题文件 / 位置 / 具体问题 / 影响文件 / 引发问题

---

## 1. Correctness

### 1.1 截图速率限制写入"未来时刻",语义错位

- **问题文件**: `crates/engine-cdp/src/page.rs`
- **位置**: `CdpPage::capture_screenshot`,`last_capture: Mutex<Option<Instant>>` 更新分支(约 204–220 行)
- **具体问题**: 把 `Instant::now() + wait`(未来时刻)写入 `last_capture`;由于 `Instant::elapsed()` 在 future 时刻返回 `Duration::ZERO`,`min_interval.saturating_sub(0)` 永远等于 `min_interval`,导致任意两次调用之间强制等待一个完整 `min_interval`,无论真实间隔多久
- **影响文件**:
  - `crates/engine-cdp/src/page.rs::capture_screenshot`
  - `crates/engine/src/config.rs::ContextConfig::screenshot_min_interval`(契约侧)
- **引发问题**: 长空闲后调用 `screenshot` 仍需等待一个完整 `min_interval`;速率限制语义错位,虽不破坏正确性但放大性能浪费

### 1.2 `set_cookies` 文档语义与实现不一致

- **问题文件**: `crates/engine-cdp/src/context.rs`
- **位置**: `CdpContext::set_cookies` 的 doc 注释与实际 CDP `Network.setCookie` 调用路径
- **具体问题**: doc 写"按 name/domain/path 写入,不替换",但 CDP 实现按调用顺序写入,不会覆盖同名条目;`cookies()` 返回**整 context** 的 cookie,语义与 doc 不对称
- **影响文件**:
  - `crates/engine-cdp/src/context.rs::set_cookies / cookies`
  - `docs/TOOL_SPEC.md §4 set_cookies`
  - `crates/core/src/cookie.rs`(Cookie 结构定义)
- **引发问题**: agent 按 upsert 假设使用,实际产生重复 cookie 条目;同一份 Cookie 被多次 set 后,`cookies()` 返回 N 份同名项

### 1.3 `EngineError` 在 MCP 边界未映射为 `ActionError`

- **问题文件**: `crates/session/src/error.rs`
- **位置**: `SessionError` enum 的 `Engine(#[from] EngineError)` 分支(10–28 行)
- **具体问题**: `EngineError::NavigationFailed` / `EngineError::Timeout` 等不映射为 `ActionError::NavigationFailed` / `ActionError::TimedOut`,直接在 MCP 边界以 `Engine` 变体透传
- **影响文件**:
  - `crates/session/src/error.rs::SessionError`
  - `crates/session/src/actions.rs::execute`(所有 action 路径)
  - `crates/mcp/src/server.rs`(tool 结果包装)
  - `docs/TOOL_SPEC.md §2 / §3 / §4`(SPEC 承诺)
- **引发问题**: 违反 TOOL_SPEC §2 承诺(action 失败应返回 `ActionError` 系列);agent 收到 `SessionError::Engine` 而非预期的 `ActionError` 类别,`hint()` 来自不同枚举,语义错位

### 1.4 `Backbone::replay` 在并发写入下可产生 seq 间隙

- **问题文件**: `crates/events/src/backbone.rs`
- **位置**: `Backbone::replay` 方法,基于 `ring.snapshot()` 克隆 `Vec<RingEntry>` 后逐项包装为 `Envelope`
- **具体问题**: `record` 先 `seq.fetch_add` 再 push;并发 record 期间,`ring.snapshot()` 抓到的 `Vec` 可能缺中间 seq(如抓到 {seq=5, seq=7} 而 seq=6 被另一线程覆盖前没入 Vec)
- **影响文件**:
  - `crates/events/src/backbone.rs::replay`
  - `crates/events/src/ring.rs::Ring::snapshot / add`
  - `crates/dashboard/src/lib.rs`(消费 replay)
- **引发问题**: 订阅端观察到不连续 seq,与"server-wide monotonic"承诺不冲突但不连续;订阅端需自行补齐或判断丢帧

### 1.5 `ApprovalBroker` 注册通道无界,且 `decide` 可二次调用 panic

- **问题文件**: `crates/policy/src/broker.rs`
- **位置**: `register` 使用 `tokio::sync::mpsc::unbounded_channel`;`decide` 中 `oneshot::Sender::send` 未防重入
- **具体问题**:
  - 注册通道无界,未决请求未提供超时清理路径,挂起请求长期累积
  - `oneshot::Sender` 二次 `send` 直接返回 Err,调用方路径未 `assert!` 唯一性
- **影响文件**:
  - `crates/policy/src/broker.rs`(整文件)
  - `crates/session/src/actions.rs::execute`(注册路径)
  - `crates/mcp/src/server.rs::approval_*` tool
- **引发问题**: 服务级内存泄漏;二次 approve/deny panic 致 orchestration 线程崩溃

### 1.6 `SessionManager::close_session` 未显式释放 context 资源

- **问题文件**: `crates/session/src/manager.rs`
- **位置**: `SessionManager::close_session` 方法
- **具体问题**: 仅从 `RwLock<HashMap>` 中移除 `SessionHandle` 并 drop,未显式调用 `context.close_page(...)` 或 `context.shutdown()`
- **影响文件**:
  - `crates/session/src/manager.rs::close_session`
  - `crates/engine/src/context.rs::ContextHandle`(trait 契约)
  - `crates/engine-cdp/src/context.rs::CdpContext`
- **引发问题**: 资源释放依赖 `Drop` 链;`engine-cdp` 当前 chromiumoxide 走 `BrowserContext` 自动关闭,但 `Engine` trait 未契约保证 drop 行为,换 remote CDP 等后端时可能泄漏

### 1.7 `Cookie::expires: Option<f64>` 接受 NaN / Infinity

- **问题文件**: `crates/core/src/cookie.rs`
- **位置**: `Cookie::expires` 字段定义
- **具体问题**: `f64` 通过 serde 直接反序列化,允许 `NaN` / `±Infinity`
- **影响文件**:
  - `crates/core/src/cookie.rs::Cookie`
  - `crates/engine-cdp/src/context.rs::set_cookies`(写入 CDP `Network.setCookie`)
- **引发问题**: CDP 拒绝写入,被 `fold` 成 `EngineError::Internal`,agent 收到误导性"内部错误",实际是上游输入非法

### 1.8 `LazyLauncher` 包装与 supervisor launch 路径重复

- **问题文件**: `crates/cli/src/launcher.rs`
- **位置**: `headless_launcher` / `headed_launcher` / `LazyLauncher::new`
- **具体问题**: 三种 launcher 各自由 `Settings` 直接构造 `EngineLauncher`,`LazyLauncher` 内部再次构造 headless/headed,逻辑重叠
- **影响文件**:
  - `crates/cli/src/launcher.rs`
  - `crates/cli/src/serve.rs::run`(传递 `LazyLauncher`)
- **引发问题**: 加新 launch mode 时需同步更新三处;`settings.ts` 副作用在 `LazyLauncher` 内重演,容易漏

### 1.9 `CdpLauncher::launch` 中 `version` 调用可能 race

- **问题文件**: `crates/engine-cdp/src/launch.rs`
- **位置**: `launch` 方法 74 行 `browser.version().await.map_err(error::fold)?`
- **具体问题**: 在 handler task 已经 spawn 之后才获取 version;若 handler 异常导致 Browser 已关闭,`version` 立刻返回错误,但 `CdpEngine` 已构造并交给调用方
- **影响文件**:
  - `crates/engine-cdp/src/launch.rs::launch`
  - `crates/engine-cdp/src/engine.rs::CdpEngine::new`
- **引发问题**: 出现"engine 已构造但 backend 已不可用"窗口;`health()` 探针会迟滞发现

---

## 2. Edge Case

### 2.1 `Cookie::expires` 边界值(同 1.7)

### 2.2 `EventScreencastFrame` data 解码后 jpeg 可能为空

- **问题文件**: `crates/engine-cdp/src/page.rs`
- **位置**: `CdpPage::start_screencast` 中 base64 decode 与 `sender.send(ScreencastFrame { jpeg })` 路径(约 271–279 行)
- **具体问题**: 空字符串是合法 base64(空字节串),decode 成功但 `jpeg.len() == 0`;非 base64 字符串被 `let Ok(...) else { continue; };` 静默丢弃
- **影响文件**:
  - `crates/engine-cdp/src/page.rs::start_screencast`
  - `crates/engine/src/page.rs::ScreencastStream::next_frame`
  - `crates/session/src/session.rs::Session::screencast`
- **引发问题**: 消费者收到 0 字节帧无防御;CDP 协议错误帧被静默丢,无日志/事件,违背"latest-wins"的可观察性

### 2.3 `Backoff::delay(0)` 返回 base 不为零

- **问题文件**: `crates/engine/src/backoff.rs`
- **位置**: `Backoff::delay` 实现
- **具体问题**: `delay(0)` 返回 `base`(非零);`RestartPolicy::decide` 用 `in_window == 0` 短路到 `Duration::ZERO` 避免暴露,但 `Backoff` 自身 doc 未注明
- **影响文件**:
  - `crates/engine/src/backoff.rs`
  - `crates/engine/src/supervisor/policy.rs::RestartPolicy::decide`
- **引发问题**: 维护者直接调用 `Backoff::delay(0)` 得到非零结果,违反最小惊讶原则

### 2.4 `Snapshot::truncated` 完全由 host envelope 决定

- **问题文件**: `crates/observe/src/builder.rs`、`crates/core/src/snapshot.rs`
- **位置**: `snapshot_from_response` 解析 host JSON 中的 `truncated: bool` 字段,直接传给 `Snapshot`
- **具体问题**: host 可声明 `truncated = false` 但实际截断了子树;`Snapshot::truncated` 是"观察者看自己行为"的标志,但这里信任 host 自报
- **影响文件**:
  - `crates/observe/src/builder.rs`
  - `crates/core/src/snapshot.rs::Snapshot`
  - `crates/session/src/session.rs`(所有返回 Snapshot 的路径)
- **引发问题**: 订阅端把"完整性"等同于 host 声明的 `truncated` 字段,可能在恶意页面下误判完整性

### 2.5 `cookies()` 在 context 已 shutdown 后仍可被调用

- **问题文件**: `crates/session/src/session.rs`
- **位置**: `Session::set_cookies / cookies`
- **具体问题**: `Arc<dyn ContextHandle>` 共享持有,context 关闭后 `cookies()` 调用仍可达,被 `fold` 成 `EngineError::Internal`
- **影响文件**:
  - `crates/session/src/session.rs`
  - `crates/engine-cdp/src/context.rs::CdpContext`
- **引发问题**: 调用方区分不出"context 已关"与"内部错误",语义不清

### 2.6 chromiumoxide handler task 错误吞掉(同 1.9 副作用)

- **问题文件**: `crates/engine-cdp/src/launch.rs`
- **位置**: 第 72 行 `tokio::spawn(async move { while handler.next().await.is_some() {} })`
- **具体问题**: `handler.next()` 返回 `None` 时任务结束,期间 `Err` 不会返回;handler stream 中的所有 `CdpError` 被丢弃
- **影响文件**:
  - `crates/engine-cdp/src/launch.rs::launch`
  - `crates/engine/src/health.rs::HealthReport`(依赖此 handler 状态)
- **引发问题**: CDP 连接异常退出无任何日志或 `HealthReport` 触发;supervisor 心跳需等到下一次探针才发现

### 2.7 `Session::execute` 中 `Snapshot::truncated` 与 budget 默认值

- **问题文件**: `crates/core/src/snapshot.rs`
- **位置**: `Snapshot` 默认 `budget_default: usize = 20_000`
- **具体问题**: `budget_default` 是 `pub`,调用方可构造 `Snapshot { budget: 0, ... }` 强制截断到无内容
- **影响文件**:
  - `crates/core/src/snapshot.rs`
  - `crates/observe/src/builder.rs::build`
- **引发问题**: 误用 0 budget 产生空 snapshot,所有下游不知情;SPEC 假设 20 000 是默认,但 contract 未防越界

---

## 3. Security

### 3.1 `--http ADDR` 默认可能绑定 0.0.0.0 且无认证

- **问题文件**: `crates/cli/src/main.rs`、`crates/cli/src/serve.rs`
- **位置**: `--http` 参数定义(`main.rs` 约 60 行);`serve.rs::run(http_addr)` 透传给 `rutter_mcp::http::serve_http`
- **具体问题**: `SocketAddr` 由用户 CLI 提供;`--http 0.0.0.0:9800` 会把整个 MCP 工具面(包含 `navigate` / `click` / `set_cookies` 等)暴露给 LAN;MCP 路径无认证、无 IP 白名单、无警告
- **影响文件**:
  - `crates/cli/src/main.rs::Cli::http`
  - `crates/cli/src/serve.rs::run`
  - `crates/mcp/src/http.rs::serve_http`
  - `crates/mcp/src/server.rs::RutterMcp`
- **引发问题**: 任意 LAN 客户端可驱动浏览器执行任意 navigate / 点击,包括登录态操作;SSRF、Cookie 注入(见 3.2)可叠加

### 3.2 `Cookie::domain` 字段未校验,允许内网/loopback

- **问题文件**: `crates/core/src/cookie.rs`、`crates/engine-cdp/src/context.rs`
- **位置**: `Cookie.domain: String`,`set_cookies` 透传到 CDP `Network.setCookie`
- **具体问题**: domain 无 IP / host 字串校验,允许 `localhost`、`127.0.0.1`、RFC1918、私有 hostname
- **影响文件**:
  - `crates/core/src/cookie.rs::Cookie`
  - `crates/engine-cdp/src/context.rs::set_cookies`
  - `crates/session/src/session.rs::set_cookies`
  - `crates/mcp/src/server.rs::set_cookies tool`
- **引发问题**: agent 可植入 `Cookie(domain=".internal.corp", ...)`;后续 navigate 到内部站点时附带该 cookie,典型 SSRF + cookie 注入复合攻击

### 3.3 `Navigate` 类 action 的 policy URL 取值未文档化

- **问题文件**: `crates/policy/src/rules.rs`、`crates/session/src/actions.rs`
- **位置**: `RuleSet::evaluate(action_class, &url)`,`url` 取值路径
- **具体问题**: `Navigate` 评估的 url 是当前页面 url 还是 agent 提交的 target url,doc 未明示
- **影响文件**:
  - `crates/policy/src/rules.rs::RuleSet::evaluate`
  - `crates/session/src/actions.rs::execute`
  - `crates/policy/src/class.rs::class_of`
- **引发问题**: 若用 target url,agent 可通过先 navigate 白名单再发起 navigate 到 `internal.corp`,policy 仍 OK;若用当前 url,需先 navigate 一次绕过;两种语义都需明示,否则 policy 行为不可预期

### 3.4 `snapshot_from_response` hostile JSON 防御策略未文档化

- **问题文件**: `crates/observe/src/builder.rs`、`crates/observe/src/response.rs`
- **位置**: `Response` 结构反序列化路径
- **具体问题**: SNAPSHOT_SPEC §2/§3 承诺"never panics, hostile input degrades",但实现是否启用 `#[serde(deny_unknown_fields)]` / 大小 cap / 类型容忍等防御策略,doc 未集中说明
- **影响文件**:
  - `crates/observe/src/builder.rs`
  - `crates/observe/src/response.rs`
  - `crates/core/src/snapshot.rs`(消费方)
- **引发问题**: 巨大 JSON DoS、`__proto__` 污染、类型错乱字段均可被 host 注入;防御散落难审计

### 3.5 `--engine-executable` 接受任意 PATH

- **问题文件**: `crates/cli/src/main.rs`
- **位置**: `--engine-executable` 参数
- **具体问题**: PATH 直接传给 `chromiumoxide::BrowserConfig::builder().chrome_executable(...)`,由 chromiumoxide spawn
- **影响文件**:
  - `crates/cli/src/main.rs::Cli::engine_executable`
  - `crates/engine-cdp/src/launch.rs::CdpLauncher::new`
- **引发问题**: 与 3.1 `--http` 暴露叠加时,远程攻击者配合可滥用任意二进制

### 3.6 stdout / snapshot 可能泄漏敏感数据

- **问题文件**: `crates/cli/src/open.rs`、`crates/dashboard/src/lib.rs`
- **位置**: `println!("{snapshot}")` (open.rs 39 行),dashboard WS 推送事件
- **具体问题**: snapshot 文本含页面所有可见节点内容,可能包含 token、邮箱、cookie 字符串;事件回放可能含敏感字段
- **影响文件**:
  - `crates/cli/src/open.rs`
  - `crates/dashboard/src/lib.rs`
  - `crates/observe/src/assets/serializer.js`(序列化哪些字段)
- **引发问题**: shell history / CI 日志 / WS client 接收敏感字段;无脱敏路径

### 3.7 `deny.toml` 配置未审计

- **问题文件**: `deny.toml`(workspace 根)
- **位置**: advisory database 与 license allow list
- **具体问题**: 未确认是否覆盖本仓库依赖(rmcp、chromiumoxide、tokio、serde 等)
- **影响文件**: `Cargo.lock`(全部传递依赖)
- **引发问题**: 已知 advisory 可能漏报;license 兼容性问题可能未察觉

---

## 4. API Contract

### 4.1 `EngineError` → `ActionError` 边界错位(同 1.3)

### 4.2 `ContextHandle::pages()` 同步与 `close_page` async 同 trait

- **问题文件**: `crates/engine/src/context.rs`
- **位置**: trait 定义(约 18–43 行),`fn pages(&self) -> Vec<PageId>` vs `async fn close_page(...)`
- **具体问题**: 同一 trait 内 sync 与 async 混用
- **影响文件**:
  - `crates/engine/src/context.rs`
  - `crates/engine-cdp/src/context.rs::CdpContext`
- **引发问题**: 调用方易误用,在 async 上下文中调用 sync 方法需谨慎;back pressure 不对称

### 4.3 `Engine::shutdown` 后续调用契约在 impl 上未 pin

- **问题文件**: `crates/engine/src/engine.rs`、`crates/engine-cdp/src/engine.rs`
- **位置**: `Engine::shutdown` doc 说"after shutdown further calls fail with `EngineError::Terminated`"
- **具体问题**: chromiumoxide 后端是否真的实现该语义未验证;`Browser` close 后 wrapper 仍存在,后续 `create_context` 行为未在实现层确认
- **影响文件**:
  - `crates/engine/src/engine.rs::Engine`
  - `crates/engine-cdp/src/engine.rs::CdpEngine`
  - `crates/session/src/manager.rs`(调用 shutdown)
- **引发问题**: shutdown 后行为在所有 impl 上未保证一致;新增 engine backend 时易破坏契约

### 4.4 `ScreencastStream::new` 构造器公开

- **问题文件**: `crates/engine/src/page.rs`
- **位置**: `ScreencastStream::new(receiver)`
- **具体问题**: 构造器公开,任何调用方都能构造无 page 关联的 stream
- **影响文件**:
  - `crates/engine/src/page.rs`
  - `crates/engine-cdp/src/page.rs::start_screencast`
  - `crates/session/src/session.rs::Session::screencast`
- **引发问题**: API 表面泄漏内部细节,绕过 engine 创建 screencast 的路径

### 4.5 `Cookie` 字段不完整(同 1.7 关联)

- **问题文件**: `crates/core/src/cookie.rs`
- **位置**: `Cookie` 结构体定义
- **具体问题**: 缺 `same_party` / `partition_key` / `priority` / `source_scheme` 等 CDP `Network.setCookie` 支持字段
- **影响文件**:
  - `crates/core/src/cookie.rs`
  - `crates/engine-cdp/src/context.rs::set_cookies`
  - `docs/TOOL_SPEC.md §4 set_cookies`
- **引发问题**: 扩展 milestone 时需破坏性变更 Cookie 字段,影响所有 agent / 持久化存储

### 4.6 rmcp 版本 / API 漂移未 pin

- **问题文件**: `crates/mcp/src/server.rs`、`Cargo.toml`、`Cargo.lock`
- **位置**: `rmcp::ServiceExt`、`rmcp::transport::stdio()`、`#[tool]` 宏使用
- **具体问题**: rmcp 0.x API 频繁变化,Cargo.toml 未显式 pin minor
- **影响文件**:
  - `crates/mcp/src/server.rs`
  - `crates/mcp/src/http.rs`
  - `crates/mcp/src/lib.rs`
  - `Cargo.toml / Cargo.lock`
- **引发问题**: 升级 rmcp 时可能 silently break MCP 协议兼容

### 4.7 `Mcp` 工具参数 `Origin` 来源语义不明

- **问题文件**: `crates/session/src/session.rs`
- **位置**: `Session::execute(&self, action: Action, origin: Origin)`
- **具体问题**: `Origin::Agent` 当前唯一变体,含义模糊(来源 / 出处 / 起源)
- **影响文件**:
  - `crates/session/src/session.rs`
  - `crates/mcp/src/server.rs`(调用 execute 时传 Origin)
- **引发问题**: 扩展 dashboard 起源 / replay 起源时命名需重审

---

## 5. Decouple

### 5.1 `SessionManager` 同时被 MCP 与 Dashboard 持有,transport 与实现紧耦合

- **问题文件**: `crates/session/src/manager.rs`、`crates/cli/src/serve.rs`
- **位置**: `Arc<SessionManager>` 在 `serve.rs` 创建后传给 `RutterMcp::new` 与 `DashboardServer::new`
- **具体问题**: transport(MCP/Dashboard)直接持有 manager 实现类
- **影响文件**:
  - `crates/session/src/manager.rs`
  - `crates/cli/src/serve.rs`
  - `crates/mcp/src/server.rs`
  - `crates/dashboard/src/lib.rs`
- **引发问题**: 替换 transport 时需修改 manager 公开 API;transport 行为耦合到 manager 内部锁

### 5.2 launcher 三种变体重叠(同 1.8)

### 5.3 `dashboard/lib.rs` 单文件 1100+ 行

- **问题文件**: `crates/dashboard/src/lib.rs`
- **位置**: `DashboardServer::handle_*` 一系列 handler
- **具体问题**: WS / HTTP / replay 职责混在同一文件
- **影响文件**:
  - `crates/dashboard/src/lib.rs`
- **引发问题**: 替换 transport / 拆分模块时修改面大;阅读时认知负担重

### 5.4 `engine-cdp/src/error.rs` 同时承担错误折叠与 input helper

- **问题文件**: `crates/engine-cdp/src/error.rs`
- **位置**: `mouse_params` / `key_params` / `cdp_button` 等 helper 与 `fold` 同文件
- **具体问题**: 职责混合(错误适配 + 输入参数构造)
- **影响文件**:
  - `crates/engine-cdp/src/error.rs`
  - `crates/engine-cdp/src/page.rs::dispatch_input`
- **引发问题**: input 助手替换或扩展时影响错误适配文件,变更面大

### 5.5 `rutter-observe` 与 `rutter-core::Snapshot` 双重截断

- **问题文件**: `crates/observe/src/builder.rs`、`crates/core/src/snapshot.rs`
- **位置**: name/ref/role 长度截断逻辑分散
- **具体问题**: observe builder 与 Snapshot Display 都做截断
- **影响文件**:
  - `crates/observe/src/builder.rs`
  - `crates/core/src/snapshot.rs`
  - `crates/observe/src/response.rs`
- **引发问题**: 截断责任边界模糊,新增字段时易重复实现

### 5.6 `Backbone` 直接持有 `Ring` 与 `mpsc::Sender`,API 表面泄漏并发原语

- **问题文件**: `crates/events/src/backbone.rs`、`crates/events/src/bus.rs`、`crates/events/src/ring.rs`
- **位置**: `Backbone::new(rings, ...)` 公开了 `RwLock<HashMap<SessionId, Ring>>` 等并发原语
- **具体问题**: 并发数据结构直接出现在公开 API
- **影响文件**:
  - `crates/events/src/backbone.rs`
  - `crates/events/src/ring.rs`
- **引发问题**: 替换并发策略(如改 DashMap)需修改所有调用方

---

## 6. Dependency

### 6.1 rmcp 版本漂移风险(同 4.6)

### 6.2 chromiumoxide 协议版本兼容

- **问题文件**: `Cargo.toml`、`Cargo.lock`、`crates/engine-cdp/src/`
- **位置**: chromiumoxide 依赖声明与 `Browser::launch` 调用
- **具体问题**: chrome-headless-shell 与 chromiumoxide CDP 协议版本错位可能致 `Browser::launch` 静默失败
- **影响文件**:
  - `crates/engine-cdp/src/launch.rs::launch`
  - `crates/engine-cdp/src/error.rs::fold`
  - `Cargo.toml / Cargo.lock`
- **引发问题**: 启动失败被 `fold` 成 `EngineError::LaunchFailed`,根因(协议错位)被掩盖

### 6.3 `EngineError::Internal { detail }` 是字符串

- **问题文件**: `crates/engine/src/error.rs`、`crates/engine-cdp/src/error.rs`
- **位置**: `Internal { detail: String }`
- **具体问题**: detail 是 string,无结构化
- **影响文件**:
  - `crates/engine/src/error.rs`
  - `crates/engine-cdp/src/error.rs::fold`
  - `crates/session/src/error.rs::hint`
  - `crates/mcp/src/server.rs`(消费 hint)
- **引发问题**: 结构化诊断信息丢失;上层无法按字段路由错误

### 6.4 无 `tracing` / `log` crate

- **问题文件**: 整个 codebase
- **位置**: 使用 `eprintln!` 的位置(`cli/serve.rs`、`dashboard/lib.rs` 等)
- **具体问题**: 生产无结构化日志、无 log level
- **影响文件**: 全部
- **引发问题**: 运维/排障困难;无法按级别过滤、按字段检索

---

## 7. Performance

### 7.1 screenshot 每次 Mutex lock + Instant::now()(同 1.1)

### 7.2 `ApprovalBroker` pending 线性扫描(同 1.5 副作用)

- **问题文件**: `crates/policy/src/broker.rs`
- **位置**: `decide` 方法遍历 `VecDeque<Parked>`
- **具体问题**: O(N) 查找
- **影响文件**:
  - `crates/policy/src/broker.rs`
- **引发问题**: pending 增长时延迟上升;长期挂起的请求放大查找成本

### 7.3 Dashboard WebSocket backpressure 策略未明示

- **问题文件**: `crates/dashboard/src/lib.rs`
- **位置**: `handle_ws` 推送路径
- **具体问题**: 慢客户端时 backpressure 策略未明示(unbounded / bounded / latest-wins)
- **影响文件**:
  - `crates/dashboard/src/lib.rs`
  - `crates/events/src/bus.rs`(事件源)
- **引发问题**: 内存爆炸或丢事件未明示,observability 缺失

### 7.4 `Backbone::record` 持有 write lock 阻塞 reader(同 8.4)

### 7.5 `SessionManager::sessions` 是 `RwLock<HashMap>` 但 hot path

- **问题文件**: `crates/session/src/manager.rs`
- **位置**: `RwLock<HashMap<SessionId, SessionHandle>>`
- **具体问题**: dashboard 高频拉取 session 列表时与 record / create 路径争锁
- **影响文件**:
  - `crates/session/src/manager.rs`
  - `crates/dashboard/src/lib.rs`
  - `crates/mcp/src/server.rs`
- **引发问题**: 读写竞争激烈时延迟上升,需评估是否用 DashMap / 分片锁

### 7.6 `Snapshot::Display` 每次重新格式化

- **问题文件**: `crates/core/src/snapshot.rs`
- **位置**: `Snapshot` 的 `fmt::Display`
- **具体问题**: 每次 `Display` 调用重新构造 `String`
- **影响文件**:
  - `crates/core/src/snapshot.rs`
  - `crates/mcp/src/server.rs`(tool 返回文本)
  - `crates/cli/src/open.rs`(println!)
- **引发问题**: 单次操作,非热路径;但同一 snapshot 被多次序列化时重复分配

---

## 8. Resilience

### 8.1 Dashboard `tokio::spawn` 失败无告警

- **问题文件**: `crates/cli/src/serve.rs`
- **位置**: 第 52–56 行 dashboard spawn 块
- **具体问题**: spawn 内只 `eprintln!`,主 server 不知 dashboard 已死
- **影响文件**:
  - `crates/cli/src/serve.rs`
  - `crates/dashboard/src/lib.rs`
  - `crates/events/src/backbone.rs`(本应触发事件)
- **引发问题**: 端口冲突等场景静默失败;MCP 仍正常服务但 dashboard 不可用,用户无感知

### 8.2 serve 模式无 graceful shutdown

- **问题文件**: `crates/cli/src/serve.rs`、`crates/mcp/src/server.rs`
- **位置**: `RutterMcp::serve` 退出路径
- **具体问题**: stdin EOF / SIGTERM 后,无显式 `engine.shutdown()`
- **影响文件**:
  - `crates/cli/src/serve.rs`
  - `crates/mcp/src/server.rs`
  - `crates/engine/src/engine.rs::Engine::shutdown`
- **引发问题**: chromiumoxide 子进程可能孤儿化;manager 持有的 SessionContext 未显式关闭

### 8.3 `Backbone::publish` 满载丢帧无 metric

- **问题文件**: `crates/events/src/bus.rs`
- **位置**: `publish` 走 broadcast channel send
- **具体问题**: broadcast 满时 `send` 失败,当前实现仅"fire-and-forget"
- **影响文件**:
  - `crates/events/src/bus.rs`
  - `crates/dashboard/src/lib.rs`(消费端)
- **引发问题**: 订阅端看到 seq 不连续无 metric,无法区分丢帧与 bug

### 8.4 `Backbone::record` write lock 长持

- **问题文件**: `crates/events/src/backbone.rs`
- **位置**: `record` 方法持有 `rings.write()` 整个 body
- **具体问题**: writer 等待时阻塞所有 reader(replay、snapshot)
- **影响文件**:
  - `crates/events/src/backbone.rs`
  - `crates/events/src/ring.rs`
  - `crates/dashboard/src/lib.rs`
- **引发问题**: dashboard 高频拉取时影响 record 吞吐

### 8.5 MCP tool 无 per-session rate limit

- **问题文件**: `crates/mcp/src/server.rs`
- **位置**: `RutterMcp` 各 `#[tool]` 方法
- **具体问题**: 无 QPS / RPS cap
- **影响文件**:
  - `crates/mcp/src/server.rs`
  - `crates/session/src/session.rs`(下游执行)
- **引发问题**: agent 循环调用或恶意 prompt 致 OOM / CPU 占满;screenshot / snapshot 尤其易触发

### 8.6 `Session` drop 未触发 cleanup

- **问题文件**: `crates/session/src/session.rs`、`crates/session/src/manager.rs`
- **位置**: `Session` 与 `SessionHandle` 的 `Drop`
- **具体问题**: `Drop` 实现无显式 `context.close_page` 等清理(同 1.6)
- **影响文件**:
  - `crates/session/src/session.rs`
  - `crates/session/src/manager.rs`
- **引发问题**: 异常 drop 路径(panic / 中断)下 context 未释放;依赖后端 drop 语义不可靠

---

## 9. Maintainability

### 9.1 `session/session.rs` 1000+ 行"上帝对象"

- **问题文件**: `crates/session/src/session.rs`
- **位置**: `Session` struct 的所有方法(execute、tabs、cookie、wait_for、screencast、close_*、set_cookies、select_tab)
- **具体问题**: 多个职责堆在一个 struct
- **影响文件**:
  - `crates/session/src/session.rs`
  - `crates/mcp/src/server.rs`(调用 Session 各方法)
- **引发问题**: 修改一个方法需理解整个文件;认知负担重

### 9.2 `SessionManager::new` 6 参数全 Arc

- **问题文件**: `crates/session/src/manager.rs`
- **位置**: `new` 方法签名
- **具体问题**: 6 个参数全是 `Arc<T>`,顺序任意,易写错
- **影响文件**:
  - `crates/session/src/manager.rs::new`
  - `crates/cli/src/serve.rs::run`
- **引发问题**: 调用方易传错位,无编译期强约束

### 9.3 observe 与 core 双重截断(同 5.5)

### 9.4 `Backbone` 公开并发原语(同 5.6)

### 9.5 `dashboard/lib.rs` 单文件 1100+ 行(同 5.3)

### 9.6 `engine-cdp` 5 个模块,职责仍有混合(同 5.4)

---

## 10. Naming

### 10.1 `EntryMode` / `LaunchMode` 命名冲突

- **问题文件**: `crates/cli/src/entry.rs`、`crates/engine/src/config.rs`
- **位置**: `EntryMode` enum vs `LaunchMode` enum 同时存在
- **具体问题**: 两个 Mode 同时存在,语义不同但命名相似
- **影响文件**:
  - `crates/cli/src/entry.rs`
  - `crates/engine/src/config.rs`
  - `crates/cli/src/main.rs`
  - `crates/cli/src/browse.rs`
- **引发问题**: 阅读时混淆"命令模式"与"启动模式"

### 10.2 `Snapshot::truncated` bool 无原因分类

- **问题文件**: `crates/core/src/snapshot.rs`
- **位置**: `truncated: bool`
- **具体问题**: budget/hostile/safety 截断无法区分
- **影响文件**:
  - `crates/core/src/snapshot.rs`
  - `crates/observe/src/builder.rs`
- **引发问题**: 诊断时无法区分截断来源;SPEC §6 列举多种截断原因,但 bool 丢信息

### 10.3 `Origin` 参数语义模糊(同 4.7)

### 10.4 `page` / `tab` 术语混用

- **问题文件**: `crates/engine/src/context.rs`、`crates/session/src/session.rs`、`docs/TOOL_SPEC.md`
- **位置**: `PageHandle` (engine 层) vs `TabInfo` (session 层)
- **具体问题**: `page == tab` 关系无顶部说明
- **影响文件**:
  - 全部涉及 page/tab 的文件
  - `docs/TOOL_SPEC.md §4`(tools 命名)
- **引发问题**: 阅读与跨层引用时困惑

### 10.5 `engine-cdp` 命名与 `CdpLauncher::new(executable, backend)` 冗余

- **问题文件**: `crates/engine-cdp/src/launch.rs`
- **位置**: `CdpLauncher::new`
- **具体问题**: `executable` 与 `backend` 冗余(任何 chrome 系 binary 默认就是 chromiumoxide)
- **影响文件**:
  - `crates/engine-cdp/src/launch.rs`
  - `crates/cli/src/launcher.rs`(调用)
- **引发问题**: 调用方需为同一信息传两次

---

## 11. Documentation

### 11.1 关键函数缺 `@param` / `@returns`

- **问题文件**: `crates/engine-cdp/src/launch.rs`、`crates/engine-cdp/src/context.rs`、`crates/engine/src/page.rs`
- **位置**:
  - `CdpLauncher::with_extra_args`(launch.rs)
  - `CdpContext::cookies`(context.rs)
  - `ScreencastStream::next_frame`(page.rs)
- **具体问题**: 函数 doc 缺 `@param` / `@returns` / `@throws` / 错误条件说明
- **影响文件**:
  - `crates/engine-cdp/src/launch.rs`
  - `crates/engine-cdp/src/context.rs`
  - `crates/engine/src/page.rs`
- **引发问题**: 维护者难快速理解签名语义;调用方需读实现

### 11.2 README 与 BLUEPRINT 同步

- **问题文件**: `README.md`、`docs/BLUEPRINT.md`
- **位置**: 两文档顶层
- **具体问题**: README 内容比 BLUEPRINT 旧,未指明真值源
- **影响文件**:
  - `README.md`
  - `docs/BLUEPRINT.md`
- **引发问题**: 新 contributor 拿到错误信号;蓝图权威性弱化

### 11.3 `replay` 并发 seq 间隙无文档

- **问题文件**: `crates/events/src/backbone.rs`
- **位置**: `replay` 方法 doc
- **具体问题**: 未注明并发下 seq 间隙行为
- **影响文件**:
  - `crates/events/src/backbone.rs`
  - `crates/dashboard/src/lib.rs`
- **引发问题**: 订阅端误判丢帧(同 1.4)

### 11.4 `Backoff::delay(0)` 语义未文档化(同 2.3)

### 11.5 `key_params` v1 限制 doc 不一致

- **问题文件**: `crates/engine-cdp/src/error.rs`
- **位置**: `key_params` doc vs TOOL_SPEC §4 press_key v1 limitation
- **具体问题**: 两处对"无 virtual key codes"的描述位置不同
- **影响文件**:
  - `crates/engine-cdp/src/error.rs`
  - `docs/TOOL_SPEC.md §4 press_key`
- **引发问题**: 维护者不知从哪个文档看限制

---

## 12. Testing

### 12.1 `SessionManager` 几乎无单元测试

- **问题文件**: `crates/session/src/manager.rs`
- **位置**: `#[cfg(test)] mod tests`
- **具体问题**: 仅 3–5 行基础用例,缺并发、关闭、lazy 失败等覆盖
- **影响文件**:
  - `crates/session/src/manager.rs`
- **引发问题**: 核心 orchestration 缺回归保障

### 12.2 `ApprovalBroker` 测试不足(同 1.5 副作用)

- **问题文件**: `crates/policy/src/broker.rs`
- **位置**: `#[cfg(test)] mod tests`
- **具体问题**: 缺并发、超时未决清理、二次 decide panic、unbounded 上限等用例
- **影响文件**:
  - `crates/policy/src/broker.rs`
- **引发问题**: broker 行为无回归保障

### 12.3 e2e 全部 `#[ignore]`,默认 `cargo test` 不验证后端

- **问题文件**: `crates/cli/tests/*`、`crates/engine-cdp/tests/*`
- **位置**: 全部 `#[ignore = "requires a downloaded engine binary"]`
- **具体问题**: 默认 `cargo test` 不跑这些
- **影响文件**:
  - `crates/cli/tests/http_e2e.rs`
  - `crates/cli/tests/mcp_e2e.rs`
  - `crates/cli/tests/approval_e2e.rs`
  - `crates/engine-cdp/tests/integration.rs`
  - `crates/engine-cdp/tests/screencast.rs`
- **引发问题**: PR 默认不验证后端;CI 需显式 `cargo test -- --ignored` 才跑

### 12.4 1.1 截图速率限制 bug 无回归测试

- **问题文件**: `crates/engine-cdp/src/page.rs`
- **位置**: `capture_screenshot` 缺对应测试
- **具体问题**: 无 test 覆盖长空闲场景下应不等待
- **影响文件**:
  - `crates/engine-cdp/src/page.rs`
  - `crates/engine/src/config.rs::ContextConfig`(契约侧)
- **引发问题**: bug 修复后无防回归

### 12.5 `rutter-observe` 无 property / fuzz

- **问题文件**: `crates/observe/`
- **位置**: 整 crate
- **具体问题**: hostile input 承诺(SNAPSHOT_SPEC §3 never panics)无 proptest / cargo-fuzz 验证
- **影响文件**:
  - `crates/observe/src/builder.rs`
  - `crates/observe/src/response.rs`
  - `crates/observe/src/assets/serializer.js`
- **引发问题**: SPEC 承诺无验证

### 12.6 `engine-cdp` 集成测试边界不足

- **问题文件**: `crates/engine-cdp/tests/integration.rs`
- **位置**: 整文件
- **具体问题**: 缺启动失败(bad executable)、并发 create_context、cookie 跨 context 隔离、超时边界等
- **影响文件**:
  - `crates/engine-cdp/tests/integration.rs`
  - `crates/engine-cdp/src/launch.rs`
- **引发问题**: 边界行为无验证

### 12.7 MCP 层无 mock client test

- **问题文件**: `crates/mcp/src/server.rs`、`crates/cli/tests/mcp_e2e.rs`
- **位置**: `RutterMcp` tool 路由、`isError` 标签、hint 行格式
- **具体问题**: rmcp mock client 测试缺失
- **影响文件**:
  - `crates/mcp/src/server.rs`
  - `crates/cli/tests/mcp_e2e.rs`
- **引发问题**: 契约层测试每次需真 chromiumoxide

### 12.8 `snapshot_from_response` 缺 hostile input 测试

- **问题文件**: `crates/observe/src/builder.rs`、`crates/observe/src/response.rs`
- **位置**: 反序列化路径
- **具体问题**: 无 `__proto__`、巨大 JSON、字段类型错乱、缺字段等场景测试
- **影响文件**:
  - `crates/observe/src/builder.rs`
  - `crates/observe/src/response.rs`
- **引发问题**: SNAPSHOT_SPEC §2 §3 承诺无验证

---

## 跨方向汇总(按严重度)

### 必须修 (High)

| # | 方向 | 文件:位置 | 一句话 |
|---|------|----------|--------|
| 1.3 | Correctness | `session/error.rs:10-28` | `EngineError` 在 MCP 边界未映射为 `ActionError`,违反 SPEC |
| 3.1 | Security | `cli/main.rs:60` + `cli/serve.rs` | `--http` 可能绑定 0.0.0.0 且无认证,MCP 工具面暴露 |
| 3.2 | Security | `core/cookie.rs` + `engine-cdp/context.rs` | `Cookie::domain` 无 SSRF / 内网校验 |

### 应该修 (Medium)

| # | 方向 | 文件:位置 | 一句话 |
|---|------|----------|--------|
| 1.1 | Correctness | `engine-cdp/page.rs:204-220` | 截图速率限制写入未来时刻,语义错位 |
| 1.4 | Correctness | `events/backbone.rs::replay` | 并发 record 下 seq 间隙 |
| 1.5 | Correctness | `policy/broker.rs` | unbounded mpsc + 双重 decide panic |
| 1.6 | Correctness | `session/manager.rs::close_session` | 未显式释放 context 资源 |
| 1.7 | Correctness | `core/cookie.rs::Cookie.expires` | f64 接受 NaN / Infinity |
| 3.3 | Security | `policy/rules.rs::evaluate` | Navigate 评估 url 取值未文档化 |
| 3.4 | Security | `observe/builder.rs` | hostile JSON 防御策略未集中文档化 |
| 3.6 | Security | `cli/open.rs` + `dashboard/lib.rs` | stdout / snapshot 可能泄漏敏感数据 |
| 4.6 | API Contract | `mcp/server.rs` | rmcp 版本漂移未 pin |
| 8.1 | Resilience | `cli/serve.rs:52-56` | dashboard 死亡无告警 |
| 8.2 | Resilience | `cli/serve.rs` + `mcp/server.rs` | serve 模式无 graceful shutdown |
| 8.5 | Resilience | `mcp/server.rs` | MCP tool 无 per-session rate limit |
| 9.1 | Maintainability | `session/session.rs` | 1000+ 行"上帝对象" |
| 9.2 | Maintainability | `session/manager.rs::new` | 6 参数全 Arc,顺序易错 |
| 12.1 | Testing | `session/manager.rs` | SessionManager 几乎无单元测试 |
| 12.3 | Testing | `cli/tests/*` + `engine-cdp/tests/*` | e2e 全部 `#[ignore]`,默认 cargo test 不验证后端 |

### 可以修 (Low / Suggest)

包含 1.8、1.9、2.x 剩余、3.5、3.7、4.2、4.3、4.4、4.5、4.7、5.x、6.3、6.4、7.x、8.3、8.4、8.6、9.3–9.6、10.x、11.x、12.2、12.4–12.8

---

## 总结

- **High 严重度问题**:3 个,集中在 SPEC 契约违反 (1.3) 与 MCP 暴露 (3.1)、Cookie 注入 (3.2)
- **Medium 严重度问题**:16 个,覆盖 correctness、security、resilience、maintainability、testing 多个方向
- **Low / Suggest**:约 30 项,多为命名、文档、解耦、依赖管理

**本次审查未执行**:`cargo test`、`cargo clippy`、`cargo audit`、`cargo deny check`、`cargo build` —— 完全未读,避免在"只读"约束下做额外修改。