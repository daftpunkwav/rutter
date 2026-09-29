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
- **`Host` validation** defends against DNS rebinding, and a present
  **`Origin`** header must name the dashboard's *own* origin: a loopback
  name **and the port the listener actually bound**, compared exactly.
  State-changing requests (`POST /api/decisions`, WebSocket messages)
  must come from there, so relaxing the cookie's `SameSite` for a second
  dashboard origin would not open the approval surface to cross-site
  submits. The port is the half that carries the defence: `SameSite` is
  a *site* check and a site spans every port on a host, so a page served
  from `http://127.0.0.1:<any other port>` is same-site with the
  dashboard, has the token cookie attached to its request, and is a
  loopback origin — matching the host alone let it submit an approval
  grant. An authority that omits the port is read as the scheme's
  default, so a dashboard on port 80 still accepts its own origin. No
  `Origin` header (a non-browser client) falls back to the `Host` and
  token checks.
- **Response headers**, stamped on every answer — the routes, a refused
  gate, and the fallback alike — by one middleware, so a route added
  later inherits them instead of having to remember them
  ([`auth::harden`](../crates/dashboard/src/auth.rs)):

  | Header | Why |
  |---|---|
  | `Cache-Control: no-store` | The first visit's URL is the one place the token itself appears; it must not settle in the browser's disk cache or back/forward cache |
  | `Referrer-Policy: no-referrer` | Keeps that URL out of the `Referer` of anything the page ever loads or navigates to. The dashboard loads nothing cross-origin today; the header makes that a property of the server rather than of the frontend |
  | `X-Frame-Options: DENY`, `Content-Security-Policy: frame-ancestors 'none'` | Refuse to frame the approval UI. The cookie's `SameSite=Strict` does not cover the first visit: that one authenticates from the query string, so a holder of the URL could otherwise embed the card its own token opened and overlay the Grant button |
  | `X-Content-Type-Options: nosniff` | Keeps `/app.js` and `/i18n/en.json` from being read as anything but what they are |

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
| `{"type":"screencast","on":true,"session":"…"}` | Starts a screencast of that session's active page (`on: false`, or dropping the connection, stops it); the `screencast-ack` carries `"started": true`, or `"started": false` — with a `reason` when it refused (an unknown session, a request naming no session, or `no open page to observe` for a session with nothing open), without one for a plain `on: false` |
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
([engine supervision](engine-supervision.md#4-cdp-transport-notes)).
