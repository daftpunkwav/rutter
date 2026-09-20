# src/ — dashboard client

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `index.html` | Page shell: panes (timeline, approvals, live view), `data-i18n` hooks |
| `app.js` | WebSocket client: replay/live rendering, approval decisions, screencast on/off, 1 s reconnect |

The token arrives in the first visit's query and is exchanged by the
server for an HttpOnly cookie; this script never stores it — requests
fall back to the cookie when no query token is present (refresh,
bookmark). Missing i18n keys fall back to the key name itself.
