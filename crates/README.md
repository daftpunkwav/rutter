# crates/ — workspace members

English | [中文](README.zh.md)

Ten crates, one dependency direction: everything flows from `core`
outward, and only `cli` (the composition root) sees all of them. The
seams exist so that each crate has one reason to change
(blueprint §5, §8.1); `cli` wires them together at startup.

| Crate | Role | Depends on (workspace) |
|---|---|---|
| [`core/`](core/README.md) | Domain vocabulary: actions, snapshots, references, cookies, errors | — |
| [`events/`](events/README.md) | Typed event backbone: bus, per-session rings, replay | core |
| [`policy/`](policy/README.md) | Verdict rules (TOML) and the approval broker | core |
| [`observe/`](observe/README.md) | In-page scripts plus the snapshot builder (pure, no I/O) | core |
| [`engine/`](engine/README.md) | Engine/page traits, Chrome-for-Testing download, supervisor | core |
| [`engine-cdp/`](engine-cdp/README.md) | The only CDP speaker (chromiumoxide); public face is one launcher | core, engine |
| [`session/`](session/README.md) | Orchestration: sessions, actions, auto-wait, storage, recovery | core, engine, events, observe, policy |
| [`mcp/`](mcp/README.md) | MCP tool surface on rmcp (stdio + streamable HTTP) | core, engine, session |
| [`dashboard/`](dashboard/README.md) | Local supervision dashboard (events, approvals, screencast) | core, engine, events, policy, session |
| [`cli/`](cli/README.md) | Binary entry modes: browse, serve, open; wires everything | all of the above |

Adding a crate: register it in the root `Cargo.toml` members list,
declare workspace deps there (no loose version numbers), and keep the
directions above — a lower crate must never import a higher one.

Tests: every crate has a `tests/` directory exercising its public API;
unit tests for private paths stay beside the code in `src/`. Tests that
span crates live in the workspace-root `tests/` package.
