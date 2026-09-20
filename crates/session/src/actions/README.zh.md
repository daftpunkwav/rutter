# actions/ — 动作执行

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `mod.rs` | `Executor::run`（一个 Action（动作） → 自动等待 → dispatch → settle → 快照）与 `PageOps`（url/snapshot/wait_for）；自动等待预算用的 `phase_rank` |
| `tests.rs` | 无浏览器测试：阶段超时、预算重启、输入坐标 |

自动等待阶段按序运行（visible → stable → enabled），各自有独立预
算，只有阶段前进时才重启（TOOL_SPEC §3）——闪烁的页面无法把等待无
限拉长。policy 与审批是调用者（`Session::execute`）的职责，绝不在
这里运行。
