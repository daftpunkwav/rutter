# src/ — 文件地图

[English](README.md) | 中文

所有实现模块都刻意保持私有；crate 边界只有 `launch.rs` 的
`CdpLauncher`。

| 文件 | 职责 |
|---|---|
| `launch.rs` | `CdpLauncher`：接收已解析的二进制、启动 chromiumoxide、包装为 `Engine` |
| `engine.rs` | `CdpEngine`：chromiumoxide 浏览器之上的浏览器 Context（上下文） |
| `context.rs` | `CdpContext`：target、cookies、幂等的上下文关闭 |
| `page.rs` | `CdpPage`：evaluate、输入、截图、带 deadline 的 screencast |
| `error.rs` | `with_deadline` 包装器 + 把 CDP 错误折入 `EngineError` |

绝不让 chromiumoxide 类型出现在 `pub` 签名上——`rutter-engine` 的
trait 是唯一出口（docs/architecture.md）。每个 CDP 调用都带 deadline；任何
调用都不许裸挂。
