# actions/ — action execution

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `Executor::run` (one action → auto-wait → dispatch → settle → snapshot) and `PageOps` (url/snapshot/wait_for); `phase_rank` for the auto-wait budgets |
| `tests.rs` | Browser-free tests: phase timeouts, budget restarts, input coordinates |

Auto-wait phases run in order (visible → stable → enabled), each with
its own budget that restarts only when the phase advances (docs/tool-catalog.md
§3) — a flickering page cannot stretch the wait indefinitely.
Policy and approval are the caller's (`Session::execute`) concern and
never run here.
