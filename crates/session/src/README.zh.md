# src/ — 文件地图

[English](README.md) | 中文

| Item | 职责 |
|---|---|
| `lib.rs` | 公开导出；本 crate 是 engine/observe/policy/events 的交汇点 |
| `manager.rs` | `SessionManager`：惰性启动引擎、会话上限、恢复任务 |
| `session/` | `Session`：Page（页面）、execute、screencast、close（见其 README） |
| `actions/` | `Executor`/`PageOps`：每次调用一个 Action（动作）、自动等待（见其 README） |
| `resolve.rs` | 经 observe 脚本做 Reference（引用）解析 |
| `storage.rs` | 存储状态（cookies + localStorage），原子且仅限属主的持久化 |
| `wait.rs` | `poll_until`，带 600 s 预算钳制（时钟溢出防护） |
| `config.rs` | `SessionConfig` 默认值由 docs/tool-catalog.md §3 钉死（`wait_for` 预算默认值在 MCP 工具层，§4） |
| `error.rs` | `SessionError`，衔接引擎错误与动作错误 |
| `mock.rs` | 仅供测试的引擎替身（`#[cfg(test)]`），用于无浏览器测试 |

新的会话行为必须走 `Session::execute` 的门 → 动作 → 事件 → 刷新序
列；绕过它就会丢失 policy、事件或持久化。
