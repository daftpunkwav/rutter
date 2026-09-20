# rutter-mcp/ — MCP 工具面

[English](README.md) | 中文

把 `docs/TOOL_SPEC.md` 暴露为一个 rmcp server：stdio 与 streamable
HTTP 两种 transport，一条 MCP 连接 = 一个 Session（会话）。快照以文
本返回（带 `… truncated` 标记），截图以图像块返回，失败以携带消息加
提示的 `isError` 结果返回。

## 边界

只做协议映射。参数按规范校验（`invalid_params`）；全部语义都在
`rutter-session`，本 crate 绝不直接触碰引擎本身。引擎在第一个需要
Page（页面）的工具调用时惰性启动。

## 消费者

- `cli` 以 stdio（子进程客户端）或 `--http ADDR`（streamable HTTP，
  每连接一个会话）运行服务器。
