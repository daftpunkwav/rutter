# rutter-engine/ — 引擎 trait、下载、监督

[English](README.md) | 中文

为 rutter 其余部分定义浏览器 Engine（引擎）是什么（`Engine`、
`ContextHandle`、`PageHandle` trait），并拥有一切维持其存活的东西：
Chrome-for-Testing 下载器，以及带心跳、有上限退避重启与熔断器的
supervisor。

## 边界

只说 `rutter-core` 类型和自己定义的 trait——它不能点名 CDP 类型。下
载哪个二进制、如何启动它，由从上层注入的 `EngineLauncher` 实现决定
（`engine-cdp` 提供真实实现；测试提供脚本化的替身）。

## 消费者

- `session` 只通过 trait 驱动引擎。
- `engine-cdp` 实现 trait 并暴露唯一的 launcher。
- `cli` 选择启动模式并解析 `--engine-executable`。
