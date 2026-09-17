# Snapshot Specification

> Status: v1 · 2026-09-17 · Normative contract for `rutter-observe`.
>
> This document is the contract for the observation pipeline: the JSON
> the in-page serializer produces, how references are minted, the text
> rendering agents read, and the token-budget rules. `rutter-observe`
> implements exactly this; golden and property tests pin it.

## 1. Pipeline

```
injected serializer (page)       rutter-observe
  composed DOM  ──►  JSON envelope  ──►  Snapshot  ──►  YAML text
      serializer_script()         snapshot_from_response()   Display
```

The serializer script is an embedded asset of `rutter-observe`
(`observe::serializer_script()`). The caller evaluates it inside a page
via the engine's `evaluate` (no engine dependency in `rutter-observe`),
then converts the returned JSON with `observe::snapshot_from_response()`.

## 2. Serializer envelope

```json
{
  "version": 1,
  "truncated": false,
  "viewport": { "width": 1280, "height": 720 },
  "scroll": { "x": 0, "y": 0 },
  "root": { "...node...": "see §3" }
}
```

- `version` — serializer format version, currently `1`.
- `truncated` — the serializer hit its own node guard (50 000 nodes)
  and omitted part of the composed DOM.
- `viewport` / `scroll` — CSS pixels of the page's viewport and scroll
  position. The budget consumes only `viewport`: node rects are
  viewport-relative (§3), so scroll offsets play no part in
  classification and `scroll` is reserved for future consumers
  (screencast framing in milestone M2). If the viewport is absent or
  malformed the converter treats it as unbounded (no viewport-first
  folding) and relies on the remaining budgets.
- `root` — one node for the document root (role `root`).

The serializer walks the composed DOM including open shadow roots and
slot assignments. It omits elements that are not visible (empty
bounding box or `visibility: hidden`) — hidden elements are never
guessed at (blueprint §7.2, honesty rule). The serializer never throws:
every per-node computation is guarded, and on failure the node degrades
to role `generic` with no name.

## 3. Node shape

| Field      | Type               | Meaning                                             |
|------------|--------------------|-----------------------------------------------------|
| `role`     | string             | ARIA role: explicit `role` attribute first, then the implicit role of the tag (link, button, textbox, heading, list, listitem, image, …), else `generic`. |
| `name`     | string, optional   | Accessible name, computed as: `aria-label` → `alt` (image) → associated `<label>` (form controls) → `placeholder` (text fields) → `title` → visible text, whitespace-collapsed, capped at 120 characters. |
| `value`    | string, optional   | Current value of form controls, capped at 200 characters. |
| `ref`      | string, optional   | Stable handle (§4); present on actionable, enabled elements. |
| `checked`  | bool, optional     | Checkbox/radio state.                               |
| `disabled` | bool               | `disabled` attribute or `aria-disabled="true"`; default false. |
| `rect`     | object, optional   | `{x, y, width, height}` in CSS pixels, integers, viewport-relative. |
| `children` | array, optional    | Child nodes in tree order. A host with an open shadow root reports only its shadow tree; slotted light-DOM nodes appear inside their slot (flattened), and unrendered light children are omitted. |

Unknown fields are ignored by the converter. Malformed values degrade
to defaults (missing role → `generic`, wrong types → field absent);
overlong strings are clamped by the converter (roles and refs to 64
characters, names and values to 200) and set `truncated`. The converter
never fails on hostile page data (blueprint §8.4).

Skipped elements: `script`, `style`, `noscript`, `template`, `head`,
`meta`, `link`, `title`, `br`, SVG internals (an `svg` element is
reported as a single node without children).

## 4. Reference minting (v1)

- The serializer keeps a per-page store on `window`
  (`__rutterRefStore`: a `WeakMap<Element, string>` plus an integer
  counter). An actionable element receives `e<N>` the first time it is
  observed and keeps it for later snapshots of the same page.
- Actionable roles v1: `button`, `link`, `textbox`, `searchbox`,
  `checkbox`, `radio`, `combobox`, `listbox`, `option`, `menuitem`,
  `tab`, `slider`, `spinbutton`, `switch`, `treeitem`.
- After a navigation the counter resets; old references no longer match
  any element and resolve to `ActionError::ReferenceExpired`.

## 5. Text rendering (YAML style)

One node per line, children indented two spaces per level:

```
- button "Sign in" [checked] [ref=e17]
```

Suffixes render in this order: `"name"` (double quotes inside names
escaped as `\"`), `[checked]` (only when true), `[disabled]` (only when
true), `[ref=eN]`, `× N` (folded-subtree count, §6). A folded summary
line renders as `- listitem × 20` (role of the folded items, their
count as `× N`). This format is implemented by `Snapshot`'s `Display`
in `rutter-core` and pinned by its unit tests.

## 6. Token budget (v1)

Budget is measured in rendered characters of the YAML text; the default
budget is 20 000 characters per snapshot. Rules apply in order; every
budget-induced change sets `Snapshot::truncated = true`.

1. **Depth budget.** Nodes deeper than 48 levels below the root are cut
   (summary line `- generic × N more` at the cut point is not rendered;
   the children are simply absent).
2. **Sibling folding.** A run of ≥ 8 consecutive same-role siblings
   with no name is folded into one summary node (role kept, `× N`
   set). Runs shorter than 8 pass through unchanged.
3. **Viewport-first culling.** Node rects are viewport-relative; any
   subtree entirely outside the band `[-200 px, viewport height + 200 px]`
   on the y axis whose rendered size exceeds 400 characters is replaced
   by a summary node of its root (`× N` counts the folded children),
   regardless of the remaining budget.
4. **Hard budget.** The remaining budget is spent in document order:
   children keep their full text while it fits; a child that no longer
   fits shrinks recursively; a leaf or a line that does not fit at all
   is dropped. Every budget-induced fold or drop sets `truncated`.
5. **Safety caps** (hostile-input limits, independent of budget):
   depth 512, 100 000 nodes — unchanged from v0; tripping them sets
   `truncated` and degrades, never panics.

`snapshot_from_response(url, json)` never panics and never returns an
error; hostile input degrades (§2, §3).
