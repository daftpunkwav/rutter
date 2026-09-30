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
- **The streamable HTTP transport carries no credentials of any
  kind.** A loopback bind is exactly as unauthenticated as a LAN one:
  the Origin and Host checks below keep *browsers* out, but any
  process that can open a TCP connection to the address — another
  program running as this user, anything in a container sharing the
  network namespace — speaks the protocol and drives the browser with
  this user's sessions. Isolation must come from who can reach the
  socket, not from a token. Use stdio (the default) for an
  agent-supervised server.
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
  action returns `isError: true` whose text carries the failure
  message plus its actionable hint on a second line (`hint: …`).
  Agents read them as data. Validation rejections (`amount` <= 0, a
  viewport out of range) are `invalid_params` protocol errors whose
  message names the rule; those values are typed signed in the schemas
  precisely so rmcp's deserializer never answers them with its own
  hint-less error text. Only a body that does not deserialize at all
  (a wrong-typed or missing field) surfaces that deserializer text,
  without a hint.
- Protocol-level failures (a refused session id, session capacity, an
  engine dead beyond the breaker) are JSON-RPC errors with code
  `-32000` and the same message-plus-hint text.
- Snapshots are rendered in the YAML text form of
  [snapshot format](snapshot-format.md#5-text-rendering-yaml-style),
  under the 20 000-character budget; the text ends with a marker line
  `… truncated` when `Snapshot::truncated` is set.
- Every mutating tool returns a fresh snapshot by default, unless
  stated otherwise below.

### Which vocabulary a failure speaks

A tool result renders the *session-layer* error
([`SessionError`](../crates/session/src/error.rs)) verbatim, so two
vocabularies can reach the same agent, distinguished by which arm the
failure took:

- **Action failures** speak [`ActionError`](../crates/core/src/error.rs)
  — the table below — message and hint both included, unchanged.
- **Engine-layer failures** (a navigation that could not be sent, a
  dead engine, a page cap, a read-only operation the backend refuses)
  keep the engine's own classification
  ([`EngineError`](../crates/engine/src/error.rs)) and its hint. The
  agent therefore sees engine text such as `engine terminated` or
  `capacity exceeded: …` — a failure the engine reports before any
  action taxonomy applies.
- **Session-layer failures** the engine never sees — `Capacity`
  (too many concurrent clients), `NoOpenPage` (nothing open to
  observe), `InvalidId` (an id that cannot name the session's storage
  file; the shipped transports mint conforming ids, so this reaches
  library callers, not this tool surface), `StorageWrite` (the
  session's state file could not be written), `Internal` — each speaks
  its own message.

What holds for **all** of them, and what an agent may rely on, is the
envelope rather than the wording: one text block, the message first,
the hint on its own `hint: ` line, and never empty.

The [`ActionError`](../crates/core/src/error.rs) taxonomy is also what
the [event backbone](events.md#2-event-vocabulary) carries: an
`ActionFailed` event reports its `error` in that vocabulary even when
the tool result reported the same failure in the engine's. Every
variant carries an English hint an agent can act on:

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

Twenty-five tools. Parameter types: `reference` is a snapshot handle
(`e17`); `direction` is one of `up|down|left|right`; durations are
milliseconds.

**Dialogs.** A page that calls `alert`, `confirm`, `prompt`, or
triggers a `beforeunload` confirmation never wedges the session: the
session dismisses every dialog immediately (a `prompt` resolves to its
dismiss value, `confirm` to `false`) and records the dismissal on the
event timeline ([events](events.md)); the page cannot block tools
waiting on it.

### navigate
`{ url: string }` → snapshot. Navigates the active page (opening the
first page when needed). Navigation uses the context's navigation
timeout; a failure to load reports the engine's own
`navigation to '<url>' failed: <cause>` message, with the transport
classification (`DnsFailed`, `TlsFailed`, an HTTP status, …) in the
cause field. The [event](#2-result-conventions) for the same failure
carries `ActionError::NavigationFailed`.

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
screenshot rate cap; a capture failure reports the engine's own
message ([result conventions](#2-result-conventions)).

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
(`a`, `Enter`, `Tab`, `ArrowLeft`, `F5`, …). An empty `key` is
`invalid_params`. Known keys carry their
physical code and virtual key code, and printable keys insert their
character into the focused element; unknown keys degrade to a
name-only event.

### select_option
`{ reference: string, values: string[] }` → snapshot. Auto-wait, then
selects options whose `value` is in `values` and dispatches
`input`/`change`. Fails with `NotInteractable` on non-select elements.

### upload_file
`{ reference: string, paths: string[] }` → snapshot. Auto-wait, then
sets the files of the file input `reference` points to. An empty
`paths` array is `invalid_params`. `paths` are
resolved by the machine the engine runs on; a path that does not exist
→ `NotInteractable`. Other refusals: the element is not a file input,
or a single-file input was given more than one path. Like every
action, the upload passes the [policy gate](policy.md#2-rule-evaluation)
under the `file_upload` class, which requires approval by default.

### scroll
`{ direction: up|down|left|right, amount: number, reference?: string }`
→ snapshot. Scrolls the page (no `reference`) or the resolved container
by `amount` pixels; `amount` ≤ 0 is `invalid_params`.

### set_viewport
`{ width: number, height: number }` → snapshot. Overrides the active
page's viewport in CSS pixels; values below `1` or above `10 000`
  are `invalid_params`. A display change: no policy judgment, and the
override survives navigations until the page closes.

### wait_for
`{ text: string, timeout_ms?: number }` → snapshot. Polls the page's
text content until `text` appears (default budget 10 000 ms);
exhaustion → `ActionError::TimedOut`. Requested budgets above
600 000 ms are clamped to that server-side maximum
([`MAX_POLL_BUDGET`](../crates/session/src/wait.rs)); the timeout
error reports the effective budget.

### tabs_list
`{}` → text block, one line per page: `<page-id> <url>`; the active
page is suffixed ` (active)`. Two boundary texts complete the format:
a page whose URL is not known yet prints `(unknown url)` in its
place, and a session with no page at all prints
`no pages open; navigate to open one`. Listing first reconciles with
the engine: windows that appeared without rutter opening them — a
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

### tabs_open
`{ url?: string }` → snapshot. Opens a new page in the session's
context and makes it the active one (the previous page keeps its
tracked slot); with `url`, the new page navigates there, judged by the
policy exactly like a `navigate` action at that URL. A denied
navigation or a navigation failure leaves the new page open at its
blank start, so the agent can see what it got and `tabs_close` it. The
context's page cap applies; over cap → action failure.

### get_cookies
`{}` → text block, every cookie scoped to the session's context, one
line per cookie as `domain<TAB>name=value` plus its flags
(`path=`, `secure`, `httpOnly`, `sameSite=`, `expires=`). Observation
only: no auto-wait, no snapshot, and the context's cookies are read
even when no page is open. Writing is [`set_cookies`](#set_cookies)
below; reading needs no approval, writing goes through the policy's
`cookies` class ([policy](policy.md)).

### set_cookies
`{ cookies: [{ name, value, domain, path?, secure?, http_only?,
same_site?, expires? }] }` → text confirmation. Cookies apply to the
session's context (context isolation, [glossary](glossary.md)).
`same_site` is `strict|lax|none`; `expires` is seconds since the Unix
epoch, and omitting it sets a session cookie. Writing goes through the
policy's `cookies` class, which requires approval by default
([policy](policy.md)); persistence across restarts works through
storage state ([sessions](sessions.md#3-storage-state)).

### network_requests
`{}` → text block, the network requests the active page made, oldest
first, one line per request as `METHOD url -> status [type]` or
`METHOD url -> failed (error) [type]`. Requests are recorded when they
finish; redirect hops are not reported, only how a request ended. A
bounded buffer keeps the most recent entries per page. Observation
only, like [`console_messages`](#console_messages).

### console_messages
`{}` → text block, the console output the active page produced —
`console.*` calls and uncaught exceptions — one `[level] text` line per
entry, oldest first. A bounded buffer keeps the most recent entries per
page; a closed page's entries go with it. Observation only: no
auto-wait, no snapshot, and a session with no page reports none instead
of opening one.

### close_session
`{}` → text confirmation. Closes the session's pages and context and
drops engine references. The call is terminal for the connection:
later tool calls on the same connection fail with `invalid_params`
naming the closed session. On a connection that never opened a
session the call is a no-op success, and later tool calls open a
session as usual. The engine keeps serving other sessions and
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
   actionable refs; `read` returns the page's markdown.
2. **Interact**: on a synthetic page, `click` a button that mutates the
   DOM → the returned snapshot reflects the change.
3. **Form**: `type` into an input (and `select_option`) → the snapshot
   value field reflects the input; `screenshot` returns non-empty PNG
   bytes.
