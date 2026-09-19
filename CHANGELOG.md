# Changelog

All notable changes to this project are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Dashboard access token derives from an OS-entropy source and is
  exchanged for an HttpOnly, SameSite=Strict cookie on first visit; it
  is no longer a weakly seeded hash persisted in localStorage.
- Session storage state (cookies plus localStorage) is written
  atomically and, on Unix, owner-only (0600).
- Engine downloads live on the pinned Chrome for Testing storage host,
  reject non-https artifact URLs, and cap the fetched and extracted
  byte sizes so a compromised manifest source cannot exhaust memory or
  disk.
- The MCP HTTP transport warns when bound to a non-loopback address.
- Policy verdicts are judged at the URL an action leads to:
  navigations evaluate their canonicalized target URL, and a page that
  will not answer fails closed to approval instead of judging on an
  `about:blank` placeholder.

### Fixed

- Closing an already-disposed browser context succeeds: Chrome answers
  with several error texts ("Failed to find context with id ..." among
  them), and all of them now fold into success.
- Session recovery asks the supervisor for the live engine on every
  restart (a cached dead engine broke all recovery) and no longer runs
  twice per restart.
- Dashboard WebSocket no longer loses events between replay and
  subscribe, resyncs from the event rings after broadcast lag, and the
  loopback Host check rejects prefix-spoofed names.
- Malformed inputs (absurd wait budgets, blank policy patterns,
  oversized approval windows) are rejected or clamped instead of
  panicking; browser targets that vanished on their own no longer
  wedge the page cap.
- Every serve exit path (client disconnect, Ctrl-C) shuts the
  supervised engine down; closing a session disposes its browser
  context; concurrent session and page counts are capped.

### Changed

- MSRV is 1.88 (locked rmcp 3.4 / time 0.3 require it); dependency
  features trimmed to their used surface; cargo-deny runs clean.
- Contracts updated to match behavior: wait_for budget clamp, actual
  close_session semantics, serve flags, blueprint sketches.
- Internal renames for accuracy (session `tabs_*` APIs are now
  `pages`/`select_page`/`close_page`); MCP tool names unchanged.
- Oversized modules split (dashboard auth/ws, MCP params; module tests
  into sibling files) and every source directory gained a README.

## [0.1.0] - 2026-09-18

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
