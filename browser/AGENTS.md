# browser/ agent rules

The shell contract lives in [README.md](README.md). A `README.md`
change includes `README.zh.md`.

## Role

- One window, one page. No tab strip, bookmarks, or profiles UI.
- rutter attaches over CDP. Humans use the address bar. `F12` opens
  DevTools.
- `toolbar.html` is part of the attach contract.
  `crates/engine-cdp/src/context.rs` treats a URL as the toolbar only
  when it starts with `file://` and ends with `/toolbar.html`. A
  rename updates that matcher in the same change.
- This shell does not receive Chromium's `--app` switch. An explicit
  engine executable is launched with `app_window` left false.

## Attach

- `RUTTER_PROFILE`, when set, is applied with `app.setPath("userData")`
  before anything else reads `userData`.
- An existing `--remote-debugging-port` switch is left as passed.
  Otherwise `RUTTER_CDP_PORT` is used when it is an integer from 1 to
  65535. Any other value picks a free port. The value `NaN` is never
  written into the switch.
- The chosen port is written to `<userData>/cdp-port` with mode
  `0o600`. A failed write does not stop the browser.

## Versions

- Electron is pinned in `package.json`. A pin change includes
  `package-lock.json`.
- The engine rutter downloads follows Chrome for Testing's
  last-known-good stable channel
  (`crates/engine/src/download/manifest.rs`).
- An Electron bump stays within one Chromium major of that engine
  line. The integration suites and the e2e suites are run against
  that pair.

## Security

- Permission requests and Chromium's synchronous permission checks go
  through `permission-gate.js` only. Granted: `fullscreen`,
  `pointerLock`, `clipboard-sanitized-write`. Every other permission
  is denied.
- The gate is attached to the default session and on
  `session-created`.
- Renderer-initiated top-level navigation (`will-navigate`) uses
  `isWebSchemeUrl` from `scheme-gate.js`. `file:`, `devtools:`, and
  `javascript:` are refused.
- The toolbar window and the shared view set `contextIsolation: true`,
  `nodeIntegration: false`, `sandbox: true`, and `webviewTag: false`.
- The `navigate` channel accepts a sender only when its URL is the
  toolbar document (`file://` and `toolbar.html`).

## Tests

`scripts/check_js.sh` runs `tests/js/scheme-gate.test.mjs` and
`tests/js/permission-gate.test.mjs`. A gate change updates those
tests in the same commit.
