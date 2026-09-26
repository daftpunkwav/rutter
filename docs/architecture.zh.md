# rutter 架构

[English](architecture.md) | 中文

按阅读顺序组织的入口文档：系统里有什么、各部分如何通信、哪些边界
必须守住。从本文开始读；每一节都链接到负责该主题细节的文档。

rutter 是一个面向 AI agent 的单二进制无头浏览器编排服务，使用
Rust 编写。它以受监管子进程的方式管理真实浏览器引擎，以
token 预算内的可访问性树快照（而非像素）暴露页面，以确定性的
三阶段 auto-wait 语义执行类型化动作，并通过 localhost 仪表盘和
人工审批门对 agent 进行监督。

设计上不做的事：rutter 不实现自己的渲染引擎（引擎是 trait 背后的
外部进程）、不做 stealth 或反指纹、不做消费者浏览器 UI、也不做
云服务。

## 工作区结构

十个 crate，一个依赖方向：箭头只向下。更深的规则是分层——一个
crate 永远不 import 排在它下方的 crate。Rust 在语言层面禁止循环
依赖；这张表就是分层契约。

| Crate | 依赖 | 唯一职责 |
|---|---|---|
| `rutter-core` | serde | 领域词汇：`Action`、`Snapshot`、`Reference`、错误 |
| `rutter-engine` | core | `Engine`/`Page` trait、supervisor、引擎下载器 |
| `rutter-engine-cdp` | engine, core | CDP 实现细节（chromiumoxide）——仅此而已 |
| `rutter-observe` | core | 注入式序列化脚本 + 纯 DOM→`Snapshot` 构建器 |
| `rutter-policy` | core | 规则集、verdict 评估、approval broker |
| `rutter-events` | core, policy | 事件类型、broadcast bus、环形缓冲、replay |
| `rutter-session` | core, events, engine, observe, policy | 编排：执行、context、storage state、恢复 |
| `rutter-mcp` | core, session | MCP 宿主（rmcp）与工具面 |
| `rutter-dashboard` | core, events, policy, session | HTTP/WS 服务器、内嵌前端、决策提交 |
| `rutter`（cli） | 以上除 `events` 外的全部（经传递引入） | 二进制入口模式、组合根、配置加载 |

这些 seam 存在的理由：

- **`core` 对上层一无所知。** 它是纯词汇；更换引擎或传输层永远
  不会触及它。
- **`events` 只因一个字段接触 `policy`。** 审批事件携带 policy 构造
  的简报，因为只有 policy 知道类别、被判定的规范化 URL、以及说话的
  那条规则。骨干里其余部分不认识 policy。
- **`engine-cdp` 是唯一允许说 CDP 的 crate。** 更换或新增引擎被
  限制在一个 crate 加 CLI 中的一个注册点
  （[`EngineLauncher`](../crates/engine/src/supervisor/mod.rs)）。
- **`observe` 是纯数据变换**（`serde_json::Value` 进，`Snapshot`
  出）加上它自有的 JS 资产。没有 async 代码；不依赖引擎即可单元
  测试。
- **`policy` 是纯计算**（`RuleSet` + class × URL → `Verdict`）
  加上 approval broker 的 parked future 记账。没有 I/O。
- **`session` 组合以上一切**——引擎、观测、策略、事件唯一交汇的
  地方。
- **`dashboard` 永远不执行动作。** 它只读事件、提交策略决策；
  监督 = 观测 + verdict。

每个目录的 `README.md` 陈述该模块的职责、边界与文件地图，从
[`crates/README.md`](../crates/README.md) 开始。

## 运行时形态

```
Agent (any MCP client)
  │  MCP (stdio / streamable HTTP)
  ▼
┌─ rutter (single Rust process) ─────────────────────────────┐
│  cli          argument parsing, config, process lifecycle  │
│  mcp          tool surface (rmcp)                          │
│  dashboard    axum server + embedded frontend (localhost)  │
│  session      orchestration: actions, contexts, recovery   │
│  policy       verdicts + approval broker (pure, no I/O)    │
│  observe      snapshot builder + injected serializer (pure)│
│  events       typed event backbone + ring buffer + replay  │
│  engine       Engine/Page traits, supervisor, downloader   │
│  engine-cdp   the only crate that knows CDP (chromiumoxide)│
│  core         shared domain vocabulary (types, errors)     │
└─────────────────────────────────────────────────────────────┘
  │ CDP over WebSocket (localhost)          ▲ events + frames
  ▼                                        │
chrome-headless-shell (child process)   Human browser (dashboard viewer)
```

- 每个 rutter 进程一个引擎子进程；每个 MCP session 一个浏览器
  context；每个 context 的页面数有上限。
- 引擎在第一个 session 请求时惰性启动——`rutter serve` 在任何
  引擎 I/O 之前就绪——并受监管：心跳探针、带上限的退避重启、
  滑动窗口熔断器（[引擎监管](engine-supervision.zh.md)）。
- 一次重启会触发 storage state 回放与页面恢复，且每个 session
  都会通过 `EngineRestarted` 事件得知此事
  （[sessions](sessions.zh.md)）。

### 入口模式

| 调用 | 行为 |
|---|---|
| `rutter` | browse 模式：有头引擎窗口，供人类直接操作 |
| `rutter serve` | MCP 服务器；引擎默认 headless，`--headed` 覆盖；`--dashboard PORT` 与 `--policy FILE` 挂载仪表盘与规则集 |
| `rutter open` | 一次性诊断：导航、打印快照、退出 |

无参数启动映射到 browse 模式，因此直接运行二进制就能得到可用的、
人类操作的 session，不需要任何 MCP client。引擎模式与仪表盘是
正交的开关；入口模式只设定默认值。命令行与环境变量见
[README](../README.md#usage)。

## 横切不变量

这些不变量在全代码库成立；各子系统文档依赖它们而不再复述。

- **一切等待都有界。** 每个 I/O 操作都有截止时间：CDP 命令 30 s、
  导航 30 s、每个 auto-wait 阶段 5 s、轮询预算钳制到 600 s。
  不存在无界等待。
- **引擎崩溃是错误，不是 rutter 崩溃。** 引擎死亡、重启中或熔断
  打开期间，受影响的操作以 `Terminated` 失败；心跳持续重试，
  supervisor 自行恢复。
- **错误是数据。** 每个动作失败都是
  [错误分类](tool-catalog.zh.md#2-结果约定)中的一个变体，可序列化，
  携带可执行的英文提示。`Internal` 意味着一个被围堵的 bug，绝不
  是静默通过。
- **敌意输入降级，绝不 panic。** 快照管线把任意页面数据当作不可信
  输入：截断、折叠、标记未知而不是失败。`unwrap`/`expect`/`panic`
  只允许出现在测试与进程初始化中（clippy 拒绝）。
- **事件绝不等待消费者。** 发布是 fire-and-forget；语义事件通过
  有界的 per-session ring 为迟到者保留，而 screencast 帧在负载下
  可丢弃（[events](events.zh.md)）。
- **确定性失败不重试。** 带上限的指数退避只用于引擎启动/连接；
  熔断器阻止重启风暴。

性能目标（设计目标；没有自动化基准度量它们——`scripts/benchmark.sh`
只报告 navigate+snapshot 成功率）：

| 指标 | 目标 |
|---|---|
| rutter 启动 → MCP 就绪（引擎惰性） | < 100 ms |
| 快照往返，热引擎，p50 | < 150 ms |
| 编排层 RSS（不含引擎） | < 50 MB |
| 每引擎进程的并发 context | 由 `max_sessions` 封顶（默认 8） |
| 仪表盘帧延迟（页面变化 → 像素） | < 500 ms |

## 文档地图

| 文档 | 角色 |
|---|---|
| [术语表](glossary.zh.md) | 规范领域词汇；一个概念一个词 |
| [工具目录](tool-catalog.zh.md) | MCP 工具面：传输、语义、auto-wait、错误映射 |
| [快照格式](snapshot-format.zh.md) | 观测管线：序列化器信封、引用、token 预算 |
| [读取格式](read-format.zh.md) | 读取管线：reader 信封、提取规则、守卫 |
| [引擎监管](engine-supervision.zh.md) | 引擎 trait、二进制获取、supervisor、CDP 说明 |
| [Sessions](sessions.zh.md) | session 模型、动作执行路径、storage state、恢复 |
| [事件](events.zh.md) | 事件词汇、backbone 语义、replay |
| [策略](policy.zh.md) | 动作分类、verdict 评估、fail-closed 规则、审批 |
| [仪表盘](dashboard.zh.md) | 服务器、访问控制、WebSocket 协议、screencast |
| [测试](testing.zh.md) | 测试层级、契约由哪些测试钉住、如何运行 |
