# Rutter Browser (Electron shell)

The Rutter browser: a minimal Chromium surface with one window and one
page — no tab strip, bookmarks, or profiles UI. Agents drive it over
CDP (rutter attaches to its debugging port); humans drive it with the
address bar; `F12` opens DevTools.

## Run

```sh
npm install
npm start                # dev: electron .
npm run package          # dist/RutterBrowser/RutterBrowser.exe (double-click)
```

## How rutter finds it

- When rutter spawns this app it passes `RUTTER_CDP_PORT` and
  `RUTTER_PROFILE` environment variables; the app enables the debugging
  port on that exact port with that per-launch profile.
- Standalone launches pick a free port and publish it in
  `<userData>/cdp-port` (`%APPDATA%/Rutter Browser/cdp-port`) so the
  independently started browser can be discovered on the machine.
- When rutter attaches, its fallback skips the shell's own toolbar
  document: the `toolbar.html` filename is part of the attach contract
  (see `crates/engine-cdp/src/context.rs`).

Electron reserves the `--app` switch, so rutter's chromeless-window
flag is deliberately not applied to this engine (see
`crates/engine-cdp/src/launch.rs`).

## Engine and shell versions

The two Chromium surfaces are pinned separately, on purpose:

- This shell pins Electron in `package.json`. An Electron release line
  embeds a fixed Chromium major (Electron 38 ships Chromium 140), so
  the pin holds the shell's CDP surface still.
- The engine rutter spawns follows Chrome for Testing's
  last-known-good stable channel (see
  `crates/engine/src/download/manifest.rs`), so its Chromium major
  moves over time.

The engine leads the protocol: rutter speaks the CDP surface the
engine exposes, and the shell only has to accept the attach flow.
When bumping the Electron pin, choose the newest release whose
Chromium major stays within one step of the engine's Chrome for
Testing line, then run the integration and e2e suites against the
real pair — that is the compatibility contract, not the version
numbers themselves.

## Security gates

Electron's defaults grant more than this shell should, so the shell
pins its own:

- **Permissions default to deny.** Permission requests and Chromium's
  synchronous permission checks resolve through one predicate
  (`browser/permission-gate.js`): only `fullscreen`, `pointerLock`,
  and `clipboard-sanitized-write` — the powers that cannot pull data
  off the machine — are granted; geolocation, camera, notifications,
  and everything unlisted are refused. The gate is attached to the
  default session and re-attached to every session created later
  (`app.on('session-created')`), so no session inherits the allow-all
  default.
- **One navigation whitelist.** Renderer-initiated top-level
  navigation (`will-navigate`) is gated by the same `isWebSchemeUrl`
  predicate the main-process entry points use, so a page cannot steer
  any window to `file:`, `devtools:`, or `javascript:` URLs.
  Programmatic `loadFile`/`loadURL` never fire `will-navigate`, so the
  shell's own toolbar and start targets are unaffected.
- **Pinned renderer settings.** Both the toolbar window and the shared
  view set `contextIsolation: true`, `nodeIntegration: false`,
  `sandbox: true`, and `webviewTag: false` explicitly instead of
  leaning on defaults that can drift across Electron releases.
- **The navigate channel checks its sender.** Only a frame whose URL
  is the toolbar document (`file://.../toolbar.html`, the same matcher
  the Rust attach fallback uses) may send `navigate`; anything else is
  ignored.

Both gate predicates run in `scripts/check_js.sh`
(`tests/js/scheme-gate.test.mjs`, `tests/js/permission-gate.test.mjs`),
so the shipped refusals are pinned next to the acceptances.
