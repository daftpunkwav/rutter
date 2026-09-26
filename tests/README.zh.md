# tests/ — 跨 crate 集成

[English](README.md) | 中文

横跨 crate 边界的验收测试：每个测试都拉起构建好的 `rutter` 二进制
——MCP 套件以 MCP 客户端身份驱动它，`open_e2e` 演练一次性诊断——
于是整条栈（client → transport → mcp → session → engine-cdp）一起
运行。需要引擎的测试默认 `#[ignore]`；用 `--ignored` 运行（需要缓
存中已有引擎二进制）。

| 目标 | 覆盖内容 |
|---|---|
| `mcp_e2e.rs` | stdio 上的完整任务流：navigate + snapshot/markdown 读取、改变页面的 click、表单 type/select 与 screenshot、wait_for、close_session fail-fast |
| `approval_e2e.rs` | 真实二进制上的 policy 门禁：驻留、grant/deny、超时 |
| `http_e2e.rs` | streamable HTTP transport 端到端 |
| `open_e2e.rs` | `rutter open` 一次性诊断：stdout 恰好输出一个 snapshot；缺失引擎时以失败退出并携带 hint（该情形无需引擎） |
| `read_e2e.rs` | `rutter read` 一次性抓取：stdout 输出页面的 markdown 文档 |

二进制定位：设置了 `CARGO_BIN_EXE_rutter` 时用它，否则用
`RUTTER_BIN` 覆盖，再否则用 workspace 的 `target/debug` 布局。
