# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `lib.rs` | 库根：以下每个模块都在此声明；`config`、`entry`、`error` 是公开表面 |
| `main.rs` | clap 定义与进程入口（`rutter_cli` 之上的一层薄壳） |
| `entry.rs` | `EntryMode` 枚举：browse / serve / open / read |
| `config.rs` | `Settings::resolve`：旗标 + 环境 → 运行时设置 |
| `launcher.rs` | Engine（引擎）launcher 选择：缓存的 CdpLauncher 或显式二进制 |
| `serve.rs` | MCP 服务（stdio/HTTP）+ dashboard/policy 组装；每条退出路径都会关闭引擎 |
| `browse.rs` | 有头、人工驱动的模式；Ctrl-C 停止 supervisor |
| `open.rs` | 一次性：导航一次、打印快照、退出 |
| `read.rs` | 一次性抓取：导航一次、打印 markdown、退出 |
| `error.rs` | `CliError` 呈现：engine、signal、transport 失败的提示 |

这个 crate 只组装，不实现。新行为属于下层 crate，除非它确实关乎参
数解析或进程生命周期。
