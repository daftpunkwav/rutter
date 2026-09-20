# src/ — 文件地图

[English](README.md) | 中文

| Item | 职责 |
|---|---|
| `engine.rs` | `Engine` trait：创建 Context（上下文）、descriptor、健康检查、关闭 |
| `context.rs` | `ContextHandle` trait：Page（页面）、cookies、幂等的 `close()` |
| `page.rs` | `PageHandle` trait、截图、screencast 流类型 |
| `descriptor.rs` | `EngineDescriptor`、后端、capabilities（`Display` = 事件名） |
| `health.rs` | `HealthReport` |
| `input.rs` | 经 `PageHandle` 派发的协议中立输入 Event（事件） |
| `config.rs` | `LaunchMode`、`ContextConfig`（页面上限、截图间隔） |
| `error.rs` | `EngineError` + 每个变体的提示；session 把它们映射为 `ActionError` |
| `backoff.rs` | 有上限的指数退避 |
| `download/` | Chrome-for-Testing 解析与安装（见其 README） |
| `supervisor/` | 心跳、重启、熔断器（见其 README） |

`session` 只通过这些 trait 驱动引擎；在后端 crate 里实现它们
（本 crate 的 supervisor 是 crate 内唯一的调用方）。
