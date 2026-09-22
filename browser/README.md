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
