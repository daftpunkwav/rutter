# tests/ — 真实引擎集成

[English](README.md) | 中文

针对下载好的 chrome-headless-shell 运行，默认 `#[ignore]`（CI 的
integration job 显式运行 `integration.rs`）：

| 文件 | 覆盖内容 |
|---|---|
| `integration.rs` | 启动、navigate/evaluate/screenshot、页面上限、幂等的 Context（上下文）关闭、消失目标的关闭 |
| `screencast.rs` | 真实 CDP 上的 `startScreencast` 帧流程与 ack 循环 |

首次运行会把约 150 MB 下载进 rutter 缓存（`rutter open` 复用）；离
线运行需要缓存已填充。
