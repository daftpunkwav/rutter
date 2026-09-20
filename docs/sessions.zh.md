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
- 被跟踪的页面住在
  [`PageRegistry`](../crates/session/src/pages.rs) 里：槽位由
  （`id`、最近已知 URL、句柄、活动标志）组成，锁由注册表自己持有。
  恰好一个页面是活动的，守住这一点的是注册表而不是调用方。
  `tabs_select` 切换；`tabs_close` 提升第一个剩余页面。
- **两种查找，故意不同。** `ensure_page` 在没有页面时会打开 session 的
  第一个页面——这是动作需要的。只读查找则回答 `None`，dashboard 的
  screencast 用的就是它：请求观看不应反而造出被观看的东西，所以没有
  页面时回答 `SessionError::NoOpenPage`（docs/architecture.zh.md：
  dashboard 绝不执行动作）。
- context 以
  [`ContextConfig`](../crates/engine/src/config.rs) 创建：页面上限、
  导航截止（30 s）、截图频率上限（500 ms）。
- manager 分开守护三件互不相关的事——引擎槽位（读多写少）、启动锁（只
  跨越 launch 本身持有）、session 表（只管记账）——而恢复在重建任何内容
  之前先对 session 列表做快照，且在所有锁之外执行。曾经一把锁同时守护三
  者，于是一次慢启动（在重启熔断窗口排空期间可退避长达一分钟）会把
  dashboard 读取事件骨干与 session 列表一起堵死。

## 2. 动作执行路径

`Session::execute(action, origin)` 是每个动作经过的唯一路径。MCP
工具以 `Origin::Agent` 调用它；仪表盘提交审批决策，从不执行动作：

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

- **捕获**：每次动作之后与 `set_cookies` 之后，从 context 与动作
  所在的那个页面捕获。
- **变更即持久化**：写入
  `<cache-root>/sessions/<session-id>.storage.json`。写入是原子
  的：JSON 先落在同目录临时文件中，再以一次 rename 替换正式文件；
  Unix 上仅属主可读写。状态未变则不重写；写入失败会在下次捕获时
  强制重写。
- **重载**：恢复流程与显式 `load_storage` 会重载（`save_storage`
  只写不读）。这就是登录状态在引擎重启之间存活的方式。

## 4. 恢复

当 supervisor 替换了死亡的引擎
（[引擎监管](engine-supervision.zh.md#3-进程监管)），manager 的
恢复任务重建每个 session
（[`Session::recover`](../crates/session/src/session/mod.rs)）：

1. 给注册表设门：读出被跟踪的 URL，在第 5 步落地前拒绝查找与新登记。
2. 换入新 context。
3. 从最近捕获的 storage state 回放 cookie。
4. 按最近 URL 重新打开每个被跟踪页面并恢复其 localStorage；活动
   页面按位置恢复，因此恰好一个页面回来时是活动的。空白页面不予
   恢复。
5. 装入重建后的列表、解除门、发布 `EngineRestarted`，让 agent 知道
   时间已流逝并重新截取快照。

这道门解释了：恢复中途到达的动作为什么以 `Terminated` 失败，而不是
悄悄开一个页面。没有它，在「读出」与「写回」之间登记的页面会在注册表
里被覆盖，而它的 tab 仍活在新引擎中——`tabs_list` 看不见它，页面上限
也不计它。

关闭 session 会关闭其 context 并丢弃其事件 ring；manager 关闭时
打开的 session 随之消亡。storage 文件留在磁盘上，但新 session 从
空状态开始——没有任何路径会把旧文件读回来。
