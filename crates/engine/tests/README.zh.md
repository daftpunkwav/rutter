# tests/ — 每 crate 功能测试

[English](README.md) | 中文

仅针对本 crate 的公开 API 测试。私有路径的单元测试放在 `src/` 里紧
挨代码（Rust 集成测试够不到 crate 内部）。

| 文件 | 覆盖内容 |
|---|---|
| `supervisor_flow.rs` | 脚本化 launcher 之上的 supervisor 生命周期：启动/关闭、心跳替换、熔断器、启动失败恢复 |
