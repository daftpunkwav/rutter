# rutter-dashboard/ — 监督面板

[English](README.md) | 中文

本地 web 服务器（127.0.0.1，每次启动一枚 token），把 Event（事件）
与 screencast（屏幕流）帧流式推送给内嵌的原生 JS 前端，并把人工审
批决定提交回 broker。

## 边界

观察加上 Verdict（判决）提交——dashboard 绝不执行 Action（动作）
（blueprint §5）。它读取事件骨干（先重放，后实时），只在有观看者时
拉取 screencast 帧，并把决定提交给会话驻留等待的同一个 broker。每个
端点都经过同一道门：环回 `Host` 检查 + token（query 或 HttpOnly
cookie）——见 `src/auth.rs`。

## 消费者

- `cli` 用 `--dashboard PORT` 把它挂到 `serve` 上。
