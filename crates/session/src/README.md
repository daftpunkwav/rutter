# src/ — file map

| Item | Role |
|---|---|
| `lib.rs` | Public exports; the crate is the engine/observe/policy/events junction |
| `manager.rs` | `SessionManager`: lazy engine start, session cap, recovery task |
| `session/` | `Session`: pages, execute, screencast, close (see its README) |
| `actions/` | `Executor`/`PageOps`: one action per call, auto-wait (see its README) |
| `resolve.rs` | Reference resolution through the observe scripts |
| `storage.rs` | Storage state (cookies + localStorage), atomic owner-only persistence |
| `wait.rs` | `poll_until` with the 600 s budget clamp (clock-overflow guard) |
| `config.rs` | `SessionConfig` defaults pinned by TOOL_SPEC §3 |
| `error.rs` | `SessionError` bridging engine and action errors |
| `mock.rs` | Test-only engine doubles (`#[cfg(test)]`) for browser-free tests |

New session behavior goes through `Session::execute`'s gate → action →
events → refresh sequence; bypassing it loses policy, events, or
persistence.
