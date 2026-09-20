# Events

English | [中文](events.zh.md)

The typed event backbone of
[`rutter-events`](../crates/events/src/backbone.rs): the structured
facts the orchestration layer publishes, and the delivery semantics
consumers rely on.

## 1. Envelope

Every fact travels as an
[`Envelope`](../crates/events/src/envelope.rs):

```json
{
  "seq": 41,
  "recorded_at": "2026-09-20T10:00:00.123456Z",
  "session": "stdio-1234",
  "event": { "type": "page_navigated", "page": "ctx-1:page-0",
             "url": "https://example.com/" }
}
```

- `seq` is a process-global monotonic counter — consumers sort and
  deduplicate by it.
- `recorded_at` is RFC 3339 UTC
  ([glossary](glossary.md#serialization-conventions)); clock trouble
  degrades to the epoch string rather than failing a publish.
- `session` names the workspace the fact belongs to.

## 2. Event vocabulary

The complete vocabulary lives in
[`crates/events/src/event.rs`](../crates/events/src/event.rs); every
variant is produced by the session or manager layer as labeled.

| Event | Payload | Produced when |
|---|---|---|
| `SessionStarted` | — | a session workspace came into being |
| `SessionClosed` | — | a session was closed by its client |
| `EngineStarted` | `backend`, `version` | the engine process launched for the first time |
| `EngineRestarted` | — | the supervisor replaced a dead engine; state before the restart is gone |
| `PageOpened` / `PageClosed` | `page` | a page opened/closed inside the session's context |
| `PageNavigated` | `page`, `url` | the active page navigated (`url` is effective after redirects) |
| `ActionRequested` | `page`, `origin`, `action` | an action was requested, before execution starts |
| `ActionCompleted` | `page`, `origin`, `action` | an action finished successfully |
| `ActionFailed` | `page`, `origin`, `action`, `error` | an action failed; `error` carries the [error taxonomy](tool-catalog.md#2-result-conventions) |
| `ApprovalRequested` | `request_id`, `page`, `action` | [policy](policy.md#4-approvals) parked an action until a human decides |
| `ApprovalResolved` | `request_id`, `granted` | a human answered, or the window timed out |

Actions and errors embed as [serialized
vocabulary](glossary.md#serialization-conventions) (`"type"` tag,
snake_case), so consumers name an action or failure the same way they
name events.

Screencast frames are **not** events. They flow as binary WebSocket
frames with their own latest-wins backpressure rule
([dashboard](dashboard.md#5-screencast)).

## 3. Delivery semantics

[`Backbone::publish`](../crates/events/src/backbone.rs) is
fire-and-forget: it never blocks and never fails, so publishing can
never stall action execution. Two channels with different loss rules:

- **Live bus.** A tokio broadcast channel (capacity 1024). Publishing
  succeeds even with no subscribers; a subscriber that falls further
  behind reads a `Lagged` error and must resync via replay.
- **Per-session rings.** A bounded ring per session (capacity 1000)
  keeps every semantic event for late joiners; `replay(session)`
  returns history oldest-first. Rings are dropped when their session
  closes, so a server that churns through session ids does not grow
  the map without bound.

The loss contract: **semantic events survive** (through the rings);
live subscribers that cannot keep up lose envelopes until they replay.
The dashboard shows the resync pattern
([dashboard §3](dashboard.md#3-websocket-protocol)): subscribe first,
snapshot the replay, deduplicate by sequence watermark, and refill any
`Lagged` gap from the rings.
