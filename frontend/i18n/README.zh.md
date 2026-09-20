# i18n/ — 字符串目录

[English](README.md) | 中文

`en.json` 是默认（也是当前唯一）的 locale，由 `app.js` 在启动时拉
取；拉取失败不致命（标签回退到各自的 `data-i18n` 键）。键是扁平标
识符，与 `../src/index.html` 中的 `data-i18n` 属性相对应。
