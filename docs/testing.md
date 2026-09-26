# Testing

English | [中文](testing.zh.md)

Where verification lives, which tests pin which contract, and how to
run the suites.

## 1. Levels

| Level | Scope | Tooling |
|---|---|---|
| Unit | Pure crates (core, observe, policy, events) and private paths beside the code | plain `#[test]` |
| Golden | Snapshot builder output on fixture DOM trees | `insta` |
| Property | Culling/folding invariants; serializer never panics | `proptest` |
| Integration | Real headless shell: launch, navigate, act | per-crate `tests/`, `#[ignore]`d by default |
| E2E | Full client → rutter → engine round-trips (mcp, approval, http transports; open, read CLI modes) | workspace `tests/` package |
| Benchmark | 20-site fixed corpus: navigate+snapshot success rate | harness in `scripts/` |

## 2. What pins which contract

- [Tool catalog](tool-catalog.md) semantics are pinned by
  `rutter-mcp` unit tests (result conventions, error mapping, schema
  details) and by the e2e suites in `tests/`.
- [Snapshot format](snapshot-format.md) is pinned by golden fixtures
  and property tests in `rutter-observe` and `rutter-core`
  (`Display` rendering).
- Numeric defaults — auto-wait budgets, page/session caps, approval
  window, restart policy — are asserted in the owning crate's unit
  tests (for example
  [`SessionConfig::default`](../crates/session/src/config.rs) matches
  the tool catalog).
- Engine supervision behavior (backoff, breaker, heartbeat recovery)
  uses injected clocks and fast, deterministic policy overrides in
  `rutter-engine` tests.

## 3. Running

```sh
cargo test --all            # unit + golden + property suites
```

Integration and acceptance suites drive the real engine and are
`#[ignore]`d locally; run them explicitly (they reuse the cache
`rutter open` fills):

```sh
cargo test -p rutter-engine-cdp --tests -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-integration-tests --test open_e2e -- --ignored
cargo test -p rutter-integration-tests --test read_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

Corpus harnesses:

```sh
scripts/smoke_open.sh    # 10 real sites through `rutter open`, pass/fail summary
scripts/benchmark.sh     # 20-site navigate+snapshot benchmark, 90 % success bar
```

## 4. Quality gates

CI runs on pushes to `main` and on every pull request. The quality job
runs the local checks of
[`CONTRIBUTING.md`](../CONTRIBUTING.md) plus a rustdoc gate:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo doc --locked --no-deps
cargo test --all
bash scripts/check_headers.sh   # every source file opens with a truthful header
bash scripts/check_encoding.sh  # tracked text files: UTF-8, LF, no BOM
```

Plus `cargo-deny` (`deny.toml`: advisories, licenses, bans). The
`#[ignore]`d suites run in CI too: an integration job executes every
`rutter-engine-cdp` suite (`--tests`, screencast included) and an e2e
job runs the full `rutter-integration-tests` set, both on ubuntu and
windows against the real engine; a coverage job (ubuntu) measures the
whole workspace — engine suites and e2e included — and fails below
90 % line coverage (§5). Pull-request runs are canceled when a newer
push supersedes them (concurrency group), every job carries a
`timeout-minutes` bound, and the three engine jobs share one composite
action ([engine-setup](../.github/actions/engine-setup/action.yml)) for the
engine cache and Linux engine dependencies.
Release archives are cut by cargo-dist (`cargo dist build`, configured
in the `[workspace.metadata.dist]` section of the root `Cargo.toml`); no release workflow
is checked in. The corpus harnesses are manual runs, not CI gates;
`scripts/benchmark.sh` reports a ≥ 90 % navigate+snapshot success
bar.

## 5. Coverage

Line coverage is measured with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) over the
whole workspace, including the `#[ignore]`d engine suites, in one
instrumented run (build, tests, and report must share one invocation,
or the instrumented `rutter.exe` is removed before the e2e children
inherit it):

```sh
export RUTTER_BIN="$PWD/target/llvm-cov-target/debug/rutter"
cargo llvm-cov --locked --workspace --bins --tests \
    --summary-only --output-path target/coverage-full.txt \
    -- --include-ignored --test-threads=1
```

The full run (including `open_e2e`, `read_e2e`, `mcp_e2e`,
`approval_e2e`, and `http_e2e` driving the instrumented binary)
measures **92.79 % lines**
(6356 of 6850 source lines; test targets themselves are excluded from
the count) as of 2026-09-27. The remaining lines are headed/browse-mode
paths (`crates/cli/src/browse.rs` accounts for all 15 of its lines),
network-mid failure paths of the engine downloader, defensive branches
that need a wedged browser, and some WebSocket reconnect paths.

CI gates on this number: the coverage job runs the same invocation
(lcov output instead of a summary) and fails below 90 % lines via
[`scripts/check_coverage.sh`](../scripts/check_coverage.sh), so the CI
figure and the local figure above are one method and one number.
