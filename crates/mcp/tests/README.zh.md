# tests/ — 每 crate 功能测试

[English](README.md) | 中文

仅针对本 crate 的公开 API 测试。

| 文件 | 覆盖内容 |
|---|---|
| `tool_params.rs` | 工具输入类型：schema 驱动的反序列化、`CookieInput` → `Cookie` 映射、方向映射 |
| `http_transport.rs` | 进程内 streamable HTTP 验收：`serve_http` 经 loopback、每连接一个 session、Origin 与 Host 守卫 |
| `common/` | 传输测试共享的 engine 替身夹具 |
