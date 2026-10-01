# dashboard/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). The wire contract is
[../../docs/dashboard.md](../../docs/dashboard.md).

## Boundary

- Normal workspace dependencies: `rutter-core`, `rutter-events`,
  `rutter-policy`, `rutter-session`.
- `rutter-engine` stays a dev-dependency. Production code does not
  import it.
- The dashboard reads events and submits approval decisions. It does
  not execute actions and does not open pages.
- Screencast uses `Session::screencast`. Frames are pulled only while
  a viewer is watching.
- Decisions go to `SessionManager::broker()`.

## Access

- The bind address is `127.0.0.1`.
- A handler that returns an asset, a socket, or dashboard state calls
  `auth::access_allowed` before that work: loopback `Host`, `Origin`
  bound to this server's host and port, and the launch token compared
  in constant time. The token arrives as a query parameter or an
  HttpOnly cookie. The 404 fallback stays a bare `NOT_FOUND`.
- Response hardening stays in the `harden_responses` layer.
- `DashboardServer::hand_off` delivers the token. When stderr is a
  terminal, the message contains the URL. Otherwise the URL is written
  to `dashboard-access-<port>.url` (mode `0o600` on Unix) and the
  message contains that path.
- `RUTTER_DASHBOARD_TOKEN` overrides the generated token. An empty
  value is ignored. A value shorter than 16 characters warns and is
  still used.
- `generate_token` and `engine-cdp`'s `launch_secret` stay the same
  construction. A change to one updates the other.
- HTTP and WebSocket decisions both go through `parse_decision`.

## WebSocket

- Connect order is replay, then live. A `Lagged` subscription resyncs
  from the ring.
- Client messages are `decision`, `screencast`, and `subscribe`.
- Writes are bounded and the connection sends keepalive.

## Embedded frontend

`INDEX_HTML`, `APP_JS`, and `I18N_EN` are `include_str!` of
`frontend/src/index.html`, `frontend/src/app.js`, and
`frontend/i18n/en.json`. Edit those sources. There is no frontend
build.
