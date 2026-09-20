# 事件

[English](events.md) | 中文

[`rutter-events`](../crates/events/src/backbone.rs) 的类型化事件
主干：编排层发布的结构化事实，以及消费者所依赖的投递语义。

## 1. 信封

每个事实以[`Envelope`](../crates/events/src/envelope.rs)旅行：

```json
{
  "seq": 41,
  "recorded_at": "2026-09-20T10:00:00.123456Z",
  "session": "stdio-1234",
  "event": { "type": "page_navigated", "page": "ctx-1:page-0",
             "url": "https://example.com/" }
}
```

- `seq` 是进程级全局单调计数器——消费者以它排序与去重。编号、写入
  ring、总线扇出在同一次加锁内完成，所以 ring 顺序与实流顺序都等于
  分配顺序，按水位去重不会丢掉未见过的事件。
- `recorded_at` 是 RFC 3339 UTC 字符串
  （[术语表](glossary.zh.md#序列化约定)）；时钟故障降级为 epoch
  字符串，而不是让发布失败。
- `session` 指明事实所属的工作区。

## 2. 事件词汇

完整词汇位于
[`crates/events/src/event.rs`](../crates/events/src/event.rs)；
每个变体都由 session 或 manager 层按下表所述产生。

| 事件 | 载荷 | 产生时机 |
|---|---|---|
| `SessionStarted` | — | 一个 session 工作区诞生 |
| `SessionClosed` | — | client 关闭了一个 session |
| `EngineStarted` | `backend`, `version` | 引擎进程首次启动 |
| `EngineRestarted` | — | supervisor 替换了死亡的引擎；重启前的状态已不存在 |
| `PageOpened` / `PageClosed` | `page` | 页面在 session 的 context 中打开/关闭 |
| `PageNavigated` | `page`, `url` | 活动页面导航（`url` 是重定向后的生效值） |
| `ActionRequested` | `page`, `origin`, `action` | 动作被请求，尚未开始执行 |
| `ActionCompleted` | `page`, `origin`, `action` | 动作成功完成 |
| `ActionFailed` | `page`, `origin`, `action`, `error` | 动作失败；`error` 携带[错误分类](tool-catalog.zh.md#2-结果约定) |
| `ApprovalRequested` | `request_id`, `page`, `brief` | [策略](policy.zh.md#4-审批)将动作搁置等待人工决定 |
| `ApprovalResolved` | `request_id`, `granted` | 人类已答复，或窗口超时 |

动作与错误以[序列化词汇](glossary.zh.md#序列化约定)（`"type"`
标签、snake_case）内嵌，消费者命名一个动作或失败与命名事件的方式
一致。

Screencast 帧**不是**事件。它们以二进制 WebSocket 帧旅行，有自己
的 latest-wins 背压规则（[仪表盘](dashboard.zh.md#5-screencast)）。

## 3. 投递语义

[`Backbone::publish`](../crates/events/src/backbone.rs) 是
fire-and-forget：不等待消费者、永不失败。两条通道，两种丢失规则：

- **实时 bus。** tokio broadcast channel（容量 1024）。无订阅者时
  发布也成功；落后太多的订阅者会读到 `Lagged` 错误，必须经 replay
  重新同步。
- **Per-session ring。** 每 session 一个有界 ring（容量 1000），
  为迟到者保留每个语义事件；`replay(session)` 按最旧优先返回
  历史。session 关闭时 ring 被丢弃，因此反复进出 session id 的
  服务器不会无界增长该映射。

丢失契约：**语义事件存活**（经 ring）；跟不上的实时订阅者在
replay 之前会丢信封。仪表盘演示了重同步模式
（[仪表盘 §3](dashboard.zh.md#3-websocket-协议)）：先订阅、再截取
replay 快照、按序列水位去重，并用 ring 填补任何 `Lagged` 缺口。
