# tests/ — cross-crate integration

Acceptance tests spanning crate boundaries: each one spawns the built
`rutter` binary and drives it as an MCP client, so the whole stack
(client → transport → mcp → session → engine-cdp) runs together.
`#[ignore]`d by default; run with `--ignored` (needs the engine binary
in the cache).

| Target | Covers |
|---|---|
| `mcp_e2e.rs` | TOOL_SPEC §6 task classes: read, interact, form, wait_for, close_session fail-fast |
| `approval_e2e.rs` | Policy gating over the real binary: park, grant/deny, timeout |
| `http_e2e.rs` | Streamable HTTP transport end to end |

The binary is located via `CARGO_BIN_EXE_rutter` when set, else the
workspace `target/debug` layout, else the `RUTTER_BIN` override.
