# src/ — file map

English | [中文](README.zh.md)

| Item | Role |
|---|---|
| `engine.rs` | `Engine` trait: create contexts, descriptor, health, shutdown |
| `context.rs` | `ContextHandle` trait: pages, cookies, idempotent `close()` |
| `page.rs` | `PageHandle` trait, screenshots, screencast stream types |
| `descriptor.rs` | `EngineDescriptor`, backends, capabilities (`Display` = event names) |
| `health.rs` | `HealthReport` |
| `input.rs` | Protocol-neutral input events dispatched through `PageHandle` |
| `config.rs` | `LaunchMode`, `ContextConfig` (page caps, screenshot interval) |
| `error.rs` | `EngineError` + per-variant hints; session maps these to `ActionError` |
| `backoff.rs` | Capped exponential backoff |
| `download/` | Chrome-for-Testing resolution and install (see its README) |
| `supervisor/` | Heartbeat, restarts, breaker (see its README) |

Traits here are the crate's whole contract with `session`; implement
them in a backend crate, never call one from here.
