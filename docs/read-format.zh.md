# 读取格式（Read Format）

[English](read-format.md) | 中文

读取管道（read pipeline）的契约：页面内 reader 产出的 JSON、它提取什么、
省略什么，以及守卫规则。`rutter-observe` 精确实现本契约；转换单测钉住，
提取行为由集成测试钉住。

## 1. 管道

```
injected reader (page)        rutter-observe
  composed DOM  ──►  JSON envelope  ──►  Readout ──►  title + markdown
       reader_script()          read_from_response()      Display
```

reader 脚本是 `rutter-observe` 的内嵌资产
（[`observe::reader_script()`](../crates/observe/src/lib.rs)）。调用方通过
引擎的 `evaluate` 在页面内执行它（`rutter-observe` 不依赖引擎），然后把
返回的 JSON 交给 `observe::read_from_response()` 转换。

MCP `read` 工具（docs/tool-catalog.md §4）与 `rutter read <url>` CLI 模式
消费这条管道；两者都是只读观察——不产生事件、不做 auto-wait、不过策略门。

## 2. Reader envelope

```json
{
  "version": 1,
  "truncated": false,
  "title": "Example Domain",
  "markdown": "# Example Domain\n\nHello."
}
```

- `version` — reader 格式版本，当前为 `1`。
- `truncated` — reader 触发了某个守卫（§5），省略了部分内容。
- `title` — `document.title`，空白折叠。
- `markdown` — 页面可读内容组成的 markdown 文档（§3）。

## 3. 提取规则

按文档顺序渲染：

| 内容 | Markdown |
|---|---|
| `h1`–`h6` | `#`–`######` 标题行 |
| 段落与散块文本 | 普通段落，空白折叠 |
| `ul`/`ol`/嵌套列表 | `-` / `n.` 项，每层缩进两格 |
| `dl` 术语与定义 | 普通段落 |
| `table` | GFM 表格；首行为表头，单元格内 `|` 转义 |
| `pre` | 围栏代码块，围栏长度对内容中的反引号安全 |
| `blockquote` | 内部每行加 `> ` 前缀 |
| `hr` | `---` |
| `img`（有 `src`） | `![alt](绝对 src)` |
| `a[href]` | `[文本](绝对 href)`；href 无法解析时退化为纯文本 |
| `code`/`kbd`/`samp` | 反引号行内代码 |
| `strong`/`b`、`em`/`i` | `**…**`、`*…*` |

URL 在页面内按 `document.baseURI` 解析，因此链接与图片总是绝对地址。
链接文本转义 `[` 与 `]`；图片 alt 转义 `]`。

省略：

- **隐藏元素**（空盒或 `visibility: hidden`）——诚实规则：隐藏内容绝不
  猜测。
- **站点框架（site chrome）**：`nav`、`aside`、`footer`，以及 landmark
  角色 `navigation`、`complementary`、`banner`、`contentinfo`，另有
  `aria-hidden="true"` 的元素。
- **非内容标签**：`script`、`style`、`noscript`、`template`、`head`、
  `meta`、`link`、`title`、`base`、`datalist`（serializer 的跳过表），
  另加 `svg`、`iframe`、`canvas`、`dialog`。
- **交互控件**（`input`、`textarea`、`select`、`option`、`button`）——
  可访问性快照已用 ref 覆盖它们；readout 只承载可读内容。
- **SVG 内部结构**；行内 SVG 图形不提取（v1 已知限制）。

遍历与 serializer 一致地走 composed DOM：宿主的 open shadow root 只贡献
其影子树，slot 的 light-DOM 节点出现在 slot 的位置上。

## 4. Readout 与文本形态

`read_from_response` 产出
[`Readout`](../crates/core/src/readout.rs) `{ title, markdown,
truncated }`。`Display` 渲染标题行、一个空行、然后是 markdown 正文。
`truncated` 置位时由工具层追加 `… truncated` 标记行
（docs/tool-catalog.md §2），与 snapshot 同一约定。

## 5. 守卫

守卫以两种方式降级。预算类守卫——节点数、深度、markdown 大小、单次
行内大小——触发时置 `truncated: true`；遍历本身的失败（子节点读取
失败，或逸出整个遍历的失败）同样如此。省略类守卫则静默降级：隐藏
元素与站点框架被跳过而不置标记，无法解析的 URL 退化为纯文本，超长
标题被裁剪而不置标记。reader 绝不抛异常，转换器对敌意页面数据绝不
失败。

| 守卫 | 数值 |
|---|---|
| 节点预算 | 50 000 个元素 |
| 深度预算 | 200 层 |
| Markdown 大小 | 100 000 字符 |
| 单次行内调用大小 | 20 000 字符 |
| 标题长度 | 200 字符 |

转换器独立地对标题（200）与 markdown（100 000）做钳制——按字符计数，
不会切在 UTF-8 边界上——并在钳制生效或 envelope 版本比当前构建更新时置
`truncated`。缺失或畸形 envelope 产出空且 truncated 的 readout，而不是
错误。
