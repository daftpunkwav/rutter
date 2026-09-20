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

监管入口是
[`RuleSet::review`](../crates/policy/src/rules.rs)：它接受操作与其
原始 URL，自行完成 URL 规范化，返回一个 `Review`——放行、拒绝，或
连同人类据以决策的简报一起搁置。`evaluate` 留给只问类别的场合。判
定经过 `review` 就无法跳过规范化这一步。

## 3. 判定 URL

动作被判断的 URL 是动作的**去向**，不是来处：

- `Navigate` 以其规范化后的目标 URL 判定。
- 其余所有类以规范化后的当前页面 URL 判定。

规范化运行 WHATWG 解析器
（[`canonical_url`](../crates/policy/src/canonical.rs)）：主机大小
写与默认端口被归一化，文本技巧无法绕过模式匹配。

**Fail-closed。** 当不存在可用 URL——页面不可读、目标根本不是
URL、或目标内嵌凭据
（`https://good.example@evil.example/`）——`review` 按「没有 URL」
判定：URL 限定的规则不能匹配——除非其模式是全匹配
`*`；仅类规则仍然生效，裸 `allow` 升级为
`require_approval`。此时简报带 `judged_url: null` 与 basis
`missing_url`，人类因此能分辨「目标无法核实」与「目标已知」。信息
缺失永远不会让动作在无监督下通过。

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

### 人类看到什么

被搁置的请求携带 policy 构造的
[`ApprovalBrief`](../crates/policy/src/brief.rs)，而不是它搁置的那个
动作：

| 字段 | 含义 |
|---|---|
| `class` | 得出该 verdict 时所用的类 |
| `judged_url` | 被判定用的规范化 URL；fail-closed 时为 `null` |
| `basis` | 哪条规则说话了（`rule`，含一基序号与模式）、集合的 `set_default`，还是 `missing_url` 升级 |
| `effect` | 批准即授权什么：动作本身，或带数量的 `cookies` 写入 |

这段描述归 policy 所有，因为只有 policy 知道这四件事。cookie 写入没
有对应的 `Action` 变体，拿相近的动作顶替就会出现「人类批准的是
reload，实际授权的是写 cookie」——所以 `effect` 说自己是什么。线上
形状由 `crates/policy/tests/brief_wire.rs` 钉住；前端没有 schema
文件，这个测试是唯一让两侧对齐的东西。

### 审计轨迹

每个被搁置的决定都会向 `<cache-dir>/sessions/approvals.jsonl` 追加一行
JSON：时间、session、page、`request_id`、类别、被判定的 URL、basis、效果
摘要、结果，以及等待时长。

写它的是 session 层而非 dashboard，所以即使没人盯着，记录仍然存在——而
超时恰恰是最需要被解释的情形。结果区分 `granted`、`denied`、`timed_out`
与 `cancelled`，最后一种覆盖「client 在等待中途断连」。记录留下的是决定
本身，不是经过验证的人类身份：同机同账户下，rutter 无法区分坐在 dashboard
前的人与持有 dashboard 令牌的进程
（[仪表盘 §2](dashboard.zh.md#2-访问控制)）。

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
24 h 的窗口。TOML 语法错误携带 TOML 位置；上述拒绝以消息文本
报告。
