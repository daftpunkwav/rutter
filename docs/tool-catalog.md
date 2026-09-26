# MCP Tool Catalog

English | [中文](tool-catalog.zh.md)

The contract for the `rutter-mcp` tool surface: transports, result
conventions, auto-wait semantics, and every tool with its parameters.
`rutter-mcp` implements exactly this; unit and end-to-end tests pin it,
and [`crates/core/src/action.rs`](../crates/core/src/action.rs) carries
the payload vocabulary.

## 1. Transport and sessions

- Transport: stdio (the default of `rutter serve`) or streamable HTTP
  via `rutter serve --http ADDR`. The dashboard attaches through
  `serve --dashboard PORT` and the rule set through `serve --policy
  FILE`. A non-loopback `--http` bind is refused unless confirmed with
  `serve --allow-remote` — the transport has no authentication.
- One MCP client connection is one Session. The `SessionId` is minted
  at connection start ([glossary](glossary.md#identifiers)) and
  reported in the `SessionStarted` event; it appears in error payloads
  that reference a session.
- The engine spawns lazily: `serve` starts with no engine; the first
  tool call that needs a page launches it (supervised;
  [engine supervision](engine-supervision.md)) and creates the
  session's context with the configured caps. Startup of the MCP
  server itself performs no I/O.
- Every session has exactly one context. The active page is the target
  of all page-scoped tools; `navigate` opens the first page when none
  exists.
- Transport hardening: the streamable HTTP transport rejects
  browser-originated requests (empty `Origin` allowlist) and validates
  `Host` headers against a loopback allowlist (DNS rebinding).

## 2. Result conventions

- Tools return MCP content blocks; text results use one `text` block.
- **Action failures are results, not protocol errors.** A failed
  action returns `isError: true` whose text carries the `ActionError`
  message plus its actionable hint on a second line (`hint: …`).
  Agents read them as data.
- Protocol-level failures (unknown session, engine dead beyond the
  breaker) are JSON-RPC errors with code `-32000` and the same
  message-plus-hint text.
- Snapshots are rendered in the YAML text form of
  [snapshot format §5](snapshot-format.md#5-text-rendering-yaml-style),
  under the 20 000-character budget; the text ends with a marker line
  `… truncated` when `Snapshot::truncated` is set.
- Every mutating tool returns a fresh snapshot by default, unless
  stated otherwise below.

The complete failure vocabulary is
[`ActionError`](../crates/core/src/error.rs); every variant carries an
English hint an agent can act on:

| Variant | Meaning |
|---|---|
| `NavigationFailed { url, cause }` | The page could not be loaded; `cause` classifies the transport failure (`TimedOut`, `ConnectionFailed`, `DnsFailed`, `TlsFailed`, `Aborted`, `Http { status }`) |
| `ReferenceExpired { reference }` | The element the reference points to no longer exists |
| `NotInteractable { reference, reason }` | The element exists but cannot receive the action |
| `TimedOut { phase, elapsed }` | An auto-wait phase did not complete in time |
| `ApprovalDenied { reference }` | A human approver rejected the action |
| `ApprovalTimedOut { waited }` | No approval decision arrived within the window |
| `EngineTerminated { session }` | The engine died; rutter is recovering it |
| `Internal { detail }` | A bug was contained at the action boundary — never a silent pass |

## 3. Auto-wait semantics

Every mutating tool that carries a `reference` runs the three-phase
auto-wait before acting; phases poll the live page:

| Phase | Pass condition | Budget (default) |
|---|---|---|
| `visible` | resolved element has a non-empty box | 5 s |
| `stable` | box unchanged across two samples ~80 ms apart | 5 s |
| `enabled` | element is not `disabled` and not `aria-disabled` | 5 s |

- Before `visible`, the resolver scrolls the element into view.
- Each phase's budget restarts only when the phase advances, so a
  flickering page cannot stretch the wait indefinitely.
- After the action, the executor settles for 250 ms (lets same-tick
  navigations start), then takes the fresh snapshot.
- Budget exhaustion maps to `ActionError::TimedOut` with the phase
  that failed; a reference that no longer resolves maps to
  `ActionError::ReferenceExpired`.

Every action carries an [origin](glossary.md): agent-origin actions
pass through the [policy gate](policy.md#3-the-judgment-url);
human-origin actions bypass approval and are recorded on the same
timeline.

## 4. Tools

Nineteen tools. Parameter types: `reference` is a snapshot handle
(`e17`); `direction` is one of `up|down|left|right`; durations are
milliseconds.

### navigate
`{ url: string }` → snapshot. Navigates the active page (opening the
first page when needed). Navigation uses the context's navigation
timeout; failure → `ActionError::NavigationFailed`.

### back / forward / reload
`{}` → snapshot. History operations on the active page.

### snapshot
`{}` → snapshot. No auto-wait; renders the current composed DOM.

### read
`{}` → text block, the page's readable content as a markdown document:
title line, then headings, paragraphs, lists, GFM tables, code fences,
and links with absolute URLs. No auto-wait; extraction rules and
guards are specified in [read format](read-format.md). Site chrome and
hidden content are omitted; the text ends with a `… truncated` marker
when a guard cropped the document.

### screenshot
`{}` → `image` content block (PNG, base64). Honors the context's
screenshot rate cap; capture errors map to `ActionError::Internal`.

### click
`{ reference: string }` → snapshot. Auto-wait, then mouse
pressed+released at the resolved box center.

### hover
`{ reference: string }` → snapshot. Auto-wait, then a mouse move to
the resolved box center.

### type
`{ reference: string, text: string }` → snapshot. Auto-wait, focuses
the element, inserts `text` as literal characters, then returns a
snapshot.

### press_key
`{ key: string }` → snapshot. Key names follow engine notation
(`a`, `Enter`, `Tab`). v1 limitation: key events carry the key name
but no virtual key codes; sites that ignore code-less key events are a
documented gap.

### select_option
`{ reference: string, values: string[] }` → snapshot. Auto-wait, then
selects options whose `value` is in `values` and dispatches
`input`/`change`. Fails with `NotInteractable` on non-select elements.

### scroll
`{ direction: up|down|left|right, amount: number, reference?: string }`
→ snapshot. Scrolls the page (no `reference`) or the resolved container
by `amount` pixels; `amount` ≤ 0 is `invalid_params`.

### wait_for
`{ text: string, timeout_ms?: number }` → snapshot. Polls the page's
text content until `text` appears (default budget 10 000 ms);
exhaustion → `ActionError::TimedOut`. Requested budgets above
600 000 ms are clamped to that server-side maximum
([`MAX_POLL_BUDGET`](../crates/session/src/wait.rs)); the timeout
error reports the effective budget.

### tabs_list
`{}` → text block, one line per page: `<page-id> <url>`; the active
page is suffixed ` (active)`. Listing first reconciles with the
engine: windows that appeared without rutter opening them — a
`target=_blank`/`window.open` popup, a window the human opened — are
reported under stable `target:…` ids and become selectable. Such ids
select and close like any other; closing one that the session's own
context owns closes the window, while an engine-owned surface (the app
window) is only untracked.

### tabs_select
`{ page_id: string }` → snapshot. Unknown id → `invalid_params`.

### tabs_close
`{ page_id: string }` → text confirmation. Unknown id →
`invalid_params`. Closing the active page makes the first remaining
page active; closing the last page is allowed — the next `navigate`
opens a new one.

### set_cookies
`{ cookies: [{ name, value, domain, path?, secure?, http_only?,
same_site? }] }` → text confirmation. Cookies apply to the session's
context (context isolation, [glossary](glossary.md)). `same_site` is
`strict|lax|none`; persistence across restarts works through storage
state ([sessions](sessions.md#3-storage-state)).

### close_session
`{}` → text confirmation. Closes the session's pages and context and
drops engine references. The call is terminal for the connection:
later tool calls on the same connection fail with `invalid_params`
naming the closed session. The engine keeps serving other sessions and
shuts down when the server exits (client disconnect or Ctrl-C), not
when a session closes.

## 5. Event backbone (not a tool)

The [event backbone](events.md) (typed events, bus, per-session rings,
replay) runs inside the server. MCP has no notifications or event
tools; the dashboard consumes replay over WebSocket.

## 6. Acceptance suites

Three e2e task classes pass against the real engine
(`#[ignore]`-gated integration tests driving the built binary over
stdio; see [testing](testing.md)):

1. **Read**: `navigate` → `snapshot` shows the page's heading and
   actionable refs.
2. **Interact**: on a synthetic page, `click` a button that mutates the
   DOM → the returned snapshot reflects the change.
3. **Form**: `type` into an input (and `select_option`) → the snapshot
   value field reflects the input; `screenshot` returns non-empty PNG
   bytes.
