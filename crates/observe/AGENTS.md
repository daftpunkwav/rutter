# observe/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md).

## Boundary

- Workspace dependency: `rutter-core` only.
- No async, no tokio, no filesystem, no HTTP client.
- `assets`, `read`, `resolver`, `response`, and `snapshot_builder`
  stay private. Callers use the functions exported from `lib.rs`.

## Page scripts

- `src/assets/serializer.js` and `src/assets/reader.js` are plain ES.
  There is no build step. `assets.rs` embeds them with `include_str!`.
- A script does not throw on hostile page input.
- Storage helpers in `resolver.rs` return `{ unavailable: true }` when
  `localStorage` is missing. They do not throw.
- Serializer output is the envelope `response.rs` parses. Reader
  output is the envelope `read.rs` parses.
- A format change updates `docs/snapshot-format.md` or
  `docs/read-format.md` and the matching `.zh.md`.
- Behavior is pinned by `tests/js/` through `scripts/check_js.sh`.

## Snapshot builder

- Order: convert, cut depth, fold sibling runs, cull to the viewport,
  fit the character budget, render.
- Budgets and line rules match `docs/snapshot-format.md`.
- `src/snapshot_builder/tests.rs` pins them with insta and proptest.
- A deliberate format change updates the golden files in that same
  change.

## Tests

Golden and property tests run with no browser.
