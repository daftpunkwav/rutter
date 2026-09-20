# frontend/ — dashboard sources

English | [中文](README.zh.md)

Vanilla JS with no build step: the dashboard crate embeds everything
here at compile time (`include_str!`). Plain ES2017+, English strings
via the i18n catalog, no framework, no npm.

| Directory | Contents |
|---|---|
| [`src/`](src/README.md) | `index.html` shell + `app.js` client |
| [`i18n/`](i18n/README.md) | String catalogs (English default) |

Protocol contract with `rutter-dashboard`: WebSocket events/screencast
frames + `POST /api/decisions`, per `crates/dashboard/README.md` and
blueprint §7.7.
