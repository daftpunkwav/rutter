# src/ — 文件地图

[English](README.md) | 中文

| Item | 职责 |
|---|---|
| `lib.rs` | 导出 `RutterMcp`；crate 级边界声明 |
| `server/` | rmcp server：工具、参数、结果映射（见其 README） |
| `http.rs` | streamable HTTP transport（`/mcp`）；非环回绑定时告警 |

transport 都很薄：stdio 在 `cli`（rmcp 的 `serve`），HTTP 在这里。
任何开始在本 crate 里校验业务规则的东西，都是该规则属于 `session`
的信号。
