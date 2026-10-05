// Rutter Browser main process.
//
// One window, one page surface: a thin toolbar on top and the live page
// below — no tab strip, no bookmarks, no profiles UI. Agents drive the
// page over CDP (the Chromium debugging port rutter attaches to);
// humans drive it through the toolbar and F12.
//
// The debugging port is negotiated at startup: an explicit
// `--remote-debugging-port` (what rutter passes when it spawns this
// app) wins; otherwise a free loopback port is picked and published in
// `<userData>/cdp-port` so the running browser can be discovered on
// the machine later.

const {
  app,
  BrowserWindow,
  WebContentsView,
  Menu,
  ipcMain,
  session,
} = require("electron");
const fs = require("fs");
const net = require("net");
const path = require("path");

const { isWebSchemeUrl } = require("./scheme-gate.js");
const { isPermissionGranted } = require("./permission-gate.js");

const TOOLBAR_HEIGHT = 48;
const START_PAGE = "start.html";

// A per-launch profile handed over by rutter: it keeps instances
// isolated and must be set before anything reads userData.
if (process.env.RUTTER_PROFILE) {
  app.setPath("userData", process.env.RUTTER_PROFILE);
}

let win = null;
let view = null;

// Picks a free loopback port. Must run before app ready: the debugging
// port is a Chromium switch, read once at process start.
function pickFreePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

async function ensureDebugPort() {
  if (app.commandLine.hasSwitch("remote-debugging-port")) return;
  const requested = Number(process.env.RUTTER_CDP_PORT);
  // A garbled RUTTER_CDP_PORT must not become --remote-debugging-port=NaN
  // (the debugging endpoint would never come up); any value outside the
  // valid port range falls back to a picked port.
  const port =
    Number.isInteger(requested) && requested >= 1 && requested <= 65535
      ? requested
      : await pickFreePort();
  app.commandLine.appendSwitch("remote-debugging-port", String(port));
}

// Publishes the port so the running browser can be discovered on this
// machine later. Best-effort: the browser works without it.
function portFilePath() {
  return path.join(app.getPath("userData"), "cdp-port");
}

function publishPort() {
  const port = app.commandLine.getSwitchValue("remote-debugging-port");
  if (!port) return;
  try {
    fs.mkdirSync(app.getPath("userData"), { recursive: true });
    // The debugging endpoint is unauthenticated, so the port number is
    // a capability: owner-only on Unix (Windows ignores the mode and
    // keeps the profile directory's per-user ACL).
    fs.writeFileSync(portFilePath(), String(port), { mode: 0o600 });
  } catch {
    // Discovery is best-effort.
  }
}

// The scheme gate itself lives in scheme-gate.js, shared by both
// navigation entry points and tested from tests/js. The permission
// predicate lives in permission-gate.js the same way; attaching it to
// a session is Electron glue, so it stays here.

// Electron grants every permission request by default (geolocation,
// camera, notifications, ...), so each session gets the gate: the
// async request handler and the synchronous check handler both resolve
// through the same default-deny predicate. It is attached to the
// default session at startup and re-attached to every session created
// later, so a future session cannot silently inherit the allow-all
// default. Attaching twice is harmless.
function attachPermissionGate(sess) {
  sess.setPermissionRequestHandler((_wc, permission, callback) => {
    callback(isPermissionGranted(permission));
  });
  // Electron passes a third `requestingOrigin` argument; the predicate
  // does not read it, so the handler does not declare it.
  sess.setPermissionCheckHandler((_wc, permission) =>
    isPermissionGranted(permission)
  );
}

app.on("session-created", attachPermissionGate);

// Renderer-initiated top-level navigation gets the same gate as the
// main-process entry points: a page that navigates itself cannot reach
// file:, devtools:, or javascript: URLs even if Chromium's own default
// blocks ever shift. loadFile/loadURL are programmatic and never fire
// will-navigate, so the shell's own toolbar and start targets pass
// untouched. Only window-typed contents are gated — devtools' internal
// surfaces must keep navigating their own bundled pages.
app.on("web-contents-created", (_event, contents) => {
  if (contents.getType() !== "window") return;
  contents.on("will-navigate", (event, url) => {
    if (!isWebSchemeUrl(url)) event.preventDefault();
  });
});

function contentBounds() {
  const { width, height } = win.getContentBounds();
  // A window shorter than the toolbar (resize below 48px) would hand
  // setBounds a negative height, which Electron rejects; clamp instead.
  return {
    x: 0,
    y: TOOLBAR_HEIGHT,
    width,
    height: Math.max(0, height - TOOLBAR_HEIGHT),
  };
}

function createWindow() {
  win = new BrowserWindow({
    width: 1280,
    height: 880,
    title: "Rutter Browser",
    backgroundColor: "#ffffff",
    autoHideMenuBar: true,
    show: false,
    // Pinned explicitly instead of leaning on Electron defaults: the
    // sandboxed contextBridge preload keeps working under sandbox,
    // and a pin cannot drift the way a default can across releases.
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  });
  // Cross-component contract: the Rust attach fallback
  // (crates/engine-cdp/src/context.rs, `is_toolbar_document`) excludes
  // file:// targets ending in "/toolbar.html" from the drivable page
  // surface, so this filename must not change without that matcher.
  win.loadFile("toolbar.html", { preload: path.join(__dirname, "preload.js") });
  win.once("ready-to-show", () => win.show());
  win.on("resize", () => view.setBounds(contentBounds()));

  // The shared view renders untrusted web content: the same pinned
  // webPreferences as the toolbar, so nothing here reaches Node.
  view = new WebContentsView({
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  });
  win.contentView.addChildView(view);
  view.setBounds(contentBounds());
  view.webContents.loadFile(START_PAGE);

  // No tab strip: a link that asks for a new window navigates in place.
  // Web content initiates this callback, so the scheme gate is the same
  // `isWebSchemeUrl` the navigate channel tests: a page must not steer
  // the shared view to file:, devtools:, or javascript: URLs through
  // the main process.
  view.webContents.setWindowOpenHandler(({ url }) => {
    if (isWebSchemeUrl(url)) {
      loadSharedView(url);
    }
    return { action: "deny" };
  });
  const reportUrl = (_event, url) => win.webContents.send("navigated", url);
  view.webContents.on("did-navigate", reportUrl);
  view.webContents.on("did-navigate-in-page", reportUrl);

  // A crashed renderer must not leave the surface blank: reload it so
  // the window keeps offering a drivable page (the agent's CDP surface
  // and the human's window stay recoverable). Consecutive crashes are
  // counted so a page that dies on load cannot spin reloads forever;
  // a successful navigation resets the count. A rejected reload is
  // ignored on purpose: the crashed page stays visible and a CDP
  // navigation remains the recovery path.
  let rendererCrashes = 0;
  view.webContents.on("render-process-gone", (_event, details) => {
    if (details.reason === "clean-exit" || rendererCrashes >= 3) return;
    rendererCrashes += 1;
    view.webContents.reload().catch(() => {
      // A rejected reload keeps the crashed page visible; CDP
      // navigation remains the recovery path.
    });
  });
  view.webContents.on("did-navigate", () => {
    rendererCrashes = 0;
  });

  // F12 / Ctrl+Shift+I on either surface toggles DevTools for the page.
  for (const contents of [win.webContents, view.webContents]) {
    contents.on("before-input-event", (event, input) => {
      const f12 = input.type === "keyDown" && input.key === "F12";
      const ctrlShiftI =
        input.type === "keyDown" &&
        input.control &&
        input.shift &&
        input.key.toLowerCase() === "i";
      if (f12 || ctrlShiftI) {
        if (view.webContents.isDevToolsOpened()) view.webContents.closeDevTools();
        else view.webContents.openDevTools({ mode: "detach" });
        event.preventDefault();
      }
    });
  }
}

// Only the toolbar's own document may drive this channel: the sender
// check mirrors the Rust attach fallback's `is_toolbar_document`
// matcher (crates/engine-cdp/src/context.rs) — a file:// URL ending in
// "/toolbar.html". A web page is never a file:// URL, so a frame that
// is not shell UI cannot borrow the channel even if it learns it.
ipcMain.on("navigate", (event, url) => {
  const senderUrl = event.senderFrame?.url ?? "";
  if (!(senderUrl.startsWith("file://") && senderUrl.endsWith("/toolbar.html"))) {
    console.warn("rutter-browser: navigate rejected from non-toolbar frame");
    return;
  }
  if (typeof url !== "string" || !url) return;
  // The same scheme gate `setWindowOpenHandler` applies: a bare word is
  // read as a host and completed with https, and no file:, devtools:,
  // or other privileged scheme may drive the shared view through this
  // channel.
  const target = isWebSchemeUrl(url) ? url : `https://${url}`;
  loadSharedView(target);
});

// Navigates the shared view, settling the load's rejection instead of
// leaving it unhandled.
function loadSharedView(url) {
  view.webContents.loadURL(url).catch(() => {
    // A failed load already shows Chromium's own error page, so there
    // is nothing left to report.
  });
}
ipcMain.on("back", () => view.webContents.goBack());
ipcMain.on("forward", () => view.webContents.goForward());
ipcMain.on("reload", () => view.webContents.reload());
ipcMain.on("home", () => view.webContents.loadFile(START_PAGE));

// The debugging port switch is only honored before Chromium starts, so
// the port is settled before waiting for ready.
ensureDebugPort()
  .then(async () => {
    await app.whenReady();
    // The default session may exist before this module loads, so the
    // session-created hook cannot be relied on to cover it.
    attachPermissionGate(session.defaultSession);
    Menu.setApplicationMenu(null);
    createWindow();
    publishPort();
  })
  .catch((error) => {
    console.error("rutter-browser startup failed:", error);
    app.quit();
  });

app.on("window-all-closed", () => {
  try {
    fs.rmSync(portFilePath(), { force: true });
  } catch {
    // Best-effort cleanup.
  }
  app.quit();
});
