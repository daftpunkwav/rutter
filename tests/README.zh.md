# tests/ — 跨 crate 集成

[English](README.md) | 中文

横跨 crate 边界的验收测试：每个测试都拉起构建好的 `rutter` 二进制
并以 MCP 客户端身份驱动它，于是整条栈（client → transport → mcp →
session → engine-cdp）一起运行。默认 `#[ignore]`；用 `--ignored` 运
行（需要缓存中已有引擎二进制）。

| 目标 | 覆盖内容 |
|---|---|
| `mcp_e2e.rs` | stdio 上的完整任务流：navigate + snapshot 读取、改变页面的 click、表单 type/select 与 screenshot、wait_for、close_session fail-fast |
| `approval_e2e.rs` | 真实二进制上的 policy 门禁：驻留、grant/deny、超时 |
| `http_e2e.rs` | streamable HTTP transport 端到端 |

二进制定位：设置了 `CARGO_BIN_EXE_rutter` 时用它，否则用
`RUTTER_BIN` 覆盖，再否则用 workspace 的 `target/debug` 布局。
