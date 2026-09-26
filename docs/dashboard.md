# Dashboard

English | [中文](dashboard.zh.md)

The local supervision dashboard of
[`rutter-dashboard`](../crates/dashboard/src/lib.rs): a web server
over the event backbone and the approval broker. The dashboard never
executes actions — observation plus verdict submission, nothing more.

## 1. Server

Attached with `rutter serve --dashboard <PORT>`. Binds `127.0.0.1`
only. The frontend is vanilla JS with no build step, embedded into the
binary at compile time (`include_str!`); UI strings come from the
`frontend/i18n/en.json` catalog.

| Route | Serves |
|---|---|
| `/` | The page shell; exchanges the access token for a session cookie |
| `/app.js`, `/i18n/en.json` | Application script and string catalog |
| `/ws` | The WebSocket (replay + live events, decisions, screencast) |
| `/api/decisions` | `POST` approval decisions from simple automation clients |
| `/api/pending` | `GET` how many approvals are parked right now |

## 2. Access control

Every endpoint, including the WebSocket upgrade, passes the same gate
([`auth.rs`](../crates/dashboard/src/auth.rs)):

- **Per-launch token.** A 64-bit hex token is minted once, when the
  dashboard is built — hashing launch time and process id with a
  randomly keyed hasher. `RUTTER_DASHBOARD_TOKEN` overrides it for
  automation; that override is visible to whoever launches rutter, so
  it belongs to a trusted launcher, not to a human-only channel.
- **Handing the URL over.** stderr decides the channel by what sits on
  it. A terminal gets `…/?token=…`. A pipe — an MCP client launching
  `rutter serve` — gets the listening address and the hand-off file
  path, never the URL or token, because that client
  is the process rutter exists to supervise; the URL goes to
  `<cache-dir>/dashboard-access-<port>.url`, created owner-only on
  Unix. The file is written only after the bind succeeded and is named
  by the port the server actually owns, so `--dashboard 0` lands under
  its real port and two rutter processes on one machine write
  different files — neither hand-off can clobber the other's. With
  neither a terminal nor that path available the dashboard refuses to
  start rather than fall back to printing the token.
- **What this does not buy.** rutter and its client share one machine
  and one user account, and a process in that account can read anything
  rutter writes under its own cache. The hand-off removes the
  *automatic* leak into an inherited channel; it is not a human-only
  oracle. A deployment that must deny an agent the ability to approve
  its own actions runs the dashboard outside the agent's account.
- **Token-for-cookie exchange.** The first visit with `?token=` sets
  an `HttpOnly`, `SameSite=Strict` session cookie; later requests
  authenticate through the cookie alone. Generated tokens are always
  cookie-safe; a `RUTTER_DASHBOARD_TOKEN` override that is not keeps
  authenticating through the query parameter.
- **`Host` validation** defends against DNS rebinding.

## 3. WebSocket protocol

One socket carries everything
([`ws.rs`](../crates/dashboard/src/ws.rs)). On connect:

1. Replay of every open session's history, sorted by global sequence
   number.
2. The live stream. Envelopes at or below the replay watermark are
   duplicates and are skipped.

If the client falls behind the broadcast channel (`Lagged`), the
server refills the gap from the per-session rings before continuing —
the [loss contract](events.md#3-delivery-semantics) in action. If the
engine has not started yet, the server sends
`{"type":"note","text":"engine not started yet"}` and closes.

Client messages (JSON text frames):

| Message | Effect |
|---|---|
| `{"type":"decision","request_id":"apr-7","grant":true}` | Submits an approval; the reply is a `decision-ack` echoing the id and whether it was accepted. Built through serde, so a hostile `request_id` cannot forge reply fields |
| `{"type":"screencast","on":true,"session":"…"}` | Starts a screencast of that session's active page (`on: false`, or dropping the connection, stops it); acknowledged with `screencast-ack` |
| `{"type":"subscribe"}` | Accepted as a no-op: replay and live delivery start automatically on connect |

Server frames: text frames carry JSON envelopes and acks; binary
frames carry JPEG screencast images.

## 4. Decision HTTP API

`POST /api/decisions` with `{"request_id":"apr-7","grant":false}`
submits the same decisions for simple automation clients, behind the
same token gate. An unknown or already-resolved `request_id` answers
`404`; a malformed body answers `400`.

## 5. Screencast

Screencast is **on-demand**: streaming starts when a viewer asks and
stops when it leaves. It is also **read-only**: a session with no open
page answers `no open page to observe` rather than opening one, because
creating a tab is an action and the dashboard performs none
(docs/architecture.md). Frames flow one way — CDP
`Page.startScreencast` (JPEG, width ≤ 1024, per-frame ack) through a
bounded channel of 4, dropping frames over a full channel. A stalled
viewer loses frames, not memory, and the capture can never block
action execution. CDP stops screencasts on navigation; the transport
restarts the capture on the navigation event
([engine supervision §4](engine-supervision.md#4-cdp-transport-notes)).
