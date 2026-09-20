# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `lib.rs` | 公开导出：`RuleSet`、`Verdict`、`ApprovalBroker` |
| `rules.rs` | `RuleSet`：判决评估（`evaluate`、`evaluate_action`）、审批窗口 |
| `class.rs` | `ActionClass`：规则轴及其解析 |
| `pattern.rs` | `Pattern`：URL glob 匹配（空白模式被拒绝，而不是放宽） |
| `config.rs` | TOML 解析；拒绝空白模式与超过 24 h 的审批窗口 |
| `broker.rs` | `ApprovalBroker`：驻留/决定/等待，每个请求 id 一个决定 |

判决计算保持纯净；只有 broker 持有状态，而 session manager 持有每进
程唯一的 broker 实例。
