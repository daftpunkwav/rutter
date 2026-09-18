# rutter-session/ — orchestration

The only place where engine, observation, policy, and events meet: one
`Session` per MCP client owns a browser context, executes typed actions
with three-phase auto-wait, evaluates policy and parks approvals,
persists storage state, and reopens its pages after an engine restart.

## Boundary

Speaks `core` types, drives engines only through `engine`'s traits,
reads snapshots through `observe`, decides through `policy`, and
publishes `events`. Nothing above it knows how an action actually
happens; nothing below it may call back into this crate.

## Key concepts

- `SessionManager` — one supervised engine per process, one session
  per client, `max_sessions`/`max_pages` caps, and the recovery task.
- `Session::execute` — policy gate → action → events → URL/storage
  refresh; runs after every attempt, successful or not.
- Storage state (cookies + localStorage) persists on change and is
  replayed automatically when the supervisor replaces a dead engine.
