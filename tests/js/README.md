# tests/js/ — behavioural suite for the shipped JavaScript

English | [中文](README.zh.md)

The injected page scripts and the dashboard client carry logic a
string assertion cannot reach: the ref store's sweep, the markdown
table rules, the character budget, the reconnect reset. These tests
**run** the shipped files against a minimal DOM and assert what they
do. Run them with `bash scripts/check_js.sh` (or
`node --test tests/js/*.test.mjs`); CI runs the same script.

No dependencies: `node --test` and `node:vm` are Node builtins, and
the DOM is a fixture in [`dom.mjs`](dom.mjs). Node 18 or newer.

| File | Runs |
|---|---|
| `dom.mjs` | The fixture: a DOM small enough to read, plus a `ControllableWeakRef` so a test can decide when an element is collected |
| `serializer.test.mjs` | `crates/observe/src/assets/serializer.js` — the ref sweep and its threshold, refs on actionable elements only, shadow-DOM traversal, roles that name an `Object.prototype` member, the "never throws" guards |
| `reader.test.mjs` | `crates/observe/src/assets/reader.js` — colspan headers, cell-less leading rows, nested tables, pipe escaping, the character budget, site chrome, links, lists, quotes, fences |
| `app.test.mjs` | `frontend/src/app.js` — the silent decision ack, a screencast refusal that names its reason, the reconnect reset of the timeline and the session set, the reconnect backoff, approval de-duplication, decision-post failures, the catalog's fallback to the key |
| `scheme-gate.test.mjs` | `browser/scheme-gate.js` — the scheme gate both navigation entry points test: web URLs and `about:blank` pass, privileged schemes (`file:`, `javascript:`, `devtools:`, lookalike spellings) are refused |

The fixture models exactly the platform surface the page scripts and
the dashboard client touch, limits included. A member they start using
that `dom.mjs` does not model raises a `TypeError` in the test that
needs it, which is deliberate: a silently missing member would make a
behavioural test vacuous. The same goes the other way — `children`,
`rows`, and `cells` are array-like but **not** iterable, as the
platform's `HTMLCollection` is, so a walker that iterates one instead
of copying it first fails here rather than in a browser.
