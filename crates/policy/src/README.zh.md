# src/ — 文件地图

[English](README.md) | 中文

| 文件 | 职责 |
|---|---|
| `lib.rs` | 主要导出：`RuleSet`、`Review`、`Verdict`、`ApprovalBroker` |
| `rules.rs` | `RuleSet`：`review`（规范化、匹配、带简报作答）与 `evaluate`、审批窗口 |
| `brief.rs` | `ApprovalBrief`、`ApprovalEffect`、`VerdictBasis`：人类据以决策的内容 |
| `class.rs` | `ActionClass`：规则轴及其解析 |
| `pattern.rs` | `Pattern`：URL glob 匹配（空白模式被拒绝，而不是放宽） |
| `canonical.rs` | `canonical_url`：判决 URL 的 WHATWG 规范化（凭据诱饵 fail-closed） |
| `config.rs` | TOML 解析；拒绝空白模式与超过 24 h 的审批窗口 |
| `broker.rs` | `ApprovalBroker`：驻留/决定/等待，每个请求 id 一个决定 |

判决计算保持纯净；只有 broker 持有状态，而 session manager 持有每进
程唯一的 broker 实例。
