# builder/ — 快照流水线

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `mod.rs` | `build()`：转换 → 剪深度 → 折叠同级 run → viewport 剔除 → 适配字符预算 → 渲染 |
| `tests.rs` | Golden 快照（insta）、property 测试（proptest）、规范规则单元测试 |

预算与规则逐字遵循 SNAPSHOT_SPEC §6；改动它们就是契约变更。
`tests.rs` 把规范钉死，使流水线可以在不漂移的前提下重构。
