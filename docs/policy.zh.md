# 策略与审批

[English](policy.md) | 中文

rutter 如何决定哪些动作直接运行、哪些被拒绝、哪些等待人类。这里的
一切是纯计算加 parked future——
[`rutter-policy`](../crates/policy/src/rules.rs) 不做任何 I/O。
判断来自 rutter 的规则，绝非 agent 的自我声明；这正是监督的意义。

## 1. 分类与 verdict

动作被分为六类（[`class.rs`](../crates/policy/src/class.rs)）：
`navigation`、`pointer`、`keyboard`、`selection`、`scroll`、
`cookies`。

每类映射到三个 verdict 之一：

| Verdict | 效果 |
|---|---|
| `allow` | 直接运行，无监督开销 |
| `deny` | 直接拒绝；调用方看到 `ApprovalDenied` |
| `require_approval` | 搁置动作，直到人类批准或拒绝，或窗口超时 |

## 2. 规则评估

[`RuleSet::evaluate`](../crates/policy/src/rules.rs) 按序匹配规则
列表；**首个匹配生效**，然后是默认 verdict。规则可设置：
`action_class`（单类，缺省为所有类）、`url_pattern`（`*` 通配）、
及其 `verdict` 的任意组合。

内置默认集对敏感类保守：**cookies 需要审批**，其余全部放行，审批
窗口 120 s。加载策略文件会替换整个集合——空文件是显式的宽松配置
（默认 `allow`、无规则），不是内置集。

## 3. 判定 URL

动作被判断的 URL 是动作的**去向**，不是来处：

- `Navigate` 以其规范化后的目标 URL 判定。
- 其余所有类以规范化后的当前页面 URL 判定。

规范化运行 WHATWG 解析器
（[`canonical_url`](../crates/policy/src/canonical.rs)）：主机大小
写与默认端口被归一化，文本技巧无法绕过模式匹配。

**Fail-closed。** 当不存在可用 URL——页面不可读、目标根本不是
URL、或目标内嵌凭据
（`https://good.example@evil.example/`）——session 调用
`evaluate_without_url`：URL 限定的规则不能匹配，仅类规则仍然
生效，裸 `allow` 升级为 `require_approval`。信息缺失永远不会让
动作在无监督下通过。

## 4. 审批

[`ApprovalBroker`](../crates/policy/src/broker.rs) 搁置动作直到
人类决定：

1. session 打开审批：broker 铸造 `apr-<n>` 并交出决定接收端。
2. session 发布 `ApprovalRequested` 并搁置。
3. 人类通过仪表盘答复——WebSocket `decision` 消息或
   `POST /api/decisions`（[仪表盘](dashboard.zh.md#4-决策-http-api)）。
4. `ApprovalResolved` 记录结果；`granted: true` 时被搁置的动作
   恢复执行。

映射：拒绝 → `ActionError::ApprovalDenied`；窗口期内沉默 →
`ApprovalTimedOut`。窗口默认 120 s，可配置
（`approval_timeout_ms`，上限 24 h）。对未知审批的迟到决定被
拒绝；被取消的等待者（断开的 client）回收自己的槽位。审批只适用
于[agent origin](glossary.zh.md)的动作；human origin 绕过审批，
记录在同一条时间线上。

## 5. TOML 配置

```toml
default = "allow"            # 无规则命中的动作使用的 verdict
approval_timeout_ms = 120000 # 人工答复窗口

[[rules]]
action_class = "navigation"
url_pattern  = "https://*.example.com/*"
verdict      = "allow"

[[rules]]
action_class = "cookies"
verdict      = "require_approval"
```

[`parse_policy`](../crates/policy/src/config.rs) 拒绝：未知的
verdict 或类名、既无 `action_class` 也无 `url_pattern` 的规则、
空白 `url_pattern`（它会静默把规则放宽到所有 URL）、以及超过
24 h 的窗口。解析错误携带 TOML 位置，方便运维修正文件。
