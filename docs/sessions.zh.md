# Sessions

[English](sessions.md) | 中文

[`rutter-session`](../crates/session/src/manager.rs) 的 session
模型：一个 client 拥有什么、一个动作如何在其中旅行、登录状态如何
存活，以及引擎在其脚下死亡时会发生什么。

## 1. Session 与页面

- 一条 MCP 连接是一个 session；一个 session 恰好持有一个浏览器
  context。上限保护共享引擎：
  [`SessionConfig`](../crates/session/src/config.rs) 默认每服务器
  8 个 session、每 session 8 个页面。
- session 跟踪它打开的页面（`PageSlot`：id、最近已知 URL、句柄、
  活动标志）。恰好一个页面是活动的；页面级工具以它为目标。
  `tabs_select` 切换；`tabs_close` 提升第一个剩余页面；无页面时
  `navigate` 打开第一个。
- context 以
  [`ContextConfig`](../crates/engine/src/config.rs) 创建：页面上限、
  导航截止（30 s）、截图频率上限（500 ms）。

## 2. 动作执行路径

`Session::execute(action, origin)` 是每个动作经过的唯一路径——
MCP 工具与仪表盘控制皆然：

```
resolve active page
  → policy gate (agent origin only; policy.md)
  → ActionRequested event
  → executor: auto-wait → act → settle (tool-catalog.md §3)
  → ActionCompleted | ActionFailed event
  → refresh tracked page URL
  → persist storage state if it changed
```

- [origin](glossary.zh.md) 决定归因以及审批规则是否适用；两种
  origin 共享执行路径与事件时间线。
- 该路径上观察到的引擎失败被映射进
  [`ActionError`](../crates/core/src/error.rs) 分类再进入事件
  载荷，消费者因此只读一种失败词汇。

## 3. Storage state

[`StorageState`](../crates/session/src/storage.rs) 是
`{ cookies, origins }`——context cookie 加每个 origin 的
localStorage：

- **捕获**：每次动作之后与 `set_cookies` 之后，从 context 与打开
  的页面捕获。
- **变更即持久化**：写入
  `<cache-root>/sessions/<session-id>.storage.json`。写入是原子
  的：JSON 先落在同目录临时文件中，再以一次 rename 替换正式文件；
  Unix 上仅属主可读写。状态未变则不重写；写入失败会在下次捕获时
  强制重写。
- **重载**：显式 `save_storage`/`load_storage` 调用与恢复流程都会
  重载。这就是登录状态在引擎重启与进程重启之间存活的方式。

## 4. 恢复

当 supervisor 替换了死亡的引擎
（[引擎监管](engine-supervision.zh.md#3-进程监管)），manager 的
恢复任务重建每个 session
（[`Session::recover`](../crates/session/src/session/mod.rs)）：

1. 换入新 context。
2. 从最近捕获的 storage state 回放 cookie。
3. 按最近 URL 重新打开每个被跟踪页面并恢复其 localStorage；活动
   页面按位置恢复，因此恰好一个页面回来时是活动的。空白页面不予
   恢复。
4. 发布 `EngineRestarted`，让 agent 知道时间已流逝并重新截取
   快照。

关闭 session 会关闭其 context 并丢弃其事件 ring；manager 关闭时
打开的 session 随之消亡，但它们的 storage state 留在磁盘上，下次
启动时重载。
