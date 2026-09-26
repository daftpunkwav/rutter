# Read Format

English | [中文](read-format.zh.md)

The contract for the read pipeline: the JSON the in-page reader
produces, what it extracts, what it omits, and the guard rules.
`rutter-observe` implements exactly this; unit tests pin the
conversion and integration tests pin the extraction.

## 1. Pipeline

```
injected reader (page)        rutter-observe
  composed DOM  ──►  JSON envelope  ──►  Readout ──►  title + markdown
       reader_script()          read_from_response()      Display
```

The reader script is an embedded asset of `rutter-observe`
([`observe::reader_script()`](../crates/observe/src/lib.rs)). The
caller evaluates it inside a page via the engine's `evaluate` (no
engine dependency in `rutter-observe`), then converts the returned
JSON with `observe::read_from_response()`.

The MCP `read` tool (docs/tool-catalog.md §4) and the
`rutter read <url>` CLI mode consume the pipeline; both are read-only
observation — no events, no auto-wait, no policy gate, no page
creation.

## 2. Reader envelope

```json
{
  "version": 1,
  "truncated": false,
  "title": "Example Domain",
  "markdown": "# Example Domain\n\nHello."
}
```

- `version` — reader format version, currently `1`.
- `truncated` — the reader hit one of its guards (§5) and omitted part
  of the content.
- `title` — `document.title`, whitespace-collapsed.
- `markdown` — the readable content of the page as a markdown
  document (§3).

## 3. Extraction rules

Rendered, in document order:

| Content | Markdown |
|---|---|
| `h1`–`h6` | `#`–`######` heading lines |
| paragraphs and stray block text | plain paragraphs, whitespace-collapsed |
| `ul`/`ol`/nested lists | `-` / `n.` items, two spaces per nesting level |
| `dl` terms and definitions | plain paragraphs |
| `table` | GFM table; the first row is the header, `|` escaped in cells |
| `pre` | fenced code block, fence length safe against embedded backticks |
| `blockquote` | every inner line prefixed `> ` |
| `hr` | `---` |
| `img` (with `src`) | `![alt](absolute-src)` |
| `a[href]` | `[text](absolute-href)`; unresolvable hrefs degrade to the text |
| `code`/`kbd`/`samp` | backtick spans |
| `strong`/`b`, `em`/`i` | `**…**`, `*…*` |

URLs are resolved against `document.baseURI` in the page, so links and
images always carry absolute URLs. Link text escapes `[` and `]`;
image alt escapes `]`.

Omitted:

- **Hidden elements** (empty box or `visibility: hidden`) — the
  honesty rule: hidden content is never guessed at.
- **Site chrome**: `nav`, `aside`, `footer`, and the landmark roles
  `navigation`, `complementary`, `banner`, `contentinfo`, plus
  `aria-hidden="true"` elements.
- **Non-content tags**: `script`, `style`, `noscript`, `template`,
  `head`, `meta`, `link`, `title`, `base`, `datalist` (the serializer's
  skip list) plus `svg`, `iframe`, `canvas`, and `dialog`.
- **Interaction surfaces** (`input`, `textarea`, `select`, `option`,
  `button`) — the accessibility snapshot covers them with refs; the
  readout is for readable content.
- **SVG internals**; inline SVG graphics are not extracted (v1
  limitation).

Traversal walks the composed DOM like the serializer: an open shadow
root contributes only its shadow tree, and slotted light-DOM nodes
appear inside their slot.

## 4. Readout and text form

`read_from_response` yields a
[`Readout`](../crates/core/src/readout.rs) `{ title, markdown,
truncated }`. `Display` renders the title, one blank line, then the
markdown body. The tool layer appends the `… truncated` marker line
when `truncated` is set (docs/tool-catalog.md §2), the same convention
as snapshots.

## 5. Guards

Every guard trip sets `truncated: true` and degrades; the reader never
throws, and the converter never fails on hostile page data.

| Guard | Value |
|---|---|
| Node budget | 50 000 elements |
| Depth budget | 200 levels |
| Markdown size | 100 000 characters |
| Per-inline-call size | 20 000 characters |
| Title length | 200 characters |

The converter independently clamps title (200) and markdown
(100 000) — character-counted, so no UTF-8 boundary can split — and
sets `truncated` when a clamp bites or the envelope version is newer
than this build understands. A missing or malformed envelope yields an
empty, truncated readout rather than an error.
