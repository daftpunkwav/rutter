# rutter-engine-cdp/ — CDP 后端

[English](README.md) | 中文

在 chromiumoxide 之上实现 `rutter-engine` 的 trait。这是 workspace
中唯一允许点名 CDP 或 chromiumoxide 类型的 crate（docs/architecture.md）；公
开面是 `CdpLauncher`——一个 `EngineLauncher`——所有实现模块都是私有
的，公开签名中不出现任何 CDP 类型。

## 边界

只依赖 `core` 与 `engine`。消费者把 `CdpLauncher::new` 接进
supervisor，永远无需知道 CDP 存在。所有 CDP 调用都在 deadline 之下
运行（`error::with_deadline`）；浏览器侧的意外（目标消失、Context
（上下文）已经死亡）会折入协议中立的错误分类法，而不是泄漏协议文
本。

## 测试

`tests/` 针对真实下载的引擎运行，默认 `#[ignore]`（见其 README）。
