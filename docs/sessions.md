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
- The tracked pages live in
  [`PageRegistry`](../crates/session/src/pages.rs): slots of
  (`id`, last known URL, handle, active flag), behind the registry's own
  lock. Exactly one page is active, and the registry — not a caller —
  upholds it. `tabs_select` switches; `tabs_close` promotes the first
  remaining page.
- **Pages rutter did not open join through discovery.** A `window.open`
  tab or a human's window exists in the engine with no slot;
  `tabs_list` adopts what the engine reports (never another session's
  context) before listing, so those windows are visible and selectable
  like any other.
- **Two lookups, deliberately different.** `ensure_page` opens the
  session's first page when there is none, which is what an action needs.
  A read-only lookup answers `None` instead, and that is what the
  dashboard's screencast uses: asking to watch must not create the thing
  watched, so a session with no page answers `SessionError::NoOpenPage`
  (docs/architecture.md: the dashboard executes no actions).
- Contexts are created with
  [`ContextConfig`](../crates/engine/src/config.rs): page cap,
  navigation deadline (30 s), screenshot rate cap (500 ms).
- The manager guards three unrelated things separately — the engine slot
  (read-mostly), the startup lock (held across the launch only), and the
  session map (bookkeeping only) — and recovery snapshots the session list
  before rebuilding anything, outside every lock. A single lock used to
  cover all three, so one slow launch, which can back off for up to a
  minute while the restart breaker drains, also blocked the dashboard from
  reading the event backbone and the session list.

## 2. Action execution path

`Session::execute(action, origin)` is the one path every action
takes. The MCP tools call it with `Origin::Agent`; the dashboard
submits approval decisions and never executes actions:

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

- **Captured** from the context and the page the action ran on, after
  every action and after `set_cookies`.
- **Persisted on change** to
  `<cache-root>/sessions/<session-id>.storage.json`. The write is
  atomic: the JSON lands in a sibling temporary file that replaces the
  real one in one rename, owner-only on Unix. An unchanged state is
  not rewritten; a failed write forces a rewrite on the next capture.
- **Reloaded** by recovery and by an explicit `load_storage`
  (`save_storage` only writes). This is how login state survives
  engine restarts.

## 4. Recovery

When the supervisor replaces a dead engine
([engine supervision](engine-supervision.md#3-process-supervision)),
the manager's recovery task rebuilds every session
([`Session::recover`](../crates/session/src/session/mod.rs)):

1. Gate the registry: read the tracked URLs out, and refuse lookups and
   new registrations until step 5 lands.
2. Swap in a fresh context.
3. Replay cookies from the last captured storage state.
4. Re-open each tracked page at its last URL and restore that page's
   localStorage; the active page is restored by position, so exactly
   one page comes back active. Blank pages are not restored.
5. Install the rebuilt list, reopen the registry, and publish
   `EngineRestarted`, so the agent knows time passed and takes a fresh
   snapshot.

The gate is why an action arriving mid-rebuild fails with `Terminated`
instead of quietly opening a page. Without it, a page registered between
the read-out and the write-back was overwritten in the registry while its
tab stayed alive in the new engine — untracked by `tabs_list` and
unaccounted by the page cap.

Closing a session closes its context and drops its event ring; open
sessions die with a manager shutdown. The storage file stays on disk,
but a new session starts from an empty state — nothing reads the old
file back.
