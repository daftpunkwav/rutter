# i18n/ — string catalogs

English | [中文](README.zh.md)

`en.json` is the default (and currently only) locale, fetched by
`app.js` at startup; a failed fetch is non-fatal — the page connects
regardless, and only keys missing from a loaded catalog fall back to
their key names. Keys are flat identifiers looked up through the
`data-i18n` attributes in `../src/index.html` and the `t()` helper in
`../src/app.js`.
