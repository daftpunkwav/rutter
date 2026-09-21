# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `action.rs` | `Action` 枚举、`Origin`、`ScrollDirection`——请求词汇表 |
| `snapshot.rs` | `Snapshot`/`SnapshotNode` 及 YAML 渲染（docs/snapshot-format.md） |
| `reference.rs` | `Reference`——不透明的单页元素句柄 |
| `cookie.rs` | 传给 Engine（引擎）后端的 `Cookie`/`SameSite` |
| `ids.rs` | Newtype id（`SessionId`、`PageId`、`ContextId`） |
| `error.rs` | `ActionError` 分类法；serde 标签与事件约定一致 |

先在这里新增领域词汇，然后每次教一个消费者。任何带 I/O 或 async 的
东西都不属于这个 crate。
