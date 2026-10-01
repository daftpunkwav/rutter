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
- `storage_dump_script` returns `{ unavailable: true }` when
  `localStorage` is missing or throws. `storage_restore_script`
  swallows `setItem` failures and returns `{ restored: N }`. Neither
  script throws out of the function.
- Serializer output is the envelope `response.rs` parses. Reader
  output is the envelope `read.rs` parses.
- A readout format change updates `docs/read-format.md` and
  `docs/read-format.zh.md` in the same change. A snapshot format
  change updates `docs/snapshot-format.md` and
  `docs/snapshot-format.zh.md` in their own docs commit, before the
  implementation commit.
- Behavior is pinned by `tests/js/` through `scripts/check_js.sh`.

## Snapshot builder

- Order: convert, cut depth, fold sibling runs, measure, then one
  `fit` walk that folds out-of-viewport subtrees and spends the
  character budget, then `into_snapshot_node`. Text rendering stays
  in `rutter-core`.
- Budgets and line rules match `docs/snapshot-format.md`.
- `src/snapshot_builder/tests.rs` pins them with insta and proptest.
- A deliberate format change updates the golden files in that same
  change.

## Tests

Golden and property tests run with no browser.
