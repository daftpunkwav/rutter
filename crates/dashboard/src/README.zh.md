# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `lib.rs` | `DashboardServer`：绑定、路由、静态处理器、内嵌资源、HTTP 决定端点 |
| `auth.rs` | 端点门禁：环回 `Host` 检查、token（query/HttpOnly cookie）、常数时间比较 |
| `ws.rs` | WebSocket 循环：重放 → 实时、Lagged 重同步、screencast 转发、决定消息 |

每个新路由都必须经过 `auth::access_allowed`——这道门存在的意义就是
路由不能带着一半的安全检查上线。
