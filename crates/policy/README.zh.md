# rutter-policy/ — 监督规则

[English](README.md) | 中文

纯 Verdict（判决）计算加审批状态机。TOML 规则集把 Action（动作）类
× URL 模式映射到 `Allow | Deny | RequireApproval`；
`ApprovalBroker` 驻留需审批的动作、发布请求，并在人工决定或超时后了
结。

## 边界

只依赖 `core`；除解析 TOML 配置外无 I/O。它从不执行任何东西，也从不
追问请求者是谁——危险性由规则决定，绝不由 agent 自我声明决定
（docs/policy.md）。审批窗口（默认 120 s，超过 24 h 一律拒绝）位于
`RuleSet` 上，经配置中的 `approval_timeout_ms` 设置。

## 消费者

- `session` 在 agent 动作之前评估判决，并驻留在 broker 上。
- `dashboard` 向同一个 broker 提交决定（session manager 持有的那个
  ——每个进程恰好一个）。
