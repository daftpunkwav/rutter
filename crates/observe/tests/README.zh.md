# tests/ — 每 crate 功能测试

[English](README.md) | 中文

仅针对本 crate 的公开 API 测试。

| 文件 | 覆盖内容 |
|---|---|
| `snapshot_pipeline.rs` | envelope → 快照转换（截断旗标、渲染出的 ref）、内嵌脚本契约标记 |
| `page_scripts.rs` | 用 `node --test` 真实执行内嵌的 serializer 与 reader（无 Node 时跳过）：ref 清扫、markdown 表格规则与字符预算由执行证明，而不是读源码 |
