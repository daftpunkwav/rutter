# MCP Tool Specification

> Status: v1 · 2026-09-17 · Normative contract for the `rutter-mcp`
> tool surface (blueprint §7.8). Written before implementation; the
> spec is the contract. Reference for naming: `docs/BLUEPRINT.md` §6.

## 1. Transport and sessions

- Transport: stdio (blueprint §3) or streamable HTTP via
  `rutter serve --http ADDR`. The binary entry is
  `rutter serve [--headed]`; the dashboard attaches through
  `serve --dashboard PORT` and the policy file through `serve --policy FILE`.
  A non-loopback `--http` bind is refused unless confirmed with
  `serve --allow-remote` (the transport has no authentication).
- One MCP client connection is one Session. The SessionId is minted by
  the server at connection start and reported in the
  `SessionStarted` event; it appears in error payloads that reference a
  session.
- The engine is spawned lazily: `serve` starts with no engine; the
  first tool call that needs a page launches the engine (supervised,
  blueprint §8.5) and creates the session's Context with the configured
  caps. Startup of the MCP server itself performs no I/O.
- Every session has exactly one Context. The active Page is the target
  of all page-scoped tools; `navigate` opens the first page when none
  exists.

## 2. Result conventions

- Tools return MCP content blocks. Text results use one `text` block.
- **Action failures are results, not protocol errors**: a failed action
  returns `isError: true` with a `text` block containing the
  `ActionError` message plus its actionable hint on a second line
  (`hint: …`). Agents read them as data (blueprint §7.3).
- Protocol-level failures (unknown session, engine dead beyond the
  breaker) are JSON-RPC errors with code `-32000` (server error) and
  the same message-plus-hint text.
- Snapshots are rendered in the YAML text form of
  `docs/SNAPSHOT_SPEC.md` §5, under the 20 000 character token budget;
  the text ends with a marker line `… truncated` when
  `Snapshot::truncated` is set.
- Every mutating tool returns a fresh snapshot by default
  (blueprint §7.3), unless stated otherwise below.

## 3. Auto-wait semantics

Every mutating tool that carries a `reference` runs the three-phase
auto-wait before acting; phases poll the live page:

| Phase     | Pass condition                                        | Budget (default) |
|-----------|--------------------------------------------------------|------------------|
| `visible` | resolved element has a non-empty box                  | 5 s              |
| `stable`  | box unchanged across two samples ~80 ms apart         | 5 s              |
| `enabled` | element is not `disabled` and not `aria-disabled`     | 5 s              |

Before `visible`, the resolver scrolls the element into view. After the
action, the executor settles for 250 ms (v1: lets same-tick navigations
start), then takes the fresh snapshot. Budget exhaustion maps to
`ActionError::TimedOut` with the phase that failed. A reference that no
longer resolves maps to `ActionError::ReferenceExpired`.

## 4. Tools

Parameter types: `reference` is a snapshot handle (`e17`); `direction`
is one of `up|down|left|right`; durations are milliseconds.

### navigate
`{ url: string }` → snapshot. Navigates the active page (opening the
first page when needed). Navigation uses the context's navigation
timeout; failure → `ActionError::NavigationFailed`.

### back / forward / reload
`{}` → snapshot. History operations on the active page.

### snapshot
`{}` → snapshot. No auto-wait; renders the current composed DOM.

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
text content until `text` appears (default budget 10 s); exhaustion →
`ActionError::TimedOut`. Requested budgets above 600 000 ms are
clamped to that server-side maximum; the timeout error reports the
effective budget.

### tabs_list
`{}` → text block, one line per page: `<page-id> <url>`; the active
page is suffixed ` (active)`.

### tabs_select
`{ page_id: string }` → snapshot. Unknown id → `invalid_params`.

### tabs_close
`{ page_id: string }` → text confirmation. Unknown id →
`invalid_params`. Closing the active page
makes the first remaining page active; closing the last page is
allowed — the next `navigate` opens a new one.

### set_cookies
`{ cookies: [{ name, value, domain, path?, secure?, http_only?,
same_site? }] }` → text confirmation. Cookies apply to the session's
Context (context isolation, blueprint §6). `same_site` is
`strict|lax|none`; persistence across restarts works through storage state (see §4.2).

### close_session
`{}` → text confirmation. Closes the session's pages and context and
drops engine references. The call is terminal for the connection:
later tool calls on the same connection fail with `invalid_params`
naming the closed session. The engine keeps serving other sessions and
shuts down when the server exits (client disconnect or Ctrl-C), not
when a session closes.

## 5. Event backbone (not a tool)

The event backbone of blueprint §7.5 (typed events, bus, per-session
ring, replay) runs inside the server. MCP has no notifications or
event tools; the dashboard consumes replay over WebSocket.

## 6. Acceptance suites

Three e2e task classes pass against the real engine
(`#[ignore]`-gated integration tests driving the built binary over
stdio):

1. **Read**: `navigate` → `snapshot` shows the page's heading and
   actionable refs.
2. **Interact**: on a synthetic page, `click` a button that mutates the
   DOM → the returned snapshot reflects the change.
3. **Form**: `type` into an input (and `select_option`) → the snapshot
   value field reflects the input; `screenshot` returns non-empty PNG
   bytes.
