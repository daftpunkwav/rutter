// Bridges the toolbar page to the main process. The toolbar is local
// UI, so the bridge is deliberately tiny: navigation commands out, one
// URL event back.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("rutter", {
  navigate: (url) => ipcRenderer.send("navigate", url),
  back: () => ipcRenderer.send("back"),
  forward: () => ipcRenderer.send("forward"),
  reload: () => ipcRenderer.send("reload"),
  home: () => ipcRenderer.send("home"),
  onNavigated: (callback) =>
    ipcRenderer.on("navigated", (_event, url) => callback(url)),
});
