# i18n/ — string catalogs

English | [中文](README.zh.md)

`en.json` is the default (and currently only) locale, fetched by
`app.js` at startup; a failed fetch is non-fatal (labels fall back to
their `data-i18n` keys). Keys are flat identifiers matching the
`data-i18n` attributes in `../src/index.html`.
