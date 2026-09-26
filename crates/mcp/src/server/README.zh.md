# server/ — rmcp 宿主

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `mod.rs` | `RutterMcp`：Session（会话）生命周期（OnceCell + 关闭后 fail-fast）、19 个工具、结果/协议错误映射 |
| `params.rs` | 纯输入类型：参数结构体、`Direction`、`SameSiteInput`、`CookieInput` → `Cookie` |
| `tests.rs` | stub 引擎上的生命周期测试、截断标记映射（私有路径） |

参数映射测试在本 crate 的 `tests/tool_params.rs`。

`#[tool_router]` impl 块必须把所有 `#[tool]` 方法放在一处（rmcp 宏
约束）。工具名与参数名是 docs/tool-catalog.md 契约——不得重命名。
