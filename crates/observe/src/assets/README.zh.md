# assets/ — 页内脚本

[English](README.md) | 中文

`serializer.js` 在 Page（页面）内部运行（经 `evaluate` 注入），把无
障碍树序列化为 `response.rs` 解析的 JSON envelope：role/name/ref/
rect 节点、viewport 报告与截断旗标。它把 ref 铸造进每页一个的存储
（docs/snapshot-format.md §4）；导航使其失效。

约束：无构建步骤、纯 ES、在敌意页面上不得抛异常（例如在不透明
Origin（来源）上访问存储会返回 `{unavailable: true}`）。输出格式的
变更就是 docs/snapshot-format.md 契约变更。
