# tests/ agent rules

The suite table lives in [README.md](README.md). How to run it lives
in [../docs/testing.md](../docs/testing.md). A `README.md` change
includes `README.zh.md`.

## Placement

- This package (`rutter-integration-tests`) is cross-crate acceptance.
  Each Rust test spawns the `rutter` binary.
- One crate's public API stays in that crate's `tests/`. Private
  behavior stays beside the code.
- `js/` is not a Cargo target.

## Engine suites

- Tests that need the engine are `#[ignore]`d.
- The binary is `CARGO_BIN_EXE_rutter`, else `RUTTER_BIN`, else
  `target/debug`.
- `cargo test --all` does not drive a browser. Do not remove
  `#[ignore]` from an engine test.
- Do not skip or weaken a test that `cargo test --all` already runs.

## JavaScript suite

- `scripts/check_js.sh` runs `node --test tests/js/*.test.mjs`.
- The tests execute the shipped files against `js/dom.mjs`. No npm
  dependencies. Node 18 or newer.
- Without Node, or with Node older than 18, `check_js.sh` exits 0
  and prints `skipped`.
- A DOM member the shipped script uses is modeled in `dom.mjs`. An
  unmodeled member throws in the test that touches it.
