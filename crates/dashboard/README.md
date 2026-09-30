# rutter-dashboard/ — supervision dashboard

English | [中文](README.zh.md)

Local web server (127.0.0.1, per-launch token) streaming events and
screencast frames to the embedded vanilla-JS frontend, and submitting
human approval decisions back to the broker.

## Boundary

Observation plus verdict submission — the dashboard never executes
actions (docs/architecture.md). It reads the event backbone (replay, then
live), pulls screencast frames only while a viewer watches, and posts
decisions to the same broker the sessions park on. Every endpoint goes
through one gate: loopback `Host` check + token (query or HttpOnly
cookie) — see `src/auth.rs`. The token reaches a human through
`DashboardServer::hand_off`, which chooses the channel by what sits on
stderr: a terminal gets the URL, a pipe — the supervised MCP client's
case — gets a file that is owner-only on Unix, and only its path is
printed
(access control, docs/dashboard.md).

## Wire contract

The WebSocket message vocabulary — the envelopes a client receives
(replay, then live), the client messages it may send (`decision`,
`screencast`, `subscribe`), the acks and the binary screencast frames
that come back — is documented in
[docs/dashboard.md §3](../../../docs/dashboard.md#3-websocket-protocol).
It is the contract a third-party client implements against; the
embedded `frontend/src/app.js` is one consumer of it, not the spec.
The decision body itself is shared with the HTTP post and parsed by
one function (`parse_decision` in `src/lib.rs`), so the two transports
cannot disagree about what a decision is.

## Consumers

- `cli` attaches it to `serve` with `--dashboard PORT`.
