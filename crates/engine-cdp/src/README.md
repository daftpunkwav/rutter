# src/ — file map

English | [中文](README.zh.md)

All implementation modules are private on purpose; the crate boundary
is `launch.rs`'s `CdpLauncher` alone.

| File | Role |
|---|---|
| `launch.rs` | `CdpLauncher`: takes the resolved binary, spawns the browser itself, attaches chromiumoxide, wraps it as `Engine` |
| `engine.rs` | `CdpEngine`: browser contexts over chromiumoxide's browser |
| `context.rs` | `CdpContext`: targets, foreign-target discovery and adoption, cookies, idempotent context close |
| `page.rs` | `CdpPage`: evaluate, input, screenshots, deadline-wrapped screencast |
| `error.rs` | `with_deadline` wrappers + folding CDP errors into `EngineError` |

Never let a chromiumoxide type reach a `pub` signature — the traits in
`rutter-engine` are the only way out (docs/architecture.md). Every CDP call
goes through a deadline; none may hang bare.
