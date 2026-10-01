# session/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). The session model lives in
[../../docs/sessions.md](../../docs/sessions.md).

## Boundary

- Workspace dependencies: `rutter-core`, `rutter-engine`,
  `rutter-events`, `rutter-observe`, `rutter-policy`.
- This is the only crate where engine, observation, policy, and events
  meet.
- Engines are used through `rutter-engine` traits. No CDP types.
- `actions`, `pages`, `resolve`, `wait`, `audit`, and `observations`
  stay private. `mock` is `#[cfg(test)]`.
- Consumers other than `crates/cli` and dev-dependency tests take
  engine types from this crate's re-exports.

## Execute

- Agent actions go through `Session::execute`: resolve the active
  page, policy gate, `ActionRequested`, auto-wait then act then
  settle, `ActionCompleted` or `ActionFailed`, refresh the tracked
  URL, persist storage when it changed.
- The refresh runs after every attempt, successful or not.
- The policy gate runs only for `Origin::Agent`. `Origin::Human`
  skips approval and is still recorded.
- `actions/` runs one action and its auto-wait. It does not evaluate
  policy and does not park approvals.
- Auto-wait order is visible, then stable, then enabled. A phase
  budget restarts only when the phase advances.
- `poll_until` clamps its budget at 600 s.
- After a grant, an effect that carries a target URL proceeds. Any
  other grant re-reads the page. A page that moved goes through
  `RuleSet::review` again.
- Read-only probes (`screencast`, `active_page_id`,
  `console_messages`, `network_requests`) do not open a page.
  `screencast` on an empty session returns `NoOpenPage`.
  `execute`, `snapshot`, `read`, and `screenshot` may open the first
  page through `ensure_page`.

## Manager

- One supervisor per process. The manager holds the `ApprovalBroker`
  it was given and does not construct another.
- The engine starts on the first request that needs it.
- A `SessionId` that is empty, longer than 200 bytes, or contains a
  path separator or NUL is refused before any engine launch.
- `max_sessions` and `max_pages` are enforced. A refused session is
  not inserted.
- The engine slot, the startup lock, and the session map are three
  locks.
- `close_session` of an unknown id is a no-op. The session's `close`
  runs outside the session-map lock, tolerates context failure, then
  publishes `SessionClosed` and calls `Backbone::forget`.
- `shutdown` leaves storage files on disk. A new session starts empty
  until `load_storage`.
- Take `Supervisor::engine` on each use. Do not cache the handle.
- On engine replacement, replay storage state, restore pages, and
  publish `EngineRestarted`.
- `Session::pages` adopts pages in this session's context that rutter
  did not open. It does not adopt another context.
- The registry keeps exactly one active page. `select_page` publishes
  `PageActivated`.

## Storage

- Storage state is context cookies plus localStorage of every open
  page, written atomically and owner-only on Unix.
- A failed cookie read does not publish a state and does not replace
  the file with an empty state.
- `SessionConfig::context_config` stays `pub(crate)`.

## Tests

Browser-free tests use `src/mock.rs` or the doubles in
`tests/common/`. Public flows live in `tests/`. Private behavior
stays in `src/`.
