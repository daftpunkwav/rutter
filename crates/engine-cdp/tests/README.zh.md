# tests/ — 真实引擎集成

[English](README.md) | 中文

针对下载好的 chrome-headless-shell 运行，默认 `#[ignore]`（CI 的
integration job 显式运行这里的全部套件）。每个文件只负责一件事：

| 文件 | 覆盖内容 |
|---|---|
| `common/` | 共享的引擎二进制解析（`RUTTER_TEST_ENGINE` 覆盖或缓存） |
| `engine_lifecycle.rs` | 启动、健康检查、navigate/evaluate/输入/screenshot、关闭、停机 |
| `context_pages.rs` | 页面上限、幂等的 Context（上下文）关闭、消失目标的关闭、查找与 cookie 边界 |
| `page_history.rs` | back/forward/reload 的生效 URL 与越界错误 |
| `foreign_pages.rs` | 非 rutter 打开的窗口：发现、采纳、关闭语义、跨上下文不可见 |
| `screencast.rs` | 真实 CDP 上的 `startScreencast` 帧流程与 ack 循环 |
| `read.rs` | Markdown 读取：标题、链接、列表、表格、代码，并省略站点 chrome |

首次运行会把约 150 MB 下载进 rutter 缓存（`rutter open` 复用）；离
线运行需要缓存已填充。
