# assets/ — in-page scripts

English | [中文](README.zh.md)

`serializer.js` runs inside the page (injected via `evaluate`) and
serializes the accessibility tree to the JSON envelope
`response.rs` parses: role/name/ref/rect nodes, viewport report, and
the truncation flag. It mints refs into a per-page store
(SNAPSHOT_SPEC §4); a navigation invalidates them.

Constraints: no build step, plain ES, must not throw on hostile pages
(storage access on opaque origins returns `{unavailable: true}`, for
example). Output format changes are SNAPSHOT_SPEC contract changes.
