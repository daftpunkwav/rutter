# tests/js/ — 出厂 JavaScript 的行为测试

[English](README.md) | 中文

注入页面脚本与看板客户端里有字符串断言够不到的逻辑：ref store 的
清扫、markdown 表格规则、字符预算、重连重置。这些测试**真实执行**出
厂文件，跑在一个极简 DOM 上，断言它们的行为。用
`bash scripts/check_js.sh`（或 `node --test tests/js/*.test.mjs`）运行，
CI 跑的是同一个脚本。

零依赖：`node --test` 与 `node:vm` 都是 Node 内置，DOM 是
[`dom.mjs`](dom.mjs) 里的夹具。需要 Node 18 或更新。

| 文件 | 跑什么 |
|---|---|
| `dom.mjs` | 夹具：小到能读完的 DOM，外加 `ControllableWeakRef`——让测试自己决定某个元素何时被回收 |
| `serializer.test.mjs` | `crates/observe/src/assets/serializer.js`——ref 清扫及其阈值、只有可操作元素带 ref、shadow DOM 遍历、"永不抛异常"的守卫 |
| `reader.test.mjs` | `crates/observe/src/assets/reader.js`——colspan 表头、无单元格的首行、嵌套表格、竖线转义、字符预算、站点框架、链接、列表、引用、代码围栏 |
| `app.test.mjs` | `frontend/src/app.js`——静默的 decision ack、指名原因的 screencast 拒绝、重连对时间线与会话集的重置、重连退避、审批卡片去重、决策提交失败 |

夹具只建模这三个文件真正触碰到的平台接口。它们开始用到而 `dom.mjs`
没有建模的成员，会在需要它的那条测试里抛 `TypeError`——这是刻意
的：静默缺失的成员会让行为测试变成空断言。
