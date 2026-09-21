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
cargo test -p rutter-engine-cdp --test integration -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

语料基准：

```sh
scripts/smoke_open.sh    # 10 个真实站点走 `rutter open`，输出 pass/fail 汇总
scripts/benchmark.sh     # 20 站点 navigate+snapshot 基准，90 % 成功率门槛
```

## 4. 质量门

CI 在每次 push/PR 上运行，与
[`CONTRIBUTING.md`](../CONTRIBUTING.md) 中的本地检查一致：

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
bash scripts/check_headers.sh   # 每个源文件以真实的头注释开头
bash scripts/check_encoding.sh  # 跟踪的文本文件：UTF-8、LF、无 BOM
```

另有 `cargo-deny`（`deny.toml`：安全通告、许可证、禁用项）和一个
以缓存引擎运行 `rutter-engine-cdp` 集成套件的集成任务（其余
`#[ignore]` 套件为本地运行）。发布归档由 cargo-dist 生成
（`cargo dist build`，配置在根 `Cargo.toml` 的 `[dist]` 段）；仓库
中没有入库的 release workflow。语料基准是手动运行，不是 CI 门槛；
`scripts/benchmark.sh` 报告 ≥ 90 % 的 navigate+snapshot 成功率
门槛。
