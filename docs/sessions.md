# Sessions

English | [中文](sessions.zh.md)

The session model of
[`rutter-session`](../crates/session/src/manager.rs): what one client
owns, how an action travels through it, how login state survives, and
what happens when the engine dies under it.

## 1. Sessions and pages

- One MCP connection is one session; one session holds exactly one
  browser context. Caps protect the shared engine:
  [`SessionConfig`](../crates/session/src/config.rs) defaults to 8
  sessions per server and 8 pages per session.
- The session tracks its open pages (`PageSlot`: id, last known URL,
  handle, active flag). Exactly one page is active; page-scoped tools
  target it. `tabs_select` switches; `tabs_close` promotes the first
  remaining page; `navigate` opens the first page when none exists.
- Contexts are created with
  [`ContextConfig`](../crates/engine/src/config.rs): page cap,
  navigation deadline (30 s), screenshot rate cap (500 ms).

## 2. Action execution path

`Session::execute(action, origin)` is the one path every action
takes — MCP tools and dashboard control alike:

```
resolve active page
  → policy gate (agent origin only; policy.md)
  → ActionRequested event
  → executor: auto-wait → act → settle (tool-catalog.md §3)
  → ActionCompleted | ActionFailed event
  → refresh tracked page URL
  → persist storage state if it changed
```

- The [origin](glossary.md) decides attribution and whether approval
  rules apply; both origins share the execution path and event
  timeline.
- Engine failures observed on this path are mapped into the
  [`ActionError`](../crates/core/src/error.rs) taxonomy for event
  payloads, so consumers read one vocabulary of failures.

## 3. Storage state

[`StorageState`](../crates/session/src/storage.rs) is `{ cookies,
origins }` — context cookies plus localStorage per origin:

- **Captured** from the context and the open pages after every action
  and after `set_cookies`.
- **Persisted on change** to
  `<cache-root>/sessions/<session-id>.storage.json`. The write is
  atomic: the JSON lands in a sibling temporary file that replaces the
  real one in one rename, owner-only on Unix. An unchanged state is
  not rewritten; a failed write forces a rewrite on the next capture.
- **Reloaded** by explicit `save_storage`/`load_storage` calls and by
  recovery. This is how login state survives engine restarts and
  process restarts.

## 4. Recovery

When the supervisor replaces a dead engine
([engine supervision](engine-supervision.md#3-process-supervision)),
the manager's recovery task rebuilds every session
([`Session::recover`](../crates/session/src/session/mod.rs)):

1. Swap in a fresh context.
2. Replay cookies from the last captured storage state.
3. Re-open each tracked page at its last URL and restore that page's
   localStorage; the active page is restored by position, so exactly
   one page comes back active. Blank pages are not restored.
4. Publish `EngineRestarted`, so the agent knows time passed and takes
   a fresh snapshot.

Closing a session closes its context and drops its event ring; open
sessions die with a manager shutdown, but their storage states remain
on disk and reload on the next start.
