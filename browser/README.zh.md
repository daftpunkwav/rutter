# Rutter Browser（Electron 外壳）

Rutter 浏览器：一个最小化的 Chromium 页面表面——单窗口单页面，没有
标签栏、收藏夹和 profile UI。agent 通过 CDP 驱动它（rutter 会附到它
的调试端口）；人类用地址栏操作；`F12` 打开 DevTools。

## 运行

```sh
npm install
npm start                # 开发模式：electron .
npm run package          # dist/RutterBrowser/RutterBrowser.exe（可双击）
```

## rutter 如何找到它

- rutter 拉起本应用时会传 `RUTTER_CDP_PORT` 与 `RUTTER_PROFILE` 两个
  环境变量；应用会在该端口开启调试端点，并使用该独立 profile。
- 独立双击启动时会挑一个空闲端口，并发布到
  `<userData>/cdp-port`（`%APPDATA%/Rutter Browser/cdp-port`），
  便于本地 rutter `attach` 发现正在运行的浏览器。

Electron 保留了 `--app` 开关，因此 rutter 的无 chrome 窗口参数刻意
不作用于本引擎（见 `crates/engine-cdp/src/launch.rs`）。
