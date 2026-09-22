# tests/ — 每 crate 功能测试

[English](README.md) | 中文

仅针对本 crate 的公开 API 测试。

| 文件 | 覆盖内容 |
|---|---|
| `policy_flow.rs` | TOML 解析、带 URL 模式的首条匹配规则评估、审批 broker 的驻留/决定/超时生命周期 |
| `brief_wire.rs` | 审批简报的 serde 线上形状——与无 schema 的 dashboard 前端之间的契约 |
