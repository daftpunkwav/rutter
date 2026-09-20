# src/ — file map

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `lib.rs` | `DashboardServer`: bind, routes, static handlers, embedded assets, HTTP decision endpoint, token hand-off |
| `auth.rs` | The endpoint gate: loopback `Host` check, token (query/HttpOnly cookie), constant-time compare |
| `ws.rs` | WebSocket loop: replay → live, Lagged resync, screencast forwarding, decision messages |

Every new route must pass through `auth::access_allowed` — the gate
exists so a route cannot ship with half the security checks.
