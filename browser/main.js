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

const { app, BrowserWindow, WebContentsView, Menu, ipcMain } = require("electron");
const fs = require("fs");
const net = require("net");
const path = require("path");

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
  });
  // Cross-component contract: the Rust attach fallback
  // (crates/engine-cdp/src/context.rs, `is_toolbar_document`) excludes
  // file:// targets ending in "/toolbar.html" from the drivable page
  // surface, so this filename must not change without that matcher.
  win.loadFile("toolbar.html", { preload: path.join(__dirname, "preload.js") });
  win.once("ready-to-show", () => win.show());
  win.on("resize", () => view.setBounds(contentBounds()));

  view = new WebContentsView({ webPreferences: { contextIsolation: true } });
  win.contentView.addChildView(view);
  view.setBounds(contentBounds());
  view.webContents.loadFile(START_PAGE);

  // No tab strip: a link that asks for a new window navigates in place.
  // Web content initiates this callback, so only web schemes may drive
  // the shared view — a page must not steer it to file:, devtools:, or
  // javascript: URLs through the main process.
  view.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url) || url === "about:blank") {
      view.webContents.loadURL(url);
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
    view.webContents.reload().catch(() => {});
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

ipcMain.on("navigate", (_event, url) => {
  if (!url) return;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(url) ? url : `https://${url}`;
  view.webContents.loadURL(withScheme);
});
ipcMain.on("back", () => view.webContents.goBack());
ipcMain.on("forward", () => view.webContents.goForward());
ipcMain.on("reload", () => view.webContents.reload());
ipcMain.on("home", () => view.webContents.loadFile(START_PAGE));

// The debugging port switch is only honored before Chromium starts, so
// the port is settled before waiting for ready.
ensureDebugPort()
  .then(async () => {
    await app.whenReady();
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
