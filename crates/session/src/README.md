# src/ — file map

English | [中文](README.zh.md)

| Item | Role |
|---|---|
| `lib.rs` | Public exports; the crate is the engine/observe/policy/events junction |
| `manager.rs` | `SessionManager`: lazy engine start, session cap, recovery task |
| `session/` | `Session`: pages, execute, screencast, close (see its README) |
| `pages.rs` | `PageRegistry`: the tracked slots, the one-active invariant, the recovery gate |
| `actions/` | `Executor`/`PageOps`: one action per call, auto-wait (see its README) |
| `resolve.rs` | Reference resolution through the observe scripts |
| `storage.rs` | Storage state (cookies + localStorage), atomic owner-only persistence |
| `audit.rs` | Append-only approval trail: one JSON line per supervised decision |
| `wait.rs` | `poll_until` with the 600 s budget clamp (clock-overflow guard) |
| `config.rs` | `SessionConfig`, its auto-wait defaults pinned by docs/tool-catalog.md §3 (the `wait_for` budget default lives on the MCP tool layer, §4) |
| `error.rs` | `SessionError` bridging engine and action errors |
| `mock.rs` | Test-only engine doubles (`#[cfg(test)]`) for browser-free tests |

New session behavior goes through `Session::execute`'s gate → action →
events → refresh sequence; bypassing it loses policy, events, or
persistence.
