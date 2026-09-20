# rutter-session/ — 编排

[English](README.md) | 中文

引擎、观察、policy 与事件唯一交汇的地方：每个 MCP 客户端一个
`Session`（会话），拥有一个浏览器 Context（上下文），以三阶段自动等
待执行类型化 Action（动作），评估 policy 并驻留审批，持久化存储状
态，并在引擎重启后重新打开其 Page（页面）。

## 边界

说 `core` 类型，只通过 `engine` 的 trait 驱动引擎，经 `observe` 读快
照，经 `policy` 判决，并发布 `events`。它之上没有任何东西知道一个动
作究竟如何发生；它之下的任何东西都不得回调进本 crate。

## 关键概念

- `SessionManager`——每进程一个受监管引擎，每客户端一个会话，
  `max_sessions`/`max_pages` 上限，以及恢复任务。
- `Session::execute`——policy 门 → 动作 → 事件 → URL/存储刷新；每
  次尝试之后都运行，无论成功与否。
- 存储状态（cookies + localStorage）在变化时持久化，并在 supervisor
  替换死掉的引擎时自动重放。
