# frontend/ agent rules

The layout lives in [README.md](README.md). The wire contract is
[../docs/dashboard.md](../docs/dashboard.md). `rutter-dashboard`
embeds the files named below.

## Sources

- Vanilla JS, ES2017 or newer. No framework, no npm, no build step.
- `rutter-dashboard` embeds `src/index.html`, `src/app.js`, and
  `i18n/en.json` with `include_str!`.
- A `README.md` change includes `README.zh.md`.

## Copy

- User-visible prose lives in `i18n/en.json`.
- `src/index.html` references keys with `data-i18n`. `src/app.js`
  looks keys up with `t()`.
- A missing key renders as the key name. A failed catalog fetch does
  not block the WebSocket connection.

## Token and socket

- The launch token arrives on the first visit's query. The server
  exchanges it for an HttpOnly cookie.
- `app.js` does not store the token. Later requests use the cookie
  when the query token is absent.
- The client renders replay, then live events. It sends `screencast`
  on the socket. Approval decisions are `POST /api/decisions`. It
  does not send `subscribe` or a socket `decision`.
- Reconnect starts at 1 s and backs off, capped at 30 s. A reconnect
  resets the timeline and the session set.

## Tests

Behavior is pinned by `tests/js/app.test.mjs` through
`scripts/check_js.sh`.
