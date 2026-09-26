# src/ — 文件地图

[English](README.md) | 中文

| Item | 职责 |
|---|---|
| `lib.rs` | 公开入口点；实现模块私有 |
| `assets/` | `serializer.js`、`reader.js`——页内脚本（见其 README） |
| `assets.rs` | 脚本的 `include_str!` 内嵌 |
| `builder/` | DOM 树 → `Snapshot` 流水线（见其 README） |
| `read.rs` | reader envelope → `Readout`（版本/截断/钳制） |
| `response.rs` | 序列化器 envelope → builder 输入（版本/截断旗标） |
| `resolver.rs` | Reference（引用）/focus/select/wait/storage 辅助脚本 |

只做纯变换：如果你打算在这里添加对 tokio、文件系统或网络客户端的依
赖，这个改动属于 `session`。
