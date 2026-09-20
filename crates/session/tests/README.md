# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Tests the crate through its **public API** only: the manager → session
flow, the approval grant path, and the wait_for contract, all against
the scripted engine in `common/` — no real browser.

| File | Covers |
|---|---|
| `session_flow.rs` | Execute flow (navigate → snapshot), approval-granted cookies, wait_for resolve/timeout, close via the manager |
| `manager_flow.rs` | Session cap (`max_sessions`), close semantics, session-handle stability |
| `common/mod.rs` | Scripted launcher → engine → context → page doubles |

Private-behavior unit tests (page-cap races, slot bookkeeping) stay
beside the code in `src/` — Rust's `tests/` directories cannot reach
crate internals.
