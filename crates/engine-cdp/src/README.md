# src/ — file map

All implementation modules are private on purpose; the crate boundary
is `launch.rs`'s `CdpLauncher` alone.

| File | Role |
|---|---|
| `launch.rs` | `CdpLauncher`: resolve binary, launch chromiumoxide, wrap as `Engine` |
| `engine.rs` | `CdpEngine`: browser contexts over chromiumoxide's browser |
| `context.rs` | `CdpContext`: targets, cookies, idempotent context close |
| `page.rs` | `CdpPage`: evaluate, input, screenshots, deadline-wrapped screencast |
| `error.rs` | `with_deadline` wrappers + folding CDP errors into `EngineError` |

Never let a chromiumoxide type reach a `pub` signature — the traits in
`rutter-engine` are the only way out (blueprint §5). Every CDP call
goes through a deadline; none may hang bare.
