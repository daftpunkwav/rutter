// Packages the app for double-click use: copies the Electron runtime
// next to a `resources/app` copy of this directory, the layout Electron
// resolves on a bare `electron.exe` launch. Output:
// dist/RutterBrowser/RutterBrowser.exe.
//
// No installer, no signing, no auto-update — this is the minimal
// double-click story. Replace with electron-builder when installers
// matter.
const fs = require("fs");
const path = require("path");

const root = __dirname;
const dist = path.join(root, "dist", "RutterBrowser");
const electronDist = path.join(root, "node_modules", "electron", "dist");
const resources = path.join(dist, "resources");

fs.rmSync(dist, { recursive: true, force: true });
fs.cpSync(electronDist, dist, { recursive: true });
fs.mkdirSync(path.join(resources, "app"), { recursive: true });
for (const file of ["package.json", "main.js", "preload.js", "toolbar.html", "start.html"]) {
  fs.cpSync(path.join(root, file), path.join(resources, "app", file));
}
console.log(`packaged: ${path.join(dist, "RutterBrowser.exe")}`);
