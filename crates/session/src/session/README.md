# session/ — the Session type

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `Session` + `PageSlot`/`PageInfo`: active page, execute, tabs operations, storage persistence, screencast, close |
| `tests.rs` | Browser-free behavior tests against the crate's mock engine |

One active page is the invariant every caller relies on; `execute`
refreshes URL + storage after every attempt (persist on change,
blueprint §7.4), and `close` tears down the whole browser context
idempotently. `tests.rs` exists so the logic file stays readable —
keep new tests there.
