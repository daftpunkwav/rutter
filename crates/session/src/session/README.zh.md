# session/ — Session 类型

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `mod.rs` | `Session` + `PageSlot`/`PageInfo`：活动 Page（页面）、execute、tabs 操作、存储持久化、screencast、close |
| `tests.rs` | 针对本 crate mock 引擎的无浏览器行为测试 |

单一活动页面是每个调用者都依赖的不变式；`execute` 在每次尝试后刷新
URL + 存储（变化时持久化，docs/sessions.md），`close` 幂等地拆除整个
浏览器 Context（上下文）。`tests.rs` 的存在是为了让逻辑文件保持可
读——新测试请放在那里。
