# crates/ — workspace 成员

[English](README.md) | 中文

十个 crate，一条依赖方向：一切都从 `core` 向外流动，且只有 `cli`（组
合根）能看到全部成员。这些接缝的存在让每个 crate 只有一个变更理由
（docs/architecture.md）；`cli` 在启动时把它们组装在一起。

| Crate | 职责 | 依赖（workspace 内） |
|---|---|---|
| [`core/`](core/README.md) | 领域词汇：Action（动作）、Snapshot（快照）、Reference（引用）、cookies、错误 | — |
| [`events/`](events/README.md) | 类型化 Event（事件）骨干：bus、每会话 ring buffer（环形缓冲）、重放 | core |
| [`policy/`](policy/README.md) | Verdict（判决）规则（TOML）与审批 broker | core |
| [`observe/`](observe/README.md) | 页内脚本与快照构建器（纯变换，无 I/O） | core |
| [`engine/`](engine/README.md) | Engine（引擎）/Page（页面）trait、Chrome-for-Testing 下载、supervisor | core |
| [`engine-cdp/`](engine-cdp/README.md) | 唯一讲 CDP 的 crate（chromiumoxide）；公开面只有一个 launcher | core, engine |
| [`session/`](session/README.md) | 编排：Session（会话）、动作、自动等待、存储状态、恢复 | core, engine, events, observe, policy |
| [`mcp/`](mcp/README.md) | 基于 rmcp 的 MCP 工具面（stdio + streamable HTTP） | core, engine, session |
| [`dashboard/`](dashboard/README.md) | 本地监督面板（事件、审批、screencast（屏幕流）） | core, engine, events, policy, session |
| [`cli/`](cli/README.md) | 二进制入口模式：browse、serve、open；组装一切 | 以上全部 |

新增 crate：在根 `Cargo.toml` 的 members 列表里注册，并在那里声明
workspace 依赖（不允许散落的版本号），同时保持上述方向——下层
crate 绝不 import 上层 crate。

测试：每个 crate 都有 `tests/` 目录来演练其公开 API；私有路径的单元
测试放在 `src/` 里紧挨代码。横跨多个 crate 的测试放在 workspace 根
的 `tests/` 包。
