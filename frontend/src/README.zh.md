# src/ — 面板客户端

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `index.html` | 页面外壳：窗格（timeline、approvals、live view）、`data-i18n` 钩子 |
| `app.js` | WebSocket 客户端：重放/实时渲染、审批决定、screencast 开关、自 1 s 起指数退避的重连（上限 30 s） |

token 随首次访问的 query 到达，由服务器兑换成 HttpOnly cookie；本脚
本从不存储它——没有 query token 时请求回退到 cookie（刷新、书签场
景）。缺失的 i18n 键回退为键名本身。
