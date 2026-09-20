# rutter-core/ — 领域词汇

[English](README.md) | 中文

所有其他 crate 都说的协议中立语言：类型化 Action（动作）、Snapshot
（快照）、元素 Reference（引用）、cookies、id，以及 agent 看到的错
误分类法。纯数据加小型纯函数——无 I/O、无 async、不认识 Engine（引
擎）。

## 边界

不依赖 workspace 内的任何东西。每个跨 crate 签名都用这些类型表述，
因此改动它们就是一次全 workspace 的契约变更（docs/architecture.md）。

## 关键类型

- `Action` + `Origin`（来源）——agent 请求了什么，以及是谁请求的。
  按事件约定序列化（`"type"` 标签、snake_case），因为它们随事件传
  播。
- `Snapshot` / `SnapshotNode`——以 YAML 渲染的无障碍视图（格式契
  约：`docs/snapshot-format.md`）。
- `Reference`——由序列化器铸造的稳定单页元素句柄，导航后失效。
- `ActionError`——失败词汇表；按事件约定序列化（`"type"` 标签、
  snake_case），因为它随事件传播。
