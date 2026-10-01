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
  使单独启动的浏览器可在本机被发现。
- rutter 附着时，其回退逻辑会跳过壳自身的工具栏文档：`toolbar.html`
  文件名属于附着契约的一部分（见 `crates/engine-cdp/src/context.rs`）。

Electron 保留了 `--app` 开关，因此 rutter 的无 chrome 窗口参数刻意
不作用于本引擎（见 `crates/engine-cdp/src/launch.rs`）。

## 引擎与壳的版本

两套 Chromium 面是刻意分开 pin 的：

- 本壳在 `package.json` 中 pin Electron。一个 Electron 发布线内嵌
  固定的 Chromium 主版本（Electron 38 内嵌 Chromium 140），因此该
  pin 让壳的 CDP 面保持不动。
- rutter 拉起的引擎跟随 Chrome for Testing 的 last-known-good
  stable 通道（见 `crates/engine/src/download/manifest.rs`），其
  Chromium 主版本会随时间移动。

协议由引擎主导：rutter 说的是引擎暴露的 CDP 面，壳只需接受附着
流程。升级 Electron pin 时，选择 Chromium 主版本与引擎的 Chrome
for Testing 线相差不超过一步的最新 release，然后让 integration 与
e2e 套件跑真实组合来确认——兼容契约是套件，不是版本号本身。

## 安全门卫

Electron 的默认授权超出了本壳应有的范围，因此壳自己钉死：

- **权限默认拒绝。** 权限请求与 Chromium 的同步权限检查都经过
  同一个谓词（`browser/permission-gate.js`）：只有 `fullscreen`、
  `pointerLock`、`clipboard-sanitized-write` 这三种无法从机器上
  取走数据的权限被放行；geolocation、摄像头、通知以及一切未列出
  的权限一律拒绝。门卫挂在默认 session 上，并经
  `app.on('session-created')` 重新挂到之后创建的每个 session，
  不让任何 session 继承默认的全放行。
- **导航白名单只有一份。** renderer 发起的顶层导航
  （`will-navigate`）由主进程入口共用的同一个 `isWebSchemeUrl`
  谓词把守，页面无法把任何窗口引向 `file:`、`devtools:` 或
  `javascript:` URL。程序化的 `loadFile`/`loadURL` 不触发
  `will-navigate`，壳自己的 toolbar 与 start 目标不受影响。
- **renderer 设置显式钉死。** toolbar 窗口与共享 view 都显式
  设置 `contextIsolation: true`、`nodeIntegration: false`、
  `sandbox: true`、`webviewTag: false`，不依赖可能随 Electron
  版本漂移的默认值。
- **navigate 通道校验发送方。** 只有 URL 为 toolbar 文档
  （`file://.../toolbar.html`，与 Rust 侧 attach 回退使用同一
  匹配器）的 frame 才能发送 `navigate`；其余一律忽略。

两个门卫谓词都在 `scripts/check_js.sh` 中运行
（`tests/js/scheme-gate.test.mjs`、
`tests/js/permission-gate.test.mjs`），拒绝行为与放行行为一起
被固定下来。
