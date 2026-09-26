# 仪表盘

[English](dashboard.md) | 中文

[`rutter-dashboard`](../crates/dashboard/src/lib.rs) 的本地监督
仪表盘：架在事件主干与审批 broker 之上的 web 服务器。仪表盘永远
不执行动作——观测加 verdict 提交，仅此而已。

## 1. 服务器

以 `rutter serve --dashboard <PORT>` 挂载。只绑定 `127.0.0.1`。
前端是无构建步骤的原生 JS，编译期嵌入二进制（`include_str!`）；
UI 字符串来自 `frontend/i18n/en.json` 目录。

| 路由 | 服务 |
|---|---|
| `/` | 页面外壳；将访问令牌换取会话 cookie |
| `/app.js`, `/i18n/en.json` | 应用脚本与字符串目录 |
| `/ws` | WebSocket（replay + 实时事件、决策、screencast） |
| `/api/decisions` | `POST` 审批决定，供简单自动化 client 使用 |
| `/api/pending` | `GET` 当前有多少审批正被搁置 |

## 2. 访问控制

每个端点（包括 WebSocket 升级）都经过同一个门
（[`auth.rs`](../crates/dashboard/src/auth.rs)）：

- **每次启动一个令牌。** 令牌在 dashboard 构造时生成一次，为 64 位
  十六进制——以随机种子的哈希器混合启动时间与进程 id。
  `RUTTER_DASHBOARD_TOKEN` 可为自动化覆盖；该覆盖对启动 rutter 的
  一方可见，所以它属于可信启动器，不属于人类专用信道。
- **URL 的交接。** 信道由 stderr 的另一端是谁决定。终端照常拿到
  `…/?token=…`；管道（MCP client 拉起 `rutter serve`）只拿到监听
  地址与交接文件路径，拿不到 URL 或令牌——那个 client 正是 rutter
  要监管的进程——URL 写入
  `<cache-dir>/dashboard-access-<port>.url`，Unix 上以仅属主可读创建。
  文件只在绑定成功后写入，文件名取服务器实际绑定的端口：`--dashboard 0`
  落在真实端口名下，同机两个 rutter 进程写不同文件，互不覆盖。既无
  终端又无该路径时，dashboard 拒绝启动，而不是退回打印令牌。
- **这换到什么、换不到什么。** rutter 与 client 同机同用户，该用户
  账户下的进程能读到 rutter 自己缓存里的任何文件。交接消除的是
  *自动*泄漏进被继承的信道，它不是人类专属的判据。必须阻止 agent
  自批的部署，要把 dashboard 跑在 agent 账户之外。
- **令牌换 cookie。** 首次带 `?token=` 访问会设置 `HttpOnly`、
  `SameSite=Strict` 的会话 cookie；之后的请求仅凭 cookie 认证。
  生成的 token 必定可安全放入 cookie；不可安全放入的
  `RUTTER_DASHBOARD_TOKEN` 覆盖值继续以 query 参数认证。
- **`Host` 校验**防范 DNS rebinding。

## 3. WebSocket 协议

一条 socket 承载一切
（[`ws.rs`](../crates/dashboard/src/ws.rs)）。连接时：

1. replay 所有打开 session 的历史，按全局序列号排序。
2. 实时流。不高于 replay 水位的信封是重复的，会被跳过。

如果 client 落后于 broadcast channel（`Lagged`），服务器会在继续
之前从 per-session ring 填补缺口——
[丢失契约](events.zh.md#3-投递语义)的实践。引擎尚未启动时，服务
器发送
`{"type":"note","text":"engine not started yet"}` 并关闭。

client 消息（JSON 文本帧）：

| 消息 | 效果 |
|---|---|
| `{"type":"decision","request_id":"apr-7","grant":true}` | 提交审批；回复是 `decision-ack`，回显 id 与是否受理。经 serde 构建，敌意 `request_id` 无法伪造回复字段 |
| `{"type":"screencast","on":true,"session":"…"}` | 开始该 session 活动页面的 screencast（`on: false` 或断开连接则停止）；以 `screencast-ack` 确认 |
| `{"type":"subscribe"}` | 接受但无操作：replay 与实时投递在连接时自动开始 |

服务器帧：文本帧承载 JSON 信封与 ack；二进制帧承载 JPEG
screencast 图像。

## 4. 决策 HTTP API

`POST /api/decisions`，体为
`{"request_id":"apr-7","grant":false}`，供简单自动化 client 提交
同样的决定，经同一个令牌门。未知或已决定的 `request_id` 返回
`404`；格式错误的体返回 `400`。

## 5. Screencast

Screencast **按需开启**：观看者请求时开始，离开时停止。它同时是
**只读的**：没有已打开页面的 session 回答 `no open page to observe`，
而不是去开一个——开 tab 是动作，而 dashboard 不执行任何动作
（docs/architecture.zh.md）。帧单向
流动——CDP `Page.startScreencast`（JPEG，宽度 ≤ 1024，逐帧 ack）
经容量 4 的有界 channel，channel 满时丢帧。卡顿的观看者丢帧而不
丢内存，截取也永远无法阻塞动作执行。CDP 在导航时停止 screencast；
传输在导航事件上重启截取
（[引擎监管 §4](engine-supervision.zh.md#4-cdp-传输说明)）。
