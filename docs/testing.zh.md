# 测试

[English](testing.md) | 中文

验证体系在哪里、哪些测试钉住哪些契约、如何运行各套件。

## 1. 层级

| 层级 | 范围 | 工具 |
|---|---|---|
| Unit | 纯 crate（core、observe、policy、events）及代码旁的私有路径 | 普通 `#[test]` |
| Golden | fixture DOM 树上的快照构建器输出 | `insta` |
| Property | 裁剪/折叠不变量；序列化器永不 panic | `proptest` |
| Integration | 真实 headless shell：启动、导航、动作 | 各 crate `tests/`，默认 `#[ignore]` |
| E2E | 完整 MCP client → rutter → 引擎往返（mcp、approval、http 传输） | 工作区 `tests/` 包 |
| Benchmark | 20 站点固定语料：navigate+snapshot 成功率 | `scripts/` 中的基准 |

## 2. 契约由哪些测试钉住

- [工具目录](tool-catalog.zh.md)语义由 `rutter-mcp` 单元测试
  （结果约定、错误映射、schema 细节）与 `tests/` 中的 e2e 套件
  钉住。
- [快照格式](snapshot-format.zh.md)由 `rutter-observe` 与
  `rutter-core` 的 golden fixture 与 property 测试钉住
  （`Display` 渲染）。
- 数值默认——auto-wait 预算、页面/session 上限、审批窗口、重启
  策略——在所属 crate 的单元测试中断言（例如
  [`SessionConfig::default`](../crates/session/src/config.rs) 与
  工具目录一致）。
- 引擎监控行为（退避、熔断、心跳恢复）在 `rutter-engine` 测试中
  使用注入时钟与快速、确定性的策略覆盖。

## 3. 运行

```sh
cargo test --all            # unit + golden + property 套件
```

集成与验收套件驱动真实引擎，本地默认 `#[ignore]`；显式运行
（它们复用 `rutter open` 填充的缓存）：

```sh
cargo test -p rutter-engine-cdp --tests -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-integration-tests --test open_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

语料基准：

```sh
scripts/smoke_open.sh    # 10 个真实站点走 `rutter open`，输出 pass/fail 汇总
scripts/benchmark.sh     # 20 站点 navigate+snapshot 基准，90 % 成功率门槛
```

## 4. 质量门

CI 在每次 push/PR 上运行。quality job 运行
[`CONTRIBUTING.md`](../CONTRIBUTING.md) 中的本地检查，外加 rustdoc
门禁：

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo doc --locked --no-deps
cargo test --all
bash scripts/check_headers.sh   # 每个源文件以真实的头注释开头
bash scripts/check_encoding.sh  # 跟踪的文本文件：UTF-8、LF、无 BOM
```

另有 `cargo-deny`（`deny.toml`：安全通告、许可证、禁用项）。
`#[ignore]` 套件也在 CI 运行：integration job 以 `--tests` 执行
`rutter-engine-cdp` 全部套件（含 screencast），e2e job 在 ubuntu 与
windows 上以真实引擎跑完 `rutter-integration-tests` 的全部套件；
coverage job（ubuntu）度量整个 workspace——引擎套件与 e2e 一并计入
——行覆盖低于 90 % 即失败（§5）。concurrency 组会取消同一 PR 被更新的
push 取代的旧运行，每个 job 都有 `timeout-minutes` 上限，三个引擎
job 共用一个复合 action
（[engine-setup](../.github/actions/engine-setup/action.yml)）负责引擎缓存与
Linux 引擎依赖。发布归档由 cargo-dist 生成
（`cargo dist build`，配置在根 `Cargo.toml` 的 `[workspace.metadata.dist]` 段）；仓库
中没有入库的 release workflow。语料基准是手动运行，不是 CI 门槛；
`scripts/benchmark.sh` 报告 ≥ 90 % 的 navigate+snapshot 成功率
门槛。

## 5. 覆盖率

行覆盖率用
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) 在整个
workspace 上测量，包含 `#[ignore]` 的引擎套件，且必须在同一次
插桩运行中完成（build、测试、报告共用一次调用，否则插桩的
`rutter.exe` 会在 e2e 子进程继承它之前被清掉）：

```sh
export RUTTER_BIN="$PWD/target/llvm-cov-target/debug/rutter"
cargo llvm-cov --locked --workspace --bins --tests \
    --summary-only --output-path target/coverage-full.txt \
    -- --include-ignored --test-threads=1
```

全量运行（含驱动插桩二进制的 `open_e2e`、`mcp_e2e`、
`approval_e2e`、`http_e2e`）在 2026-09-27 测得 **92.79 % 行覆盖**
（6356/6850 源码行；测试目标本身不计入）。剩余未覆盖行属于：
headed/browse 模式路径（`crates/cli/src/browse.rs` 的全部 15 行）、
引擎下载器的网络中段失败路径、需要卡死浏览器才能触发的防御性分支，
以及部分 WebSocket 重连路径。

CI 对这一数字设门槛：coverage job 用同一命令形态（lcov 输出代替
汇总）度量，行覆盖低于 90 % 时经
[`scripts/check_coverage.sh`](../scripts/check_coverage.sh) 失败，
因此 CI 数字与上面的本地数字同法同源。
