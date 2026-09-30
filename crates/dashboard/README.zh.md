# rutter-dashboard/ — 监督面板

[English](README.md) | 中文

本地 web 服务器（127.0.0.1，每次启动一枚 token），把 Event（事件）
与 screencast（屏幕流）帧流式推送给内嵌的原生 JS 前端，并把人工审
批决定提交回 broker。

## 边界

观察加上 Verdict（判决）提交——dashboard 绝不执行 Action（动作）
（docs/architecture.md）。它读取事件骨干（先重放，后实时），只在有观看者时
拉取 screencast 帧，并把决定提交给会话驻留等待的同一个 broker。每个
端点都经过同一道门：环回 `Host` 检查 + token（query 或 HttpOnly
cookie）——见 `src/auth.rs`。token 本身由 `DashboardServer::hand_off`
交接，信道由 stderr 的另一端是谁决定：终端拿到 URL，管道（被监管的
MCP client 的情形）拿到一个 Unix 上仅属主可读的文件，打印行只出现
路径
（访问控制，docs/dashboard.zh.md）。

## wire 契约

WebSocket 的消息词汇表——客户端会收到的 envelope（先重放，后实
时）、客户端可以发送的消息（`decision`、`screencast`、
`subscribe`）、以及作为回应的 ack 与二进制 screencast 帧——记录在
[docs/dashboard.zh.md 第 3 节](../../../docs/dashboard.zh.md#3-websocket-协议)。
这是第三方客户端对齐实现的契约；内嵌的 `frontend/src/app.js` 只是
它的一个消费者，不是规范本身。decision 的消息体与 HTTP post 共用
同一个解析函数（`src/lib.rs` 的 `parse_decision`），两条传输不可
能对"什么是一条决定"产生分歧。

## 消费者

- `cli` 用 `--dashboard PORT` 把它挂到 `serve` 上。
