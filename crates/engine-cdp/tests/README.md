# tests/ — real-engine integration

English | [中文](README.zh.md)

Runs against a downloaded chrome-headless-shell and is `#[ignore]`d by
default (CI's integration job runs them explicitly):

| File | Covers |
|---|---|
| `integration.rs` | Launch, navigate/snapshot, page cap, idempotent context close, vanished-target close |
| `screencast.rs` | `startScreencast` frame flow and ack loop against real CDP |

First run downloads ~150 MB into the rutter cache (reused by `rutter
open`); offline runs need the cache already filled.
