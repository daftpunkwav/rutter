# Changelog

All notable changes to this project are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- The tool catalog described the wrong failure vocabulary. An `isError`
  result renders the *session* error verbatim, so an engine-layer
  failure — the navigation that could not be sent, a dead engine, a
  page cap — reported the engine's own message and hint, while the
  catalog claimed every failure speaks `ActionError` and pointed
  agents at a transport classification (`DnsFailed`, an HTTP status)
  that only ever reached them through the `ActionFailed` event. The
  catalog now names both shapes, states that the closed `ActionError`
  taxonomy is what the event backbone carries, and pins the part that
  holds for every failure — message first, a non-empty `hint: ` line
  on its own — with a test over the whole session-error surface.
- A storage-state file carrying only one of `cookies` / `origins` no
  longer reads back as an empty state, which silently logged the
  session out. Both keys default on read, so a file written by a build
  that knew only one of them keeps the half it has. A file whose
  values are of the wrong type still reads as empty, unchanged.
- The streamable HTTP transport now states on *every* bind that it
  carries no authentication, not only on a non-loopback one: a loopback
  bind is exactly as unauthenticated as a LAN bind, and loopback is
  the default. `rutter serve --http --help` and the README say so too.
- A failed `Settings::resolve` printed its hint with a `-` where the
  other two CLI hint sites print `—`; the separator is the same on all
  three now.

- An approval grant can no longer be forged cross-site from a page on
  another loopback port. The dashboard's `Origin` check matched the host
  name alone, and a browser's `SameSite=Strict` is a *site* check — a
  site spans every port on a host — so a page served from
  `http://127.0.0.1:<any other port>` was same-site, carried the token
  cookie, and passed the gate. It could then `POST /api/decisions` a
  grant, or open the decision WebSocket and drive it itself. The origin
  must now name the dashboard's own host *and* the port the listener
  bound.
- Every dashboard response now carries `Cache-Control: no-store`,
  `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`,
  `X-Frame-Options: DENY`, and `Content-Security-Policy: frame-ancestors
  'none'`, stamped by one middleware so a route added later inherits
  them. The token-bearing first-visit URL no longer settles in a browser
  cache or a `Referer`, and the approval UI can no longer be framed with
  an overlay on top of its Grant button.
- Host-scoped policy rules can no longer be slipped past with a
  trailing root dot: `https://bank.example./` canonicalizes to
  `https://bank.example/` before judgment, the same host the DNS layer
  resolves it to.
- A granted action no longer executes against a page that moved during
  the approval window: after a grant (navigations excepted — their
  target rides in the effect) the page URL is re-read and re-judged, so
  the grant never follows a page the human was not shown.
- `read` no longer reports a structurally broken reader envelope as
  "empty and complete": a missing or non-string `title`/`markdown`
  field sets `truncated` like a non-object response does.
- Mixed inline content inside a container renders as one paragraph in
  `read` output (`<div>Hello <b>world</b> again</div>` is one sentence,
  not three); inline images follow the same `src`-only rule as
  block-level ones.
- A screencast request for a session with no open page answers
  `screencast-ack` with `started: false` and the documented reason
  (`no open page to observe`) instead of a bare ack with no frames and
  no explanation; the dashboard surfaces the reason in the timeline.
- The dashboard renders the known sessions in its header and offers
  them as suggestions for the live-view session field; the list was a
  clear-only no-op before.
- A dashboard reconnect rebuilds the known-session set from the replay:
  a session closed while the socket was down is absent from the replay
  (its ring is dropped on close), and its chip used to linger until a
  manual reload.
- `scroll` and `set_viewport` rejections land on one `invalid_params`
  path with the hint contract intact for negatives as well as zero
  (the unsigned schemas used to let rmcp's deserializer answer
  negatives with hint-less error text).
- `close_session` is terminal for the connection: a second call fails
  with `invalid_params` naming the closed session instead of
  answering success twice.
- A failing `--policy` file is a regular CLI error with its hint and
  the shared exit code, where it used to exit 2 with no hint.
- The snapshot's ref-store failures (a page tampering with
  `__rutterRefStore`, or no store at all) set `truncated` instead of
  silently dropping every element reference.
- Policy patterns with an authority wildcard that no later `/` anchors
  (`https://bank.example*`, which also matches
  `bank.example.evil.com`) are rejected at load time; anchored
  spellings and bare catch-alls are unchanged.

- Headed mode launches on Windows: rutter spawns the browser itself and
  resolves the debugging endpoint by polling the port, so both
  launcher-style executables (Edge) and full Chrome start where
  chromiumoxide's stderr parsing failed. Headed
  windows are chromeless app surfaces with a fresh per-launch profile.
- Dropping a screencast viewer stops the capture even when the page
  paints no further frames: the forwarding task now watches the
  receiver directly instead of noticing the departure only on the next
  frame, so a viewer who leaves and returns is not locked out by
  "Screencast is already active".
- Dashboard access handoff no longer races between rutter processes on
  one machine: the access file is written only after the bind
  succeeded and is named by the port the server actually owns
  (`<cache-dir>/dashboard-access-<port>.url`), which also fixes the
  URL reported under `--dashboard 0`.
- `press_key` now sends full key events: physical codes and Windows
  virtual key codes for known keys (letters, digits, punctuation,
  `Enter`, arrows, function keys, …) plus the text that makes
  printable keys land in focused inputs. Sites that ignored the
  previous name-only events respond correctly now.

### Changed

- Dashboard endpoints additionally reject requests whose `Origin`
  header names a non-loopback host, matched exactly like the `Host`
  check; requests without an `Origin` header are unaffected.
- `scroll`'s `amount` and `set_viewport`'s `width`/`height` are signed
  integers in the tool schemas (`i64`), so out-of-range values reach
  rutter's own validation instead of failing deserialization.
- The dashboard's gap refill after a broadcast lag filters inside the
  ring (`replay_after`), so envelopes the client already received are
  no longer cloned only to be dropped.

### Added

- CI quality gates now also compile and test the workspace on macOS,
  which the release archives ship for.

- Network visibility: finished page requests (method, URL, status or
  failure, resource type) are recorded per page, and the new
  `network_requests` MCP tool lists the active page's entries, oldest
  first.
- Viewport control: the new `set_viewport` MCP tool resizes the active
  page in CSS pixels so screenshots, snapshots, and layouts match the
  size an agent wants to work with.
- File uploads: the new `upload_file` MCP tool sets the files of a file
  input element through the action pipeline — auto-wait, policy
  judgment (`file_upload` class), and a fresh snapshot — with clear
  refusals for non-file inputs, single-file inputs given several paths,
  and paths that do not exist.
- Explicit tab opening: the new `tabs_open` MCP tool opens a page in
  the session's context, makes it active, and optionally navigates it
  through the same policy judgment as a `navigate` action.
- Cookie reads: the new `get_cookies` MCP tool returns every cookie
  scoped to the session's browser context, complementing `set_cookies`;
  reading is observation and needs no approval.
- Dialog resilience: a page that opens `alert`, `confirm`, `prompt`,
  or `beforeunload` no longer wedges its session — the session
  dismisses every dialog immediately (prompts resolve to their dismiss
  values), records the dismissal as a `DialogAutoDismissed` event, and
  keeps serving tools while the page continues.
- Console visibility: page console output and uncaught exceptions are
  captured per page, and the new `console_messages` MCP tool returns
  the active page's entries, oldest first.
- Markdown readouts: the `read` MCP tool and the `rutter read <url>`
  CLI mode return a page's readable content as a markdown document —
  headings, paragraphs, lists, GFM tables, code fences, and links with
  absolute URLs — while site chrome and hidden content are omitted
  (docs/read-format.md).
- Tabs discovery and adoption: windows the engine opens without rutter
  — a `window.open` popup, a human's window — are reported by the
  engine, adopted into the session's context on `tabs_list`, and are
  visible and selectable like any other page under stable
  `target:…` ids.
- CI runs the e2e acceptance suites and gates workspace line coverage
  at 90 % (`cargo-llvm-cov`, engine suites and e2e included), with the
  engine-backed jobs sharing one composite setup action.
- Wire-tag pins for the event names the schema-less dashboard branches
  on (`session_started`, `session_closed`, `approval_requested`,
  `approval_resolved`), the event-side counterpart of the brief's wire
  pin.
- CI gates the dashboard and Electron-shell JavaScript with a syntax
  check on both OSes.

## [0.1.0] - 2026-09-21

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
- The MCP HTTP transport rejects browser-originated requests (Origin
  validation) and refuses non-loopback binds unless `--allow-remote`
  is passed.
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
- Actions embedded in event payloads serialize with the event
  vocabulary's `type` tag (snake_case), matching what the dashboard
  frontend reads.
- Closing a session drops its event history with it: replay consumers
  only read open sessions, so a server cycling through session ids no
  longer grows the backbone's ring map without bound.

### Changed

- MSRV is 1.88 (locked rmcp 3.4 / time 0.3 require it); dependency
  features trimmed to their used surface; cargo-deny runs clean.
- Contracts updated to match behavior: wait_for budget clamp, actual
  close_session semantics, serve flags, blueprint sketches.
- `tabs_close` rejects an unknown page id with `invalid_params`,
  matching `tabs_select`.
- Internal renames for accuracy (session `tabs_*` APIs are now
  `pages`/`select_page`/`close_page`); MCP tool names unchanged.
- Oversized modules split (dashboard auth/ws, MCP params; module tests
  into sibling files) and source directories gained READMEs.

### Added

- Rust workspace skeleton with `rutter-core`, `rutter-engine`,
  `rutter-observe`, and the `rutter` CLI crate.
- Quality-gate scripts (header gate, encoding gate), a CI workflow, and
  supply-chain configuration for cargo-deny.
- Snapshot specification (`docs/snapshot-format.md`) and a snapshot
  builder with depth budget, sibling folding, viewport-first culling,
  and a hard character budget.
- Engine binary downloader for Chrome for Testing products (manifest,
  cache, retries) with system-browser discovery for headed runs.
- Engine supervisor: heartbeat health probes, capped-backoff restarts,
  and a sliding-window circuit breaker.
- CDP engine backend on chromiumoxide with an `#[ignore]`d integration
  suite, wired into `rutter open` (snapshot to stdout) and browse mode.
- MCP server over stdio (`rutter serve`) on rmcp: the full TOOL_SPEC
  tool surface — navigate, back, forward, reload, snapshot, screenshot,
  click, hover, type, press_key, select_option, scroll, wait_for,
  tabs_list, tabs_select, tabs_close, set_cookies,
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
