# session/ — the Session type

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `Session`: active page, execute, tabs operations, storage persistence, screencast, close |
| `tests.rs` | Browser-free behavior tests against the crate's mock engine |
| `flow_tests.rs` | Browser-free tests for storage capture/save/load, the direct `recover` rebuild, the event error taxonomy, and the persistence-failure path |

One active page is the invariant every caller relies on; `execute`
refreshes URL + storage after every attempt (persist on change,
docs/sessions.md), and `close` tears down the whole browser context
idempotently. `tests.rs` and `flow_tests.rs` exist so the logic file
stays readable — keep new tests there.
