# rutter/ (cli) — 二进制入口

[English](README.md) | 中文

`rutter` 二进制与组合根：它依赖除 `events` 外的全部 workspace
crate（`events` 经 `session` 与 `dashboard` 间接到达）。三种
入口模式——browse（有头窗口，无 MCP）、serve（stdio 或 `--http` 上的
MCP，可选 `--dashboard` 与 `--policy`）、open（一次性 Snapshot（快
照））——都把旗标与环境解析为 `Settings`，并把 launcher、manager、
broker 与各服务器组装起来。

## 边界

只做组装与面向用户的错误呈现。所有行为都在它所组装的 crate 里；每
条退出路径（客户端断开、Ctrl-C）都会关闭 manager，使受监管的浏览器
绝不比服务器活得更久。

## 测试

`tests/` 覆盖库表面（settings、errors、入口模式）。二进制级的验收
测试放在 workspace 根的 `tests/` 包。
