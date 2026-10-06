# assets/ — 页内脚本

[English](README.md) | 中文

`serializer.js` 在 Page（页面）内部运行（经 `evaluate` 注入），把无
障碍树序列化为 `response.rs` 解析的 JSON envelope：role/name/ref/
rect 节点、viewport 报告与截断旗标。它把 ref 铸造进每页一个的存储
（引用铸造）；导航使其失效。

`reader.js` 以同样方式在页面内部运行，把可读内容提取为 `read.rs`
解析的 markdown envelope：`{ version, truncated, title, markdown }`，
站点框架与隐藏内容被省略（docs/read-format.md）。

`entropy.js` 不在快照时求值：引擎用
`Page.addScriptToEvaluateOnNewDocument` 安装它，因此它先于文档自身
的脚本运行。它在平台仍是平台时捕获所需的生成器、构造器与数字表，
并把 ref scope 铸造器锁定到全局对象上，这正是序列化器能铸造出页面
无法选择的 scope 的原因（`lib.rs::entropy_capture_script`）。对于
rutter 接入时已经加载完成的文档，改由引擎在隔离世界中为其铸造
scope。

约束：无构建步骤、纯 ES、在敌意页面上不得抛异常（例如在不透明
Origin（来源）上访问存储会返回 `{unavailable: true}`）。输出格式的
变更就是 docs/snapshot-format.md 与 docs/read-format.md 契约变更。
