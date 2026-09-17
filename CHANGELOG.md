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
- MCP server over stdio (`rutter serve`) on rmcp: the full TOOL_SPEC
  tool surface — navigate, history, snapshot, screenshot, click, hover,
  type, press_key, select_option, scroll, wait_for, tabs, cookies,
  close_session — with three-phase auto-wait and snapshot-after-action
  semantics.
- Typed event backbone (bus, per-session ring buffers, replay) and
  session orchestration (`rutter-session`).
- End-to-end acceptance tests driving `rutter serve` over stdio with an
  MCP client against the real engine.
- Policy engine (`rutter-policy`): TOML rules (action class x URL
  pattern -> verdict), conservative defaults, approval broker with
  configurable windows, and dashboard decision submission.
- Storage state per session (cookies plus localStorage), persisted on
  change and replayed automatically when the supervisor replaces a dead
  engine, with an `EngineRestarted` event per session.
- Supervision dashboard: localhost web server with token auth, Host
  validation, WebSocket event replay + live stream, approval controls,
  and an on-demand JPEG screencast live view (frames stream only while
  a viewer watches; the capture restarts across navigations).
