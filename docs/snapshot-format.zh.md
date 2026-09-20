# 快照格式

[English](snapshot-format.md) | 中文

观测管线的契约：页内序列化器产出的 JSON、引用的铸造方式、agent
阅读的文本渲染，以及 token 预算规则。`rutter-observe` 精确实现
本文；golden 与 property 测试将其钉住（[测试](testing.zh.md)）。

## 1. 管线

```
injected serializer (page)       rutter-observe
  composed DOM  ──►  JSON envelope  ──►  Snapshot  ──►  YAML text
      serializer_script()         snapshot_from_response()   Display
```

序列化脚本是 `rutter-observe` 的内嵌资产
（[`observe::serializer_script()`](../crates/observe/src/lib.rs)）。
调用方经引擎的 `evaluate` 在页面内执行它（`rutter-observe` 不依赖
引擎），再用 `observe::snapshot_from_response()` 转换返回的 JSON。

## 2. 序列化器信封

```json
{
  "version": 1,
  "truncated": false,
  "viewport": { "width": 1280, "height": 720 },
  "scroll": { "x": 0, "y": 0 },
  "root": { "...node...": "见 §3" }
}
```

- `version` —— 序列化器格式版本，当前为 `1`。
- `truncated` —— 序列化器触发了自身的节点守卫（50 000 节点），
  省略了部分合成 DOM。
- `viewport` / `scroll` —— 页面视口与滚动位置的 CSS 像素。预算只
  消费 `viewport`：节点矩形是视口相对的（§3），滚动偏移不参与
  分类，`scroll` 由仪表盘的 screencast 取景消费。viewport 缺失或
  格式错误时，转换器视其为无界（不执行视口优先折叠），依赖其余
  预算。
- `root` —— 文档根节点（role `root`）。

序列化器遍历合成 DOM，包括开放的 shadow root 与 slot 分配。不可见
元素（空包围盒或 `visibility: hidden`）被省略——隐藏元素从不靠猜
（诚实规则）。序列化器不抛异常：每个节点的计算都有守卫，失败时
该节点降级为无名 `generic`。

## 3. 节点形状

| 字段 | 类型 | 含义 |
|---|---|---|
| `role` | string | ARIA role：先看显式 `role` 属性，再看标签的隐式 role（link、button、textbox、heading、list、listitem、image、…），否则 `generic` |
| `name` | string, 可选 | 可访问名称，计算顺序：`aria-label` → `alt`（图片）→ 关联 `<label>`（表单控件）→ `placeholder`（文本框）→ `title` → 可见文本，空白折叠，上限 120 字符 |
| `value` | string, 可选 | 表单控件当前值，上限 200 字符 |
| `ref` | string, 可选 | 稳定句柄（§4）；出现在可操作且启用的元素上 |
| `checked` | bool, 可选 | 复选框/单选框状态 |
| `disabled` | bool | `disabled` 属性或 `aria-disabled="true"`；默认 false |
| `rect` | object, 可选 | `{x, y, width, height}`，CSS 像素，整数，视口相对 |
| `children` | array, 可选 | 按树序的子节点。带开放 shadow root 的宿主只报告其 shadow 树；被 slot 分配的 light-DOM 节点出现在其 slot 内（扁平化），未渲染的 light 子节点被省略 |

转换器忽略未知字段。格式错误的值降级为默认值（缺 role →
`generic`，类型错误 → 字段缺失）；超长字符串由转换器钳制
（role 与 ref 截到 64 字符，name 与 value 截到 200）并置
`truncated`。转换器从不在敌意页面数据上失败——
[敌意输入纪律](architecture.zh.md#横切不变量)适用于页面报告的
一切。

跳过的元素：`script`、`style`、`noscript`、`template`、`head`、
`meta`、`link`、`title`、`br`、SVG 内部（`svg` 元素报告为单节点，
无子节点）。

## 4. 引用铸造（v1）

- 序列化器在 `window` 上维护每页存储（`__rutterRefStore`：一个
  `WeakMap<Element, string>` 加整数计数器）。可操作元素第一次被
  观察到时获得 `e<N>`，并在同一页面的后续快照中保持不变。
- v1 可操作 role：`button`、`link`、`textbox`、`searchbox`、
  `checkbox`、`radio`、`combobox`、`listbox`、`option`、`menuitem`、
  `tab`、`slider`、`spinbutton`、`switch`、`treeitem`。
- 导航之后计数器重置；旧引用不再匹配任何元素，解析为
  `ActionError::ReferenceExpired`。

## 5. 文本渲染（YAML 风格）

每行一个节点，子节点每层缩进两格：

```
- button "Sign in" [checked] [ref=e17]
```

后缀按此顺序渲染：`"name"`（名称中的双引号转义为 `\"`）、
`[checked]`（仅 true 时）、`[disabled]`（仅 true 时）、`[ref=eN]`、
`× N`（折叠子树计数，§6）。折叠摘要行渲染为 `- listitem × 20`
（被折叠项的 role，数量 `× N`）。该格式由 `rutter-core` 中
`Snapshot` 的 `Display` 实现，由其单元测试钉住。

## 6. Token 预算（v1）

预算按 YAML 文本的渲染字符数计量；默认预算为每快照 20 000
字符。规则按序应用；每处预算引起的改变都置
`Snapshot::truncated = true`。

1. **深度预算。** 深于根下 48 层的节点被裁剪（裁剪点不渲染
   `- generic × N more` 摘要行；子节点直接缺席）。
2. **兄弟折叠。** ≥ 8 个连续同 role 且无名的兄弟折叠为一个摘要
   节点（保留 role，置 `× N`）。短于 8 的序列原样通过。
3. **视口优先裁剪。** 节点矩形是视口相对的；任何整体落在 y 轴
   `[-200 px, 视口高 + 200 px]` 带外、且渲染尺寸超过 400 字符的
   子树，无论剩余预算如何，都替换为其根的摘要节点（`× N` 计入
   被折叠子节点）。
4. **硬预算。** 剩余预算按文档顺序花费：放得下时子节点保留完整
   文本；放不下的子节点递归收缩；完全放不下的叶子或行被丢弃。
   每处预算引起的折叠或丢弃都置 `truncated`。
5. **安全上限**（敌意输入限制，独立于预算）：深度 512、节点
   100 000；触发时置 `truncated` 并降级，绝不 panic。

`snapshot_from_response(url, json)` 从不 panic、从不返回错误；
敌意输入一律降级（§2、§3）。
