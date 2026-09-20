# tests/ — 每 crate 功能测试

[English](README.md) | 中文

只通过本 crate 的**公开 API** 测试：manager → session 流、审批授予
路径与 wait_for 契约，全部针对 `common/` 中的脚本化引擎——无真实浏
览器。

| 文件 | 覆盖内容 |
|---|---|
| `session_flow.rs` | execute 流（navigate → snapshot）、审批授予的 cookies、wait_for resolve/timeout、经 manager 关闭 |
| `manager_flow.rs` | 会话上限（`max_sessions`）、close 语义、会话句柄稳定性 |
| `common/mod.rs` | 脚本化 launcher → engine → context → page 替身 |

私有行为单元测试（页面上限竞态、槽位记账）放在 `src/` 里紧挨代码
——Rust 的 `tests/` 目录够不到 crate 内部。
