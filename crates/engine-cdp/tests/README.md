# tests/ — real-engine integration

English | [中文](README.zh.md)

Runs against a downloaded chrome-headless-shell and is `#[ignore]`d by
default (CI's integration job runs every suite here explicitly). One
concern per file:

| File | Covers |
|---|---|
| `common/` | Shared engine-binary resolver (`RUTTER_TEST_ENGINE` override or cache) |
| `engine_lifecycle.rs` | Launch, health, navigate/evaluate/input/screenshot, close, shutdown |
| `context_pages.rs` | Page cap, idempotent context close, vanished-target close, lookup and cookie edges |
| `page_history.rs` | Back/forward/reload with effective URLs and the out-of-range error |
| `foreign_pages.rs` | Windows rutter did not open: discovery, adoption, close semantics, cross-context invisibility |
| `screencast.rs` | `startScreencast` frame flow and ack loop against real CDP |
| `read.rs` | Markdown readouts: headings, links, lists, tables, code, with site chrome omitted |

First run downloads ~150 MB into the rutter cache (reused by `rutter
open`); offline runs need the cache already filled.
