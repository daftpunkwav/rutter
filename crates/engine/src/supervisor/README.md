# supervisor/ — keep one engine alive

Owns the current engine slot: launch with retry/backoff, heartbeat
health probes, capped restarts behind a sliding-window circuit breaker,
and clean shutdown.

| File | Role |
|---|---|
| `mod.rs` | `Supervisor`: `start`/`engine`/`shutdown`, heartbeat + restart loops |
| `policy.rs` | `RestartPolicy`: window, limits, backoff schedule, breaker decisions |
| `tests.rs` | Scripted launcher/engine fixtures for the lifecycle tests |

Consumers must ask `engine()` fresh every time — the slot changes
under restarts, and caching the `Arc` pins a dead instance (the bug
that rule exists for).
