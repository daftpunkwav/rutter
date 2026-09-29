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
| `serializer.test.mjs` | `crates/observe/src/assets/serializer.js` — the ref sweep and its threshold, refs on actionable elements only, shadow-DOM traversal, the "never throws" guards |
| `reader.test.mjs` | `crates/observe/src/assets/reader.js` — colspan headers, cell-less leading rows, nested tables, pipe escaping, the character budget, site chrome, links, lists, quotes, fences |
| `app.test.mjs` | `frontend/src/app.js` — the silent decision ack, a screencast refusal that names its reason, the reconnect reset of the timeline and the session set, the reconnect backoff, approval de-duplication, decision-post failures |

The fixture models exactly the platform surface those three files
touch. A member they start using that `dom.mjs` does not model raises
a `TypeError` in the test that needs it, which is deliberate: a
silently missing member would make a behavioural test vacuous.
