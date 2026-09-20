# frontend/ — 面板源码

[English](README.md) | 中文

原生 JS，无构建步骤：dashboard crate 在编译期把这里的一切内嵌
（`include_str!`）。纯 ES2017+，文案经 i18n 目录管理（默认英文），无
框架，无 npm。

| 目录 | 内容 |
|---|---|
| [`src/`](src/README.md) | `index.html` 外壳 + `app.js` 客户端 |
| [`i18n/`](i18n/README.md) | 字符串目录（默认英文） |

与 `rutter-dashboard` 的协议契约：WebSocket 事件/screencast（屏幕
流）帧 + `POST /api/decisions`，见 `crates/dashboard/README.md` 与
docs/dashboard.md。
