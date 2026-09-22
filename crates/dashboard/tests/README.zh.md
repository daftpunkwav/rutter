# tests/ — 每 crate 功能测试

[English](README.md) | 中文

仅针对本 crate 的公开 API 测试。

| 文件 | 覆盖内容 |
|---|---|
| `token.rs` | 每次启动 token 的性质：16 个十六进制字符，每个服务器实例唯一 |
| `http_flow.rs` | 环回上的真实服务器：每条路由的 token 门、query token 换 cookie、决定 POST（`400`/`404`/`200`）、pending 计数、404 fallback |
| `ws_flow.rs` | 真实 WebSocket 循环：引擎未启动的 note、先重放后实时的顺序、决定 ack、screencast/subscribe 控制帧、无 token 时拒绝升级 |
| `common/mod.rs` | 共享的可脚本化 engine 替身（launcher → engine → context → page），流程测试无需浏览器即可运行 |
