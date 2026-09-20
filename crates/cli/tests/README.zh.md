# tests/ — 每 crate 功能测试

[English](README.md) | 中文

通过公开 API 测试库目标（`rutter_cli`）。二进制级的验收测试放在
workspace 根的 `tests/` 包，因为它们横跨每个 crate。

| 文件 | 覆盖内容 |
|---|---|
| `settings.rs` | `Settings::resolve`：旗标优先级、透传的 engine 参数 |
| `errors.rs` | `CliError`：提示齐全，engine 错误保持其身份 |
| `entry_mode.rs` | `EntryMode` 显示名与值相等性 |
