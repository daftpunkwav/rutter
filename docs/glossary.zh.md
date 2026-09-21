# 术语表

[English](glossary.md) | 中文

规范的领域词汇。一个概念一个词——代码、文档、UI 使用这些准确的
术语，且不存在同义词。
[`crates/core/src/ids.rs`](../crates/core/src/ids.rs) 承载 Context、
Page、Session 三个术语的类型层版本。

| 术语 | 含义 |
|---|---|
| Engine | rutter 管理的浏览器进程（Chromium headless shell） |
| Context | 引擎内隔离的 cookie/storage 单元 |
| Page | context 内的一个标签页/target |
| Session | 一个 MCP client 的工作区：它的 context、页面与状态 |
| Snapshot | 页面的 token 预算内可访问性树视图，带引用 |
| Reference | 元素的稳定句柄，可跨快照使用 |
| Action | agent 请求的类型化操作（click、type、…） |
| Verdict | 策略决策：`Allow` / `Deny` / `RequireApproval` |
| Event | 发布到事件主干的结构化事实 |
| Origin | 动作发起者的归因：`Agent` 或 `Human` |
| Browse mode | 入口模式：人类直接操作的有头引擎 session |

## 标识符

标识符是不透明字符串
（[`ids.rs`](../crates/core/src/ids.rs)）：调用方不得从其内容解析
或构造含义。拥有对象生命周期的层负责铸造它的标识符。

| 类型 | 形态 | 铸造方 |
|---|---|---|
| `SessionId` | stdio 上为 `stdio-<pid>`；streamable HTTP 上为 `http-<pid>-<n>` | MCP 层在连接开始时 |
| `ContextId` | 引擎分配 | `rutter-engine-cdp` |
| `PageId` | 引擎分配 | `rutter-engine-cdp` |
| `ApprovalId` | `apr-<n>`，每个 broker 独立序号 | [`ApprovalBroker`](../crates/policy/src/broker.rs) |
| `Reference` | `e<n>`，每页计数器，导航时重置 | 页内序列化器（[快照格式 §4](snapshot-format.zh.md#4-引用铸造v1)） |

## 序列化约定

在网络上或事件载荷中旅行的类型共享一套约定，消费者因此能用同一种
方式命名一切（[`action.rs`](../crates/core/src/action.rs)、
[`event.rs`](../crates/events/src/event.rs)、
[`error.rs`](../crates/core/src/error.rs)）：

- 标签枚举以 `"type"` 标签和 `snake_case` 名称序列化：`Event`
  （`"action_failed"`）、`Action`（`"click"`）与 `ActionError`
  （`"reference_expired"`）。无标签枚举序列化为裸 `snake_case`
  字符串：`Verdict`（`"allow"`）、`Origin`（`"agent"`）与
  `ScrollDirection`（`"up"`）。
- 时间戳是 RFC 3339 UTC 字符串；不存在依赖 locale 的格式。
- 一切以 UTF-8 JSON 旅行。
