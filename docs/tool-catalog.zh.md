# MCP 工具目录

[English](tool-catalog.md) | 中文

`rutter-mcp` 工具面的契约：传输、结果约定、auto-wait 语义，以及
每个工具及其参数。`rutter-mcp` 精确实现本文；单元与端到端测试
将其钉住，[`crates/core/src/action.rs`](../crates/core/src/action.rs)
承载载荷词汇。

## 1. 传输与 session

- 传输：stdio（`rutter serve` 的默认）或 streamable HTTP（
  `rutter serve --http ADDR`）。仪表盘通过 `serve --dashboard PORT`
  挂载，规则集通过 `serve --policy FILE` 加载。非 loopback 的
  `--http` 绑定必须用 `serve --allow-remote` 确认——该传输没有
  认证。
- 一条 MCP client 连接是一个 Session。`SessionId` 在连接开始时
  铸造（[术语表](glossary.zh.md#标识符)），通过 `SessionStarted`
  事件报告；引用 session 的错误载荷中也会出现。
- 引擎惰性启动：`serve` 启动时没有引擎；第一个需要页面的工具调用
  会启动它（受监管，[引擎监管](engine-supervision.zh.md)）并创建
  session 的 context（带上限配置）。MCP 服务器自身的启动不执行
  任何 I/O。
- 每个 session 恰好一个 context。活动页面是所有页面级工具的
  目标；不存在页面时 `navigate` 打开第一个页面。
- 传输加固：streamable HTTP 传输拒绝浏览器发起的请求（空的
  `Origin` 允许列表），并按 loopback 允许列表校验 `Host`
  （防 DNS rebinding）。

## 2. 结果约定

- 工具返回 MCP content block；文本结果使用一个 `text` block。
- **动作失败是结果，不是协议错误。** 失败的动作返回
  `isError: true`，其文本携带 `ActionError` 消息，第二行是可执行
  的提示（`hint: …`）。agent 将其作为数据读取。
- 协议级失败（未知 session、引擎死亡且熔断打开）是 JSON-RPC
  错误，code `-32000`，同样的消息加提示文本。
- 快照以[快照格式 §5](snapshot-format.zh.md#5-文本渲染yaml-风格)
  的 YAML 文本形式渲染，遵循 20 000 字符预算；`Snapshot::truncated`
  置位时文本以 `… truncated` 标记行结尾。
- 除非下文另有说明，每个变更类工具默认返回一份新快照。

完整的失败词汇是
[`ActionError`](../crates/core/src/error.rs)；每个变体都携带
agent 可执行的英文提示：

| 变体 | 含义 |
|---|---|
| `NavigationFailed { url, cause }` | 页面无法加载；`cause` 对传输失败分类（`TimedOut`、`ConnectionFailed`、`DnsFailed`、`TlsFailed`、`Aborted`、`Http { status }`） |
| `ReferenceExpired { reference }` | 引用指向的元素已不存在 |
| `NotInteractable { reference, reason }` | 元素存在但无法接收该动作 |
| `TimedOut { phase, elapsed }` | 某个 auto-wait 阶段未在时限内完成 |
| `ApprovalDenied { reference }` | 人工审批者拒绝了该动作 |
| `ApprovalTimedOut { waited }` | 窗口期内没有审批决定到达 |
| `EngineTerminated { session }` | 引擎死亡；rutter 正在恢复 |
| `Internal { detail }` | 动作边界处围堵了一个 bug——绝不是静默通过 |

## 3. Auto-wait 语义

每个携带 `reference` 的变更类工具在动作前运行三阶段 auto-wait；
各阶段轮询真实页面：

| 阶段 | 通过条件 | 预算（默认） |
|---|---|---|
| `visible` | 解析到的元素具有非空盒子 | 5 s |
| `stable` | 相隔约 80 ms 的两次采样盒子未变 | 5 s |
| `enabled` | 元素不是 `disabled` 且不是 `aria-disabled` | 5 s |

- `visible` 之前，解析器先把元素滚动进视口。
- 每个阶段的预算只在阶段推进时重置，因此闪烁的页面无法把等待
  无限拉长。
- 动作之后，执行器沉降 250 ms（让同 tick 的导航得以开始），然后
  截取新快照。
- 预算耗尽映射为 `ActionError::TimedOut`（带失败阶段）；引用无法
  解析映射为 `ActionError::ReferenceExpired`。

每个动作携带[origin](glossary.zh.md)：agent 发起的动作经过
[策略门](policy.zh.md#3-判定-url)；human 发起的动作绕过审批，
记录在同一条时间线上。

## 4. 工具

十九个工具。参数类型：`reference` 是快照句柄（`e17`）；
`direction` 是 `up|down|left|right` 之一；时长单位为毫秒。

### navigate
`{ url: string }` → snapshot。导航活动页面（需要时打开第一个
页面）。导航使用 context 的导航超时；失败 →
`ActionError::NavigationFailed`。

### back / forward / reload
`{}` → snapshot。活动页面的历史操作。

### snapshot
`{}` → snapshot。无 auto-wait；渲染当前合成 DOM。

### read
`{}` → text block，页面可读内容组成的 markdown 文档：标题行，然后是
标题、段落、列表、GFM 表格、代码围栏，以及带绝对 URL 的链接。无
auto-wait；提取规则与守卫见[读取格式](read-format.zh.md)。站点框架与
隐藏内容被省略；守卫裁剪了文档时，文本以 `… truncated` 标记结尾。

### screenshot
`{}` → `image` content block（PNG，base64）。遵守 context 的
截图频率上限；截取错误映射为 `ActionError::Internal`。

### click
`{ reference: string }` → snapshot。auto-wait，然后在解析盒中心
按下并释放鼠标。

### hover
`{ reference: string }` → snapshot。auto-wait，然后鼠标移动到
解析盒中心。

### type
`{ reference: string, text: string }` → snapshot。auto-wait，聚焦
元素，将 `text` 作为字面字符插入，然后返回快照。

### press_key
`{ key: string }` → snapshot。键名遵循引擎记法（`a`、`Enter`、
`Tab`）。v1 限制：按键事件只携带键名、没有虚拟键码；忽略无码
按键事件的站点是已记录的缺口。

### select_option
`{ reference: string, values: string[] }` → snapshot。auto-wait，
然后选中 `value` 在 `values` 中的选项并派发 `input`/`change`。
非 select 元素以 `NotInteractable` 失败。

### scroll
`{ direction: up|down|left|right, amount: number, reference?: string }`
→ snapshot。滚动页面（无 `reference`）或解析到的容器 `amount`
像素；`amount` ≤ 0 是 `invalid_params`。

### wait_for
`{ text: string, timeout_ms?: number }` → snapshot。轮询页面文本
直到 `text` 出现（默认预算 10 000 ms）；耗尽 →
`ActionError::TimedOut`。超过 600 000 ms 的请求预算被钳制到该
服务端上限（[`MAX_POLL_BUDGET`](../crates/session/src/wait.rs)）；
超时错误报告生效的预算。

### tabs_list
`{}` → 文本块，每页一行：`<page-id> <url>`；活动页面后缀
` (active)`。列出前先与引擎对账：非 rutter 打开的窗口——
`target=_blank`/`window.open` 弹窗、人类开的窗口——会以稳定的
`target:…` id 呈现并变为可选。这类 id 的 select/close 与普通页面
一致；属于会话自身 context 的窗口会被真正关闭，引擎自有表面
（app 窗口）只解除跟踪。

### tabs_select
`{ page_id: string }` → snapshot。未知 id → `invalid_params`。

### tabs_close
`{ page_id: string }` → 文本确认。未知 id → `invalid_params`。
关闭活动页面时提升第一个剩余页面；允许关闭最后一个页面——下一次
`navigate` 会打开新的。

### set_cookies
`{ cookies: [{ name, value, domain, path?, secure?, http_only?,
same_site? }] }` → 文本确认。cookie 应用到 session 的 context
（context 隔离，见[术语表](glossary.zh.md)）。`same_site` 是
`strict|lax|none`；跨重启的持久化经由 storage state 完成
（[sessions](sessions.zh.md#3-storage-state)）。

### close_session
`{}` → 文本确认。关闭 session 的页面与 context 并释放引擎引用。
该调用对连接是终态：同连接上后续的工具调用以 `invalid_params`
失败并指名已关闭的 session。引擎继续服务其他 session，在服务器
退出（client 断开或 Ctrl-C）时关闭，而不是在某个 session 关闭时。

## 5. 事件主干（不是工具）

[事件主干](events.zh.md)（类型化事件、bus、per-session ring、
replay）运行在服务器内部。MCP 没有通知或事件工具；仪表盘经
WebSocket 消费 replay。

## 6. 验收套件

三类 e2e 任务对真实引擎通过（`#[ignore]` 门控的集成测试，经
stdio 驱动构建的二进制；见[测试](testing.zh.md)）：

1. **读取**：`navigate` → `snapshot` 显示页面标题与可操作引用；
   `read` 返回页面的 markdown。
2. **交互**：在合成页面上 `click` 一个会变异 DOM 的按钮 → 返回的
   快照反映变化。
3. **表单**：`type` 输入框（及 `select_option`）→ 快照 value 字段
   反映输入；`screenshot` 返回非空 PNG 字节。
