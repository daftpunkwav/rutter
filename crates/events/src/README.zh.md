# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `event.rs` | 事件词汇表（serde `tag = "type"`、snake_case） |
| `envelope.rs` | `Envelope`：seq + timestamp + session + event，即消费者看到的形态 |
| `bus.rs` | tokio broadcast 扇出；容量限定慢订阅者的丢失量 |
| `ring.rs` | 有界的每会话 ring buffer（环形缓冲），最旧的先淘汰，`history()` 供重放 |
| `backbone.rs` | `Backbone`：publish + subscribe + replay，他人唯一接触的类型 |

发布绝不阻塞；`Lagged` 订阅者通过 `replay` 重同步——dashboard 的两
条代码路径都依赖这一配对。
