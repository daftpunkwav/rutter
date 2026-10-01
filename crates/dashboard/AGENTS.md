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
- Every route except the fallback calls `auth::access_allowed` before
  other work. The fallback stays a bare `NOT_FOUND`.
- `Host` must be a loopback name: `127.0.0.1`, `localhost`, or `::1`.
  The `Host` port is not checked.
- A missing `Origin` passes. A present `Origin` must parse as
  `scheme://authority`, use a loopback name, and name the port this
  server bound. An omitted port is 80 for `http` and 443 for `https`.
  Any other scheme without an explicit port fails. `Origin: null`
  fails.
- The launch token is compared in constant time. It arrives as the
  query parameter `token` or the HttpOnly cookie `rutter_token`.
- Response hardening stays in the `harden_responses` layer.
- `DashboardServer::hand_off` delivers the token. When stderr is a
  terminal, the message contains the URL. When it is not, and an
  access directory was configured, the URL is written to
  `dashboard-access-<bound-port>.url` in that directory (mode `0o600`
  on Unix) and the message contains that path. The port is the port
  the bind settled on, including a request for port `0`. With no
  access directory, `bind` fails and writes no file.
- `RUTTER_DASHBOARD_TOKEN` overrides the generated token. An empty
  value is ignored. A value shorter than 16 characters warns and is
  still used.
- `generate_token` and `engine-cdp`'s `launch_secret` stay the same
  construction. A change to one updates the other.
- HTTP and WebSocket decisions both go through `parse_decision`.

## WebSocket

- Connect order is subscribe, then replay, then live. Live envelopes
  at or below the replay watermark are skipped. A `Lagged`
  subscription resyncs with `replay_after`.
- Client messages are `decision`, `screencast`, and `subscribe`.
- Writes are bounded and the connection sends keepalive.

## Embedded frontend

`INDEX_HTML`, `APP_JS`, and `I18N_EN` are `include_str!` of
`frontend/src/index.html`, `frontend/src/app.js`, and
`frontend/i18n/en.json`. Edit those sources. There is no frontend
build.
