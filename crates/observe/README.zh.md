# rutter-observe/ — 快照构建

[English](README.md) | 中文

把序列化后的 DOM 树变成带 token 预算的 Accessibility Snapshot（快
照）。拥有页内 JavaScript（serializer、resolver、wait、storage 辅助
脚本），由 session 通过引擎的 `evaluate` 注入。

## 边界

纯数据变换：无 async、无 I/O、无 tokio 依赖——除 `core` 外唯一拥有
该保证的 workspace crate，这也是它的 golden 与 property 测试不需要
任何浏览器就能运行的原因。调用者通过引擎注入脚本，把 JSON 交还给这
里。

## 契约

渲染文本由 `docs/snapshot-format.md` 规定（行格式、ref 铸造、预算）。
规范即契约；本 crate 实现它。
