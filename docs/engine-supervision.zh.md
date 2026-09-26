# 引擎监管

[English](engine-supervision.md) | 中文

rutter 如何拥有一个浏览器进程：引擎抽象、二进制从哪里来，以及
supervisor 如何让它存活——或在不能力所能及时干净地让调用方失败。

## 1. 引擎抽象

[`rutter-engine`](../crates/engine/src/engine.rs) 定义代码库中唯一
的引擎抽象：

```rust
#[async_trait]
pub trait Engine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;   // backend, version
    async fn create_context(&self, config: ContextConfig)
        -> Result<Arc<dyn ContextHandle>, EngineError>;
    async fn health(&self) -> Result<HealthReport, EngineError>;
    async fn shutdown(&self) -> Result<(), EngineError>;
}
```

页面级操作——`navigate`、`evaluate`、`dispatch_input`、
`capture_screenshot`、`start_screencast`、…——位于
[`PageHandle`](../crates/engine/src/page.rs)。trait 之上的任何
代码都不得知道运行中的后端；更换或新增引擎被限制为实现
`EngineLauncher`（注册 trait）加 CLI 中的一个注册点
（[`crates/cli/src/launcher.rs`](../crates/cli/src/launcher.rs)）。

启动支持两种模式：`Headless`（serve 模式默认）与 `Headed`（可见
窗口，browse 模式与交互调试使用）。模式是引擎级配置，不改变
trait 之上的任何 API。

`rutter-engine-cdp` 是随附后端——唯一说 CDP 的 crate，基于
`chromiumoxide`。公开面：一个启动器（`CdpLauncher`），把可执行
文件路径变成运行中的
[`Engine`](../crates/engine-cdp/src/lib.rs)。

## 2. 引擎获取

[`ensure()`](../crates/engine/src/download/mod.rs) 解析可启动的
二进制；代码库中再无其他地方直接下载或启动引擎：

1. 显式 `--engine-executable <PATH>` 最优先。路径必须存在；其
   版本报告为 `external`；缓存不受影响。
2. 查询缓存。`<cache-root>/engines/<product>/<version>/` 存放解压
   后的二进制；命中即无需网络。
3. 未命中时，抓取 Chrome for Testing manifest 并下载对应的稳定
   渠道工件（`chrome-headless-shell` 或 `chrome`，取宿主平台对应的
   构建），一次性安装进缓存。缓存版本被复用，直到缓存目录被清空
   ——包括离线时。

缓存根默认为 `<OS 缓存目录>/rutter`，可由 `--cache-dir` /
`RUTTER_CACHE_DIR` 覆盖。进度输出到 stderr；stdout 保留给数据。

有头运行（browse 模式）在无显式路径时优先使用系统安装的浏览器：
`discover_system_browser()` 检查标准安装位置中的 Chrome、Edge 或
Chromium（不搜 `PATH`，不探测注册表），找不到则回退到完整的
Chrome for Testing 下载。

`serve` 把这一切推迟到首次启动
（[`LazyLauncher`](../crates/cli/src/launcher.rs)），因此进程在
任何下载或磁盘探测之前就达到 MCP 就绪。

## 3. 进程监管

[`Supervisor`](../crates/engine/src/supervisor/mod.rs) 拥有引擎的
生命周期：

- **心跳。** 每 10 s 探测一次 `Engine::health()`。
- **按策略重启。** 探针失败即替换引擎：启动尝试等待带上限的指数
  退避（基数 1 s，上限 30 s），同时滑动窗口熔断器统计尝试次数——
  60 s 窗口内重启过多会打开熔断器，调用方以 `Terminated` 失败，
  直到窗口排空。排空后心跳继续重试，因此故障或死亡的引擎会自行
  恢复。
- **健康的探测会关闭窗口。** 撑过一次心跳即清空尝试历史。没有这一步，
  崩溃并恢复三次的引擎即便之后干净运行一小时，仍只差一次崩溃就会被
  永久放弃——熔断器统计的会是「走到这里花了多少次」，而不是「现在
  出了多少问题」。
- **一个状态，一条重启路径。** 监管状态是单个 `Phase` 值（`Idle`、
  `Running`、`Replacing`、`BreakerOpen`），自带其重启历史，而不是由
  槽位、标志位与历史三件东西让读取方自己去对齐。策略驱动的启动只有
  一处（`bring_up`），首次启动与之后每次替换都走它。
- **重启串行化。** 互斥锁守护重启周期，并发失败只触发一次重启。
- **重启通知。** 每次成功替换都会递增一个 `watch` 计数；session
  manager 的恢复任务监视它并重建每个 session
  （[sessions](sessions.zh.md#4-恢复)）。

引擎死亡、重启中或熔断打开期间，`Supervisor::engine()` 报告
`EngineError::Terminated`，受影响的操作以该错误失败——引擎崩溃
绝不是 rutter 的崩溃。这是编排层不变量
（[架构](architecture.zh.md#横切不变量)）。

## 4. CDP 传输说明

随附后端中在 trait 之上可见的事实：

- **每条 CDP 命令都在 deadline 之下运行**——没有专属预算的调用
  使用固定的 [`COMMAND_TIMEOUT`](../crates/engine-cdp/src/error.rs)
  （30 s）——因此卡死的浏览器降级为错误，而不是挂死 session 或其
  监督。
- **Screencast** 使用 `Page.startScreencast`：JPEG，宽度 ≤ 1024，
  每帧一个 `screencastFrameAck` 回环。CDP 在导航时停止 screencast，
  因此传输在看到导航事件时重启截取。帧经过有界 channel（4）：
  慢的观看者丢帧，不丢内存（[仪表盘](dashboard.zh.md#5-screencast)）。
- **截图** 遵守每页面最小间隔（500 ms，由 context 配置），限制截取
  频率。
- **有头窗口是无 chrome UI 的应用面板**：当启动器自行解析有头浏览器
  时，headed 模式以 `--app` 加每次启动独立的 profile（无收藏、历史或
  登录态）拉起浏览器，可见窗口是纯页面表面——agent 操作、人类观看。
  显式指定的 `--engine-executable` 保留自己的窗口形态——Electron 把
  `--app` 保留为“运行该应用”之意
  （[browser README](../../browser/README.zh.md)）。rutter 自己拉起
  浏览器进程并轮询调试端口来解析地址——启动器式可执行文件（Edge）与
  完整 Chrome 都能启动；靠解析浏览器 stderr
  的启动方式在 Windows 上做不到这一点。
- **context 级隔离是尽力而为**：拒绝创建 context 与 target 的引擎
  （Electron 版 Rutter Browser，其唯一可见窗口即页面表面）会退回到
  默认 context 与浏览器既有的页面表面，此后 `descriptor()` 报告
  `per_context_isolation: false`
  （[capabilities](../crates/engine/src/descriptor.rs)）。
