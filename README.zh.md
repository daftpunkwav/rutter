# rutter

[English](README.md) | 中文

一个面向 AI agent 的单二进制无头浏览器编排服务，使用 Rust 编写。rutter
将真实浏览器引擎作为受监管的子进程管理，以结构化视图（无障碍树快照）
而非像素的方式呈现页面，以确定性语义执行类型化操作，并提供带人工审批
的本地监督面板用于敏感操作。

## 文档

| 文档 | 定位 |
|------|------|
| [`docs/architecture.md`](docs/architecture.md) | 阅读顺序入口：crate 划分、运行时形态、横切不变量 |
| [`docs/glossary.md`](docs/glossary.md) | 规范领域词汇表 |
| [`docs/tool-catalog.md`](docs/tool-catalog.md) | MCP 工具面：传输、语义、auto-wait、错误映射 |
| [`docs/snapshot-format.md`](docs/snapshot-format.md) | 快照契约：序列化、引用、token 预算 |
| [`docs/engine-supervision.md`](docs/engine-supervision.md) | 引擎 trait、二进制获取、supervisor、CDP 说明 |
| [`docs/sessions.md`](docs/sessions.md) | session 模型、动作执行路径、storage state、恢复 |
| [`docs/events.md`](docs/events.md) | 事件词汇、backbone 语义、replay |
| [`docs/policy.md`](docs/policy.md) | 动作分类、verdict、fail-closed 规则、审批 |
| [`docs/dashboard.md`](docs/dashboard.md) | 仪表盘服务器、访问控制、WebSocket 协议 |
| [`docs/testing.md`](docs/testing.md) | 测试层级、契约钉定、运行方式 |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | 工作规则与本地门禁 |

每个源码目录都带有说明自身职责、边界与文件清单的 `README.md`（四个
共享测试助手目录 `tests/common/`、`crates/session/tests/common/`、
`crates/dashboard/tests/common/` 与 `crates/mcp/tests/common/`
由各自的父级测试 README 覆盖）—— 从
[`crates/README.md`](crates/README.md) 开始。每个 README 与每篇
`docs/` 文档都有中文镜像：`README.md` 旁是 `README.zh.md`，
`docs/` 下 `<name>.md` 旁是 `<name>.zh.md`；两种语言同步更新。

## 用法

```sh
# 一次性诊断：导航到指定 URL 并把 YAML 快照打印到 stdout。
rutter open https://example.com

# stdio 上的 MCP server：连接任意 MCP 客户端（引擎无头；--headed
# 改为可见窗口）。
rutter serve

# 同一个 MCP server 跑在 streamable HTTP 上，附带监督面板与策略文件。
rutter serve --http 127.0.0.1:8080 --dashboard 7700 --policy policy.toml

# 浏览模式（无子命令）：一个由人手动操作的可见引擎窗口。
rutter
```

全局旗标（所有模式均有效）：

| 旗标 | 环境变量 | 含义 |
|------|----------|------|
| `--engine-executable <PATH>` | — | 使用指定的浏览器二进制；跳过下载与缓存 |
| `--cache-dir <DIR>` | `RUTTER_CACHE_DIR` | 引擎缓存根目录（默认：OS 缓存目录 + `rutter`） |
| `--engine-arg <ARG>` | — | 传给引擎进程的额外参数（可重复） |

Serve 旗标（所有模式均接受，仅对 `rutter serve` 生效）：

| 旗标 | 含义 |
|------|------|
| `--http <ADDR>` | 在该地址以 streamable HTTP 提供 MCP（替代 stdio） |
| `--dashboard <PORT>` | 在 127.0.0.1:`<PORT>` 附带监督面板 |
| `--policy <FILE>` | 从 TOML 文件加载监督规则集 |
| `--allow-remote` | 确认非环回 `--http` 绑定（该传输没有认证） |

### 引擎获取

首次使用时 rutter 按以下顺序解析浏览器二进制：显式
`--engine-executable`、引擎缓存、Chrome for Testing 稳定通道（`open`
用 headless shell；浏览模式在找不到系统浏览器时用完整 Chrome）。下载
只发生一次；在缓存目录清空之前，缓存版本始终复用，离线亦可用。浏览
模式在存在系统安装的 Chrome 或 Edge 时优先使用它们。

引擎是受监管的：心跳检测进程死亡，重启采用有上限的指数退避，滑动窗
口熔断器阻止重启风暴——引擎崩溃表现为受影响操作上的错误，绝不会是
rutter 自身的崩溃。

### 策略与审批

动作按类（navigation、pointer、keyboard、selection、scroll、cookies）
对照 `类 × URL 模式 → 判决` 的规则集进行判决，规则集从 TOML 文件加
载。判决为 `allow`、`deny`、`require_approval`；导航按其规范化后的
目标 URL 判决，不可读页面 fail-closed 交由人工决策。cookies 默认需
要审批。

## 安装

从源码（需要 Rust 1.88+）：

```sh
cargo install --path crates/cli
```

Windows（msvc）、macOS（x64/arm64）与 Linux（x64）的发行档案由
cargo-dist 生成（`cargo dist build`，配置在根
[`Cargo.toml`](Cargo.toml) 的 `[workspace.metadata.dist]` 段）。浏
览器引擎本身不打包——rutter 在首次使用时把 Chrome for Testing 下载
进缓存（或用 `--engine-executable` 指向已有二进制）。

## 仓库布局

```
rutter/
├── docs/         # 面向代码的文档（英文 + 中文镜像）
├── scripts/      # CI 运行的质量门禁辅助脚本
├── crates/       # workspace 成员（见下表）
├── tests/        # 驱动二进制的跨 crate 验收测试
└── frontend/     # 面板源码（原生 JS，无构建步骤）
```

| Crate | 职责 |
|-------|------|
| `rutter-core` | 共享领域词汇：动作、快照、引用、错误 |
| `rutter-engine` | 引擎与页面 trait、二进制下载器、supervisor |
| `rutter-engine-cdp` | 唯一讲 CDP 的 crate（chromiumoxide） |
| `rutter-observe` | 页内脚本与快照构建器 |
| `rutter-events` | 类型化事件骨干：bus、环形缓冲、重放 |
| `rutter-session` | 编排：上下文、页面、动作、自动等待 |
| `rutter-mcp` | MCP 工具面（rmcp） |
| `rutter-policy` | 规则集、判决、审批 broker |
| `rutter-dashboard` | 本地监督面板（事件、审批） |
| `rutter`（cli） | 二进制入口模式：browse、serve、open |

## 开发

需要稳定的 Rust 工具链（1.88 或更新）。

```sh
cargo build
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

质量门禁，与 CI 一致：

```sh
bash scripts/check_headers.sh   # 每个源文件以头注开头
bash scripts/check_encoding.sh  # 跟踪的文本文件：UTF-8、LF、无 BOM
```

引擎集成测试需要真实二进制，默认 `#[ignore]`；显式运行（复用
`rutter open` 填充的缓存）：

```sh
cargo test -p rutter-engine-cdp --test integration -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-integration-tests --test open_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

`scripts/smoke_open.sh` 用 `rutter open` 驱动 10 个真实站点并打印通
过/失败摘要；`scripts/benchmark.sh` 运行 20 站点 navigate+snapshot
基准，以 90% 的成功率为通过门槛。

## 政策

- 提交遵循 Conventional Commits，英文，祈使语气。
- 代码注释用中文或英文撰写均可；文档以英文 `README.md` 交付，
  并附中文 `README.zh.md` 镜像，两种语言同步更新。
- 安全问题请私下报告——见 [`SECURITY.md`](SECURITY.md)。
- 许可证：Apache-2.0（见 [`LICENSE`](LICENSE)）。
