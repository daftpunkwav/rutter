# Changelog

All notable changes to this project are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Rust workspace skeleton with `rutter-core`, `rutter-engine`,
  `rutter-observe`, and the `rutter` CLI crate.
- Quality-gate scripts (header gate, encoding gate), a CI workflow, and
  supply-chain configuration for cargo-deny.
- Snapshot specification (`docs/SNAPSHOT_SPEC.md`) and a snapshot
  builder with depth budget, sibling folding, viewport-first culling,
  and a hard character budget.
- Engine binary downloader for Chrome for Testing products (manifest,
  cache, retries) with system-browser discovery for headed runs.
- Engine supervisor: heartbeat health probes, capped-backoff restarts,
  and a sliding-window circuit breaker.
- CDP engine backend on chromiumoxide with an `#[ignore]`d integration
  suite, wired into `rutter open` (snapshot to stdout) and browse mode.
