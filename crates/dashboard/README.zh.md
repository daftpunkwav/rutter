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
（docs/dashboard.zh.md §2）。

## 消费者

- `cli` 用 `--dashboard PORT` 把它挂到 `serve` 上。
