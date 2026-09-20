# supervisor/ — 让一个引擎活着

[English](README.md) | 中文

掌管当前引擎槽位：带重试/退避的启动、心跳健康探测、滑动窗口熔断器
约束下的有上限重启，以及干净的关闭。

| 文件 | 职责 |
|---|---|
| `mod.rs` | `Supervisor`：`start`/`engine`/`shutdown`，心跳与重启循环 |
| `policy.rs` | `RestartPolicy`：窗口、上限、退避 schedule、熔断决策 |

生命周期测试在本 crate 的 `tests/supervisor_flow.rs`。

消费者每次都必须重新询问 `engine()`——重启期间槽位会变化，缓存
`Arc` 会钉住一个死实例（这条规则就是为那个 bug 而生）。
