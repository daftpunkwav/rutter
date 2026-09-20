# rutter-events/ — 事件骨干

[English](README.md) | 中文

带全局序号与 RFC 3339 时间戳的类型化语义 Event（事件），在 tokio
broadcast bus 上扇出，并保存在有界的每会话 ring buffer（环形缓冲）
里供重放。

## 边界

依赖 `core`，外加 `policy` 的一个字段——审批事件携带的简报只有
policy 能构造（docs/policy.zh.md）。发布是 fire-and-forget：不等待
消费者、永不失败（docs/events.md）。Backpressure（背压）：bus
可以丢弃慢订阅者的 envelope（它们从环形缓冲重同步），但语义事件绝不
从环形缓冲丢弃；screencast 帧根本不是事件——它们以二进制 dashboard
帧的形式流动。

## 消费者

- `session` 发布会话做的一切。
- `dashboard` 在连接时重放历史，然后跟随实时流。
