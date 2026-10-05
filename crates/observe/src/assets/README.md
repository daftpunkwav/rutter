# assets/ — in-page scripts

English | [中文](README.zh.md)

`serializer.js` runs inside the page (injected via `evaluate`) and
serializes the accessibility tree to the JSON envelope
`response.rs` parses: role/name/ref/rect nodes, viewport report, and
the truncation flag. It mints refs into a per-page store
(reference minting); a navigation invalidates them.

`reader.js` runs inside the page the same way and extracts the
readable content as the markdown envelope `read.rs` parses:
`{ version, truncated, title, markdown }` with site chrome and hidden
content omitted (docs/read-format.md).

`entropy.js` is not evaluated at snapshot time: the engine installs it
with `Page.addScriptToEvaluateOnNewDocument`, so it runs before the
document's own scripts. It binds the native `crypto.getRandomValues`
and locks it onto the global, which is what lets the serializer mint a
ref scope the page cannot choose (`lib.rs::entropy_capture_script`).

Constraints: no build step, plain ES, must not throw on hostile pages
(storage access on opaque origins returns `{unavailable: true}`, for
example). Output format changes are docs/snapshot-format.md and
docs/read-format.md contract changes.
