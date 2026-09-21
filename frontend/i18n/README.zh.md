# i18n/ — 字符串目录

[English](README.md) | 中文

`en.json` 是默认（也是当前唯一）的 locale，由 `app.js` 在启动时拉
取；拉取失败不致命——页面照常连接，只有已加载目录中缺失的键才回退
为键名。键是扁平标识符，经 `../src/index.html` 的 `data-i18n` 属性
与 `../src/app.js` 的 `t()` 辅助函数查找。
