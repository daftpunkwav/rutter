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
二进制；代码库中再无其他地方下载或启动引擎：

1. 显式 `--engine-executable <PATH>` 最优先。路径必须存在；其
   版本报告为 `external`；缓存不受影响。
2. 查询缓存。`<cache-root>/engines/<product>/<version>/` 存放解压
   后的二进制；命中即无需网络。
3. 未命中时，抓取 Chrome for Testing manifest 并下载对应的稳定
   渠道工件（按宿主平台选择 `chrome-headless-shell` 或 `chrome`），
   一次性安装进缓存。缓存版本被复用，直到缓存目录被清空——包括
   离线时。

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

- **每条 CDP 命令都在 30 s 截止时间内运行**
  （[`COMMAND_TIMEOUT`](../crates/engine-cdp/src/error.rs)），因此
  卡死的浏览器降级为错误，而不是挂死 session 或其监督。
- **Screencast** 使用 `Page.startScreencast`：JPEG，宽度 ≤ 1024，
  每帧一个 `screencastFrameAck` 回环。CDP 在导航时停止 screencast，
  因此传输在看到导航事件时重启截取。帧经过有界 channel（4）：
  慢的观看者丢帧，不丢内存（[仪表盘](dashboard.zh.md#5-screencast)）。
- **截图** 遵守每 context 最小间隔（500 ms），限制截取频率。
