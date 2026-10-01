# crates/ agent rules

The crate table lives in [README.md](README.md). Layering and the
cross-cutting invariants live in
[../docs/architecture.md](../docs/architecture.md). Check gates live in
[../CONTRIBUTING.md](../CONTRIBUTING.md).

## Dependencies

Workspace dependencies are exactly:

| Crate | May depend on |
|---|---|
| `rutter-core` | — |
| `rutter-engine` | `rutter-core` |
| `rutter-observe` | `rutter-core` |
| `rutter-engine-cdp` | `rutter-core`, `rutter-engine`, `rutter-observe` |
| `rutter-policy` | `rutter-core` |
| `rutter-events` | `rutter-core`, `rutter-policy` |
| `rutter-session` | `rutter-core`, `rutter-engine`, `rutter-events`, `rutter-observe`, `rutter-policy` |
| `rutter-mcp` | `rutter-core`, `rutter-session` |
| `rutter-dashboard` | `rutter-core`, `rutter-events`, `rutter-policy`, `rutter-session` |
| `rutter` (`crates/cli`) | `rutter-core`, `rutter-dashboard`, `rutter-engine`, `rutter-engine-cdp`, `rutter-mcp`, `rutter-observe`, `rutter-policy`, `rutter-session` |

- `rutter-mcp` and `rutter-dashboard` may name `rutter-engine` only
  from a dev-dependency. `rutter-mcp` may name `rutter-policy` only
  from a dev-dependency.
- `rutter-engine-cdp` is the only member that depends on
  `chromiumoxide`.
- Versions live in the root `Cargo.toml`
  `[workspace.dependencies]`. Member manifests use `workspace = true`.
- Edition, `rust-version`, `license`, and `publish` are workspace
  fields. `publish` stays `false`.
- A new crate is registered in `workspace.members`, keeps the table
  above, and is not empty.

## Production code

- Clippy denies `unwrap_used`, `expect_used`, and `panic`. The allow
  is `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]`
  on a crate root, on a module that contains tests, or on a test
  file. Keep the attribute `cfg(test)`. A non-test build denies those
  lints.
- `todo`, `unimplemented`, and `dbg` are denied, including in tests.
- `unsafe_code` is denied.
- Public items are documented.
- Every `*.rs` file opens with a `//!` header that states purpose and
  boundary.
- A wait that can finish on its own has a deadline. Ctrl-C, the MCP
  serve loop, and the dashboard serve loop wait for the process
  lifetime.
- A dead, restarting, or breaker-open engine returns `Terminated`.
- `ActionFailed` carries an `ActionError` with an English hint.
  `Internal` marks a bug. A tool result prints the `SessionError` and
  its hint. An engine failure on that path stays an `EngineError`.
- Hostile page input is truncated or folded. An unknown envelope
  version sets `truncated`. An unknown viewport disables
  viewport-first folding.
- Publishing an event does not wait on subscribers.
- Glossary names are exact. See
  [../docs/glossary.md](../docs/glossary.md).
- Out of scope: an in-tree rendering engine, stealth or
  anti-fingerprinting, a consumer browser UI, a cloud service.

## Tests and docs

- Private behavior stays in `src/` under `#[cfg(test)]`. One crate's
  public API stays in that crate's `tests/`. Cross-crate acceptance
  stays in the workspace `tests/` package.
- A test that drives a real engine is `#[ignore]`d. `cargo test --all`
  does not launch a browser.
- Do not delete, skip, or weaken a test to land a change.
- Numeric defaults are asserted in the owning crate.
- `docs/tool-catalog.md` and `docs/snapshot-format.md` change in their
  own docs commit, before the implementation commit. Each English doc
  change includes its `.zh.md` mirror. Each `README.md` change
  includes its `README.zh.md`.
- Comments may be Chinese or English.
- CI runs cargo-deny against `deny.toml` (advisories, licenses, bans).
  A new dependency passes that job.
